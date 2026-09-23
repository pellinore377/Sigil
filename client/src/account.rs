//! Account identity: one key endorses every device, recovery restores it without replacing
//! any device, and contacts pin the key instead of each device.
use super::*;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use zeroize::Zeroizing;
use sigil_crypto::{storage::StorageKey, IdentityKey, Secret32};
use sha2::{Digest, Sha256};
use rusqlite::{Connection, Transaction, TransactionBehavior};
use base64ct::{Base64UrlUnpadded, Encoding};
use sigil_crypto::account::{
    endorse, open_bundle, seal_bundle, unwrap_secret, wrap_secret,
};
use sigil_protocol::accounts::{
    AccountKey, Activate, PendingState, PublishAccountKey, RecoveryWrap, ResetIdentity,
};

pub(crate) const MIGRATION: &str = "
CREATE TABLE IF NOT EXISTS account_identity(id INTEGER PRIMARY KEY CHECK(id=1), state BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS account_pins(id BLOB PRIMARY KEY, state BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS retry_backoff(id BLOB PRIMARY KEY, since INTEGER NOT NULL, until INTEGER NOT NULL, attempts INTEGER NOT NULL);
PRAGMA user_version=86;";

/// This account's key and recovery secret, plus the passkeys that wrap the secret.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Own {
    key: Zeroizing<Id>,
    secret: Zeroizing<Id>,
    published: bool,
    passkeys: Vec<Passkey>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Passkey {
    id: String,
    label: String,
    created: u64,
}
/// A contact's pinned account key. Changing it needs the user's approval.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pin {
    pub key: Id,
    pub verified: bool,
}

fn own_aad() -> Vec<u8> {
    binding(60, &[0; 32], b"account-identity")
}
fn load_own(db: &Connection, key: &StorageKey) -> Result<Option<Own>, Error> {
    let sealed: Option<Vec<u8>> = db
        .query_row("SELECT state FROM account_identity WHERE id=1", [], |r| r.get(0))
        .optional()?;
    sealed
        .map(|sealed| {
            let bytes = key.open(&sealed, &own_aad())?;
            serde_json::from_slice(&bytes).map_err(|_| Error::InvalidStore)
        })
        .transpose()
}
fn save_own(tx: &Transaction<'_>, key: &StorageKey, own: &Own) -> Result<(), Error> {
    let bytes = Zeroizing::new(serde_json::to_vec(own).map_err(|_| Error::InvalidStore)?);
    tx.execute(
        "INSERT INTO account_identity VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET state=excluded.state",
        [key.seal(&bytes, &own_aad())?],
    )?;
    Ok(())
}
fn pin_aad(id: &Id) -> Vec<u8> {
    binding(61, id, b"account-pin")
}
pub(crate) fn pin(db: &Connection, key: &StorageKey, server: &str, account: &Id) -> Result<Option<Pin>, Error> {
    let id = event::account_reference(server, account);
    let sealed: Option<Vec<u8>> = db
        .query_row("SELECT state FROM account_pins WHERE id=?1", [id.as_slice()], |r| r.get(0))
        .optional()?;
    sealed
        .map(|sealed| {
            serde_json::from_slice(&key.open(&sealed, &pin_aad(&id))?).map_err(|_| Error::InvalidStore)
        })
        .transpose()
}
pub(crate) fn save_pin(tx: &Transaction<'_>, key: &StorageKey, server: &str, account: &Id, pin: &Pin) -> Result<(), Error> {
    let id = event::account_reference(server, account);
    let bytes = serde_json::to_vec(pin).map_err(|_| Error::InvalidStore)?;
    tx.execute(
        "INSERT INTO account_pins VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET state=excluded.state",
        (id.as_slice(), key.seal(&bytes, &pin_aad(&id))?),
    )?;
    Ok(())
}
/// Review digest shown before accepting a contact's replaced account key.
pub(crate) fn review_digest(server: &str, account: &Id, key: &Id) -> Id {
    Sha256::digest(
        [
            b"Sigil/contact-identity-review/v2\0".as_slice(),
            &(server.len() as u16).to_be_bytes(),
            server.as_bytes(),
            account,
            key,
        ]
        .concat(),
    )
    .into()
}

fn b64(bytes: &[u8]) -> String {
    Base64UrlUnpadded::encode_string(bytes)
}
fn unb64(value: &str) -> Result<Vec<u8>, Error> {
    if value.len() > 1400 {
        return Err(Error::Limit);
    }
    Base64UrlUnpadded::decode_vec(value).map_err(|_| Error::InvalidEvent)
}
fn random32() -> Result<Id, Error> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| sigil_crypto::Error::Entropy)?;
    Ok(bytes)
}
fn parts(session: &sigil_protocol::accounts::Session) -> Result<(String, String, Id), Error> {
    let (username, server) = session
        .address
        .strip_prefix('@')
        .and_then(|s| s.split_once(':'))
        .ok_or(Error::InvalidStore)?;
    Ok((username.into(), server.into(), connection::decode_id(&session.account_id)?))
}
/// Recovery code: the 64-hex secret in groups of four; dashes and spaces are ignored.
pub(crate) fn format_code(secret: &Id) -> String {
    transport::hex(secret)
        .as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).unwrap_or_default().to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join("-")
}
pub(crate) fn parse_code(code: &str) -> Result<Secret32, Error> {
    let hex: String = code
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect::<String>()
        .to_ascii_lowercase();
    let bytes = transport::unhex(&hex).ok_or(Error::InvalidEvent)?;
    Ok(Secret32::from_bytes(bytes.try_into().map_err(|_| Error::InvalidEvent)?))
}

impl ClientStore {
    fn own_key(&self) -> Result<Option<(IdentityKey, Secret32)>, Error> {
        Ok(load_own(&self.db, &self.key)?.map(|own| {
            (
                IdentityKey::from_private_bytes(&own.key),
                Secret32::from_bytes(*own.secret),
            )
        }))
    }
    fn store_own(&mut self, key: &IdentityKey, secret: &Secret32, published: bool, passkeys: Vec<Passkey>) -> Result<(), Error> {
        let session = self.profile_session()?.ok_or(Error::Unprepared)?;
        let (_, server, account) = parts(&session)?;
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        save_own(
            &tx,
            &self.key,
            &Own {
                key: key.private_bytes(),
                secret: Zeroizing::new(*secret.expose()),
                published,
                passkeys,
            },
        )?;
        save_pin(&tx, &self.key, &server, &account, &Pin { key: key.public_key(), verified: true })?;
        tx.commit()?;
        Ok(())
    }
    fn scope(&self) -> Result<Id, Error> {
        let session = self.profile_session()?.ok_or(Error::Unprepared)?;
        let (_, server, account) = parts(&session)?;
        recovery::account_scope(&server, account)
    }
    fn endorsement(&mut self, key: &IdentityKey) -> Result<(Vec<u8>, [u8; 64]), Error> {
        let own = self.own_device_binding()?;
        let fingerprint = peers::fingerprint(&peers::parse(&own)?.binding)?;
        Ok((own, endorse(key, &fingerprint)?))
    }
    /// The first device creates the account key and recovery secret and starts backups.
    /// Later devices received both when they were linked or recovered.
    pub fn ensure_account_key_online(&mut self) -> Result<(), Error> {
        let network = self.connected_client()?;
        if let Some(own) = load_own(&self.db, &self.key)? {
            if own.published {
                return Ok(());
            }
        }
        let (key, secret) = match self.own_key()? {
            Some(value) => value,
            None => match network.account_key() {
                Ok(_) => return Err(Error::Conflict),
                Err(network::Error::Status { code: 404, .. }) => {
                    (IdentityKey::generate()?, Secret32::generate()?)
                }
                Err(error) => return Err(error.into()),
            },
        };
        self.store_own(&key, &secret, false, Vec::new())?;
        self.publish_device_binding_online()?;
        let (_, endorsement) = self.endorsement(&key)?;
        let scope = self.scope()?;
        network.publish_account_key(&PublishAccountKey {
            public: transport::hex(&key.public_key()),
            bundle: Some(transport::hex(&seal_bundle(&secret, &scope, &key)?)),
            endorsement: transport::hex(&endorsement),
        })?;
        self.store_own(&key, &secret, true, self.passkeys()?)
    }
    /// Backups use the account's recovery secret, so recovering the key restores them too.
    pub(crate) fn ensure_backup(&mut self) -> Result<(), Error> {
        if recovery::configured(&self.db)? {
            return Ok(());
        }
        let (_, secret) = self.own_key()?.ok_or(Error::Unprepared)?;
        let session = self.connection_session()?.ok_or(Error::Unprepared)?;
        let (_, server, account) = parts(&session)?;
        self.configure_recovery(&server, account, secret)
    }
    fn passkeys(&self) -> Result<Vec<Passkey>, Error> {
        Ok(load_own(&self.db, &self.key)?.map(|o| o.passkeys).unwrap_or_default())
    }
    pub(crate) fn account_recovery_view(&self) -> Result<Value, Error> {
        let own = load_own(&self.db, &self.key)?;
        Ok(json!({
            "passkeys": own.as_ref().map(|o| o.passkeys.iter().map(|p| json!({"id":p.id,"label":p.label,"created":p.created})).collect::<Vec<_>>()).unwrap_or_default(),
            "ready": own.is_some_and(|o| o.published),
        }))
    }
    fn rp_id(&self) -> Result<String, Error> {
        let (profile_server, network) = match self.pending_session()? {
            Some((session, network)) => (parts(&session)?.1, network),
            None => {
                let session = self.connection_session()?.ok_or(Error::Unprepared)?;
                (parts(&session)?.1, self.connected_client()?)
            }
        };
        let origin = network.api_origin()?;
        let host = origin
            .strip_prefix("https://")
            .map(|h| h.split([':', '/']).next().unwrap_or_default().to_owned())
            .filter(|h| sigil_protocol::valid_server_name(h))
            .unwrap_or(profile_server);
        Ok(host)
    }
    pub(crate) fn passkey_create_options(&mut self) -> Result<Value, Error> {
        let session = self.connection_session()?.ok_or(Error::Unprepared)?;
        let (username, server, account) = parts(&session)?;
        Ok(json!({
            "rp_id": self.rp_id()?,
            "rp_name": "Sigil",
            "user_id": b64(&event::account_reference(&server, &account)),
            "user_name": format!("{username}@{server}"),
            "user_display": session.address,
            "challenge": b64(&random32()?),
            "salt": b64(&random32()?),
            "exclude": self.passkeys()?.iter().filter_map(|p| transport::unhex(&p.id)).map(|id| b64(&id)).collect::<Vec<_>>(),
        }))
    }
    pub(crate) fn passkey_add_online(&mut self, credential: &str, salt: &str, prf: &str, label: &str) -> Result<(), Error> {
        let credential = unb64(credential)?;
        let salt = unb64(salt)?;
        let prf: Id = unb64(prf)?.try_into().map_err(|_| Error::InvalidEvent)?;
        let label: String = label.chars().filter(|c| !c.is_control()).take(80).collect();
        if credential.is_empty() || credential.len() > 1023 || salt.len() != 32 || label.is_empty() {
            return Err(Error::InvalidEvent);
        }
        let (key, secret) = self.own_key()?.ok_or(Error::Unprepared)?;
        let scope = self.scope()?;
        let wrap = RecoveryWrap {
            id: transport::hex(&credential),
            salt: transport::hex(&salt),
            wrapped: transport::hex(&wrap_secret(&prf, &scope, &credential, &secret)?),
            label: label.clone(),
            created: conversations::now(),
        };
        self.connected_client()?.put_recovery_wrap(&wrap)?;
        let mut passkeys = self.passkeys()?;
        passkeys.retain(|p| p.id != wrap.id);
        passkeys.push(Passkey { id: wrap.id, label, created: wrap.created });
        let published = load_own(&self.db, &self.key)?.is_some_and(|o| o.published);
        self.store_own(&key, &secret, published, passkeys)
    }
    pub(crate) fn passkey_remove_online(&mut self, credential: &str) -> Result<(), Error> {
        let id = transport::hex(&unb64(credential)?);
        match self.connected_client()?.delete_recovery_wrap(&id) {
            Ok(()) | Err(network::Error::Status { code: 404, .. }) => {}
            Err(error) => return Err(error.into()),
        }
        let (key, secret) = self.own_key()?.ok_or(Error::Unprepared)?;
        let mut passkeys = self.passkeys()?;
        passkeys.retain(|p| p.id != id);
        let published = load_own(&self.db, &self.key)?.is_some_and(|o| o.published);
        self.store_own(&key, &secret, published, passkeys)
    }
    pub(crate) fn recovery_code(&self) -> Result<String, Error> {
        let (_, secret) = self.own_key()?.ok_or(Error::Unprepared)?;
        Ok(format_code(secret.expose()))
    }
    /// What a pending device shows its passkey provider.
    pub(crate) fn passkey_request(&mut self) -> Result<Value, Error> {
        let (_, network) = self.pending_session()?.ok_or(Error::Unprepared)?;
        let state = network.pending()?;
        let credentials = state
            .wraps
            .iter()
            .map(|w| {
                Ok(json!({
                    "id": b64(&transport::unhex(&w.id).ok_or(Error::InvalidStore)?),
                    "salt": b64(&transport::unhex(&w.salt).ok_or(Error::InvalidStore)?),
                }))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Ok(json!({"rp_id": self.rp_id()?, "challenge": b64(&random32()?), "credentials": credentials}))
    }
    pub(crate) fn pending_passkeys(&mut self) -> Result<usize, Error> {
        let (_, network) = self.pending_session()?.ok_or(Error::Unprepared)?;
        Ok(network.pending()?.wraps.len())
    }
    /// Recovers with a passkey's PRF output: unwraps the recovery secret, then the account key.
    pub(crate) fn recover_with_passkey_online(&mut self, credential: &str, prf: &str) -> Result<(), Error> {
        let credential = unb64(credential)?;
        let prf: Id = unb64(prf)?.try_into().map_err(|_| Error::InvalidEvent)?;
        let (_, network) = self.pending_session()?.ok_or(Error::Unprepared)?;
        let state = network.pending()?;
        let wrap = state
            .wraps
            .iter()
            .find(|w| transport::unhex(&w.id).as_deref() == Some(credential.as_slice()))
            .ok_or(Error::NotFound)?;
        let scope = self.scope()?;
        let wrapped = transport::unhex(&wrap.wrapped).ok_or(Error::InvalidStore)?;
        let secret = unwrap_secret(&prf, &scope, &credential, &wrapped).map_err(|_| Error::RecoveryMismatch)?;
        self.recover_online(secret, state)
    }
    pub(crate) fn recover_with_code_online(&mut self, code: &str) -> Result<(), Error> {
        let secret = parse_code(code)?;
        let (_, network) = self.pending_session()?.ok_or(Error::Unprepared)?;
        let state = network.pending()?;
        self.recover_online(secret, state)
    }
    fn recover_online(&mut self, secret: Secret32, state: PendingState) -> Result<(), Error> {
        self.activate_with_secret_online(&secret, state)?;
        match self.begin_history_recovery_online(Secret32::from_bytes(*secret.expose()), true) {
            Ok(()) => Ok(()),
            // No backup was published yet: start one.
            Err(Error::NotFound | Error::Network(network::Error::Status { code: 404, .. })) => {
                self.ensure_backup()
            }
            Err(error) => Err(error),
        }
    }
    /// Unlocks the account key with the recovery secret and joins the account with it.
    pub(crate) fn activate_with_secret_online(&mut self, secret: &Secret32, state: PendingState) -> Result<(), Error> {
        let AccountKey { public, bundle } = state.account_key.ok_or(Error::NotFound)?;
        let public: Id = transport::unhex(&public)
            .and_then(|v| v.try_into().ok())
            .ok_or(Error::InvalidStore)?;
        let bundle = transport::unhex(&bundle.ok_or(Error::NotFound)?).ok_or(Error::InvalidStore)?;
        let scope = self.scope()?;
        let key = open_bundle(secret, &scope, &public, &bundle).map_err(|_| Error::RecoveryMismatch)?;
        let (_, network) = self.pending_session()?.ok_or(Error::Unprepared)?;
        let (own, endorsement) = self.endorsement(&key)?;
        let session = network.activate_pending(&Activate {
            statement: transport::hex(&own),
            endorsement: transport::hex(&endorsement),
        })?;
        let passkeys = state
            .wraps
            .iter()
            .map(|w| Passkey { id: w.id.clone(), label: w.label.clone(), created: w.created })
            .collect();
        self.store_own(&key, secret, true, passkeys)?;
        self.activate_session(session)
    }
    /// Lost every device and the recovery secret: a new account key. Contacts must accept it.
    pub(crate) fn reset_identity_online(&mut self) -> Result<(), Error> {
        let (_, network) = self.pending_session()?.ok_or(Error::Unprepared)?;
        let key = IdentityKey::generate()?;
        let secret = Secret32::generate()?;
        let scope = self.scope()?;
        let (own, endorsement) = self.endorsement(&key)?;
        let session = network.reset_identity(&ResetIdentity {
            statement: transport::hex(&own),
            key: PublishAccountKey {
                public: transport::hex(&key.public_key()),
                bundle: Some(transport::hex(&seal_bundle(&secret, &scope, &key)?)),
                endorsement: transport::hex(&endorsement),
            },
        })?;
        self.store_own(&key, &secret, true, Vec::new())?;
        self.activate_session(session)?;
        let session = self.connection_session()?.ok_or(Error::Unprepared)?;
        let (_, server, account) = parts(&session)?;
        self.configure_recovery(&server, account, secret)
    }
    /// Sponsor side of linking: the joining device's endorsement, and the account key
    /// and recovery secret to seal for it.
    pub(crate) fn link_secrets(&mut self, joining: &sigil_protocol::device::Binding) -> Result<([u8; 64], Zeroizing<Vec<u8>>), Error> {
        if self.own_key()?.is_none() {
            self.ensure_account_key_online()?;
        }
        let (key, secret) = self.own_key()?.ok_or(Error::Unprepared)?;
        let endorsement = endorse(&key, &peers::fingerprint(joining)?)?;
        let mut plain = Zeroizing::new(Vec::with_capacity(64));
        plain.extend_from_slice(key.private_bytes().as_ref());
        plain.extend_from_slice(secret.expose());
        Ok((endorsement, plain))
    }
    /// Joining side: adopts the account key and recovery secret the sponsor sealed.
    pub(crate) fn adopt_link_secrets(&mut self, plain: &[u8]) -> Result<(), Error> {
        if plain.len() != 64 {
            return Err(Error::InvalidEvent);
        }
        let key = IdentityKey::from_private_bytes(plain[..32].try_into().map_err(|_| Error::InvalidEvent)?);
        let secret = Secret32::from_bytes(plain[32..].try_into().map_err(|_| Error::InvalidEvent)?);
        self.store_own(&key, &secret, true, Vec::new())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    /// A reauthorized test device joins with the original device's recovery secret.
    pub(crate) fn activate(replacement: &mut ClientStore, original: &mut ClientStore) {
        activate_with(replacement, &secret(original));
    }
    pub(crate) fn secret(original: &mut ClientStore) -> Secret32 {
        original.ensure_account_key_online().unwrap();
        original.own_key().unwrap().unwrap().1
    }
    pub(crate) fn activate_with(replacement: &mut ClientStore, secret: &Secret32) {
        let (_, network) = replacement.pending_session().unwrap().unwrap();
        let state = network.pending().unwrap();
        replacement.activate_with_secret_online(secret, state).unwrap();
    }
    #[test]
    fn recovery_codes_round_trip_with_or_without_grouping() {
        let secret = [0xab; 32];
        let code = format_code(&secret);
        assert_eq!(code.len(), 64 + 15);
        assert_eq!(parse_code(&code).unwrap().expose(), &secret);
        assert_eq!(parse_code(&code.to_lowercase().replace('-', " ")).unwrap().expose(), &secret);
        assert!(parse_code("abc").is_err());
    }
}
