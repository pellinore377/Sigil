//! Durable provider preferences and proof handling; hints never deliver messages.
use super::*;
use crate::connection::decode_id;
use base64ct::{Base64UrlUnpadded as B64, Encoding};
use serde::{Deserialize, Serialize};
use sigil_protocol::push::{
    Confirm, Disable, Payload, Register, State as RemoteState, Status as RemoteStatus, Target,
};
use web_push_native::{
    p256::{self, elliptic_curve::sec1::ToEncodedPoint},
    Auth,
};
use zeroize::Zeroize;
#[path = "push_work.rs"]
mod work;
pub use work::{Progress, ScheduledPush};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Preference {
    #[default]
    Disabled,
    Fcm {
        token: Zeroizing<String>,
    },
    Unified {
        connector: Id,
        endpoint: Option<String>,
        vapid: String,
        secret: Zeroizing<[u8; 32]>,
        auth: Zeroizing<[u8; 16]>,
    },
}
impl Preference {
    fn target(&self) -> Result<Option<Target>, Error> {
        match self {
            Self::Disabled => Ok(None),
            Self::Fcm { token } => Ok(Some(Target::Fcm {
                token: token.to_string(),
            })),
            Self::Unified {
                endpoint,
                vapid,
                secret,
                auth,
                ..
            } => {
                let Some(endpoint) = endpoint else {
                    return Ok(None);
                };
                let public = p256::SecretKey::from_slice(secret.as_slice())
                    .map_err(|_| Error::InvalidStore)?
                    .public_key();
                Ok(Some(Target::UnifiedPush {
                    endpoint: endpoint.clone(),
                    public_key: B64::encode_string(public.to_encoded_point(false).as_bytes()),
                    auth_secret: B64::encode_string(auth.as_slice()),
                    vapid_key: vapid.clone(),
                }))
            }
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    content = "request",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum Operation {
    Register(Register),
    Confirm(Confirm),
    Disable(Disable),
}
fn wipe_target(target: &mut Target) {
    match target {
        Target::Fcm { token } => token.zeroize(),
        Target::UnifiedPush {
            endpoint,
            auth_secret,
            ..
        } => {
            endpoint.zeroize();
            auth_secret.zeroize();
        }
    }
}
impl Drop for Operation {
    fn drop(&mut self) {
        match self {
            Self::Register(r) => wipe_target(&mut r.target),
            Self::Confirm(r) => r.proof.zeroize(),
            Self::Disable(_) => {}
        }
    }
}
#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Local {
    configured: bool,
    generation: u64,
    applied: u64,
    changed: u64,
    checked: u64,
    reconcile: bool,
    preference: Preference,
    status: Option<RemoteStatus>,
    registered: Option<Target>,
    pending: Option<Operation>,
}
impl Drop for Local {
    fn drop(&mut self) {
        if let Some(target) = &mut self.registered {
            wipe_target(target);
        }
    }
}
fn scope(db: &Connection, key: &StorageKey) -> Result<Id, Error> {
    let session = crate::connection::session_in(db, key)?.ok_or(Error::Unprepared)?;
    let (_, server) = session
        .address
        .rsplit_once(':')
        .ok_or(Error::InvalidStore)?;
    let account = crate::recovery::account_scope(server, decode_id(&session.account_id)?)?;
    let mut hash = Sha256::new();
    hash.update(b"Sigil/client/push-scope/v0");
    hash.update(account);
    hash.update(decode_id(&session.device_id)?);
    Ok(hash.finalize().into())
}
fn validate(state: &Local) -> Result<(), Error> {
    if state.generation > i64::MAX as u64
        || state.applied > state.generation
        || state.changed > i64::MAX as u64
        || state.checked > i64::MAX as u64
        || state
            .status
            .as_ref()
            .is_some_and(|s| !network::valid_push_status(s))
    {
        return Err(Error::InvalidStore);
    }
    if let Preference::Unified { vapid, secret, .. } = &state.preference {
        public(vapid)?;
        p256::SecretKey::from_slice(secret.as_slice()).map_err(|_| Error::InvalidStore)?;
    }
    for target in [
        state.preference.target()?.as_ref(),
        state.registered.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        if !network::push_target_fields(target) {
            return Err(Error::InvalidStore);
        }
    }
    if let Some(op) = &state.pending {
        match op {
            Operation::Register(r)
                if r.expected_revision < i64::MAX as u64
                    && network::push_target_fields(&r.target) => {}
            Operation::Disable(r) if r.expected_revision < i64::MAX as u64 => {}
            Operation::Confirm(r)
                if r.revision > 0
                    && r.revision <= i64::MAX as u64
                    && sigil_protocol::accounts::valid_credential(&r.channel)
                    && sigil_protocol::accounts::valid_credential(&r.proof) => {}
            _ => return Err(Error::InvalidStore),
        }
    }
    Ok(())
}
fn read(db: &Connection, key: &StorageKey, scope: &Id) -> Result<(Local, Option<Vec<u8>>), Error> {
    let sealed: Option<Vec<u8>> = db
        .query_row(
            "SELECT CASE WHEN length(state)<=16420 THEN state END FROM push_state WHERE id=1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let Some(bytes) = &sealed else {
        return Ok((Local::default(), None));
    };
    let bytes = key.open(bytes, &binding(55, scope, b"push state"))?;
    let state: Local = serde_json::from_slice(&bytes).map_err(|_| Error::InvalidStore)?;
    validate(&state)?;
    Ok((state, sealed))
}
fn write(
    db: &Connection,
    key: &StorageKey,
    scope: &Id,
    state: &Local,
    expected: Option<&[u8]>,
) -> Result<Vec<u8>, Error> {
    validate(state)?;
    let bytes = Zeroizing::new(serde_json::to_vec(state).map_err(|_| Error::InvalidStore)?);
    if bytes.len() > 16384 {
        return Err(Error::Limit);
    }
    let sealed = key.seal(&bytes, &binding(55, scope, b"push state"))?;
    if let Some(expected) = expected {
        if db.execute(
            "UPDATE push_state SET state=?1 WHERE id=1 AND state=?2",
            (&sealed, expected),
        )? != 1
        {
            return Err(Error::Conflict);
        }
    } else {
        db.execute("INSERT INTO push_state VALUES(1,?1)", [&sealed])?;
    }
    Ok(sealed)
}
fn public(text: &str) -> Result<p256::PublicKey, Error> {
    if text.len() != 87 {
        return Err(Error::InvalidEvent);
    }
    let mut bytes = [0; 65];
    B64::decode(text, &mut bytes).map_err(|_| Error::InvalidEvent)?;
    if bytes[0] != 4 {
        return Err(Error::InvalidEvent);
    }
    p256::PublicKey::from_sec1_bytes(&bytes).map_err(|_| Error::InvalidEvent)
}
fn time(state: &Local, now: u64) -> Result<(), Error> {
    if now == 0 || now < state.changed || now < state.checked || now > i64::MAX as u64 - 86400 {
        return Err(Error::Expired);
    }
    Ok(())
}
fn changed(
    tx: &Transaction<'_>,
    key: &StorageKey,
    scope: &Id,
    state: &mut Local,
    expected: Option<&[u8]>,
    now: u64,
) -> Result<(), Error> {
    time(state, now)?;
    state.configured = true;
    state.changed = now;
    state.generation = state
        .generation
        .checked_add(1)
        .filter(|v| *v <= i64::MAX as u64)
        .ok_or(Error::Limit)?;
    write(tx, key, scope, state, expected)?;
    crate::schedule::nudge_push(tx, key, scope, now)
}

/// The platform passes this opaque token back on distributor callbacks.
pub struct UnifiedRegistration {
    pub connection: String,
    pub vapid_key: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Disabled,
    Fcm,
    UnifiedPush,
}
#[derive(Debug, PartialEq, Eq)]
pub struct PushState {
    pub configured: bool,
    pub choice: Choice,
    pub awaiting_endpoint: bool,
    pub remote: Option<RemoteStatus>,
    pub pending: bool,
    pub updating: bool,
    pub scheduled_at: u64,
    pub next_attempt_at: u64,
    pub failures: u8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceivedHint {
    Ignored,
    Wake,
    ConfirmationQueued,
}
impl ClientStore {
    /// Explicit retry after repairing provider setup; invalid targets never retry automatically.
    pub fn retry_push_registration(&mut self, now: u64) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let scope = scope(&tx, &self.key)?;
        let (mut state, before) = read(&tx, &self.key, &scope)?;
        if !state
            .status
            .as_ref()
            .is_some_and(|s| s.state == RemoteState::Invalid)
            || state.preference.target()?.is_none()
        {
            return Err(Error::Obsolete);
        }
        changed(&tx, &self.key, &scope, &mut state, before.as_deref(), now)?;
        tx.commit()?;
        Ok(())
    }
    pub fn push_state(&self) -> Result<PushState, Error> {
        let scope = scope(&self.db, &self.key)?;
        let (state, _) = read(&self.db, &self.key, &scope)?;
        let (schedule, _) = crate::schedule::read_push(&self.db, &self.key, &scope)?;
        let choice = match state.preference {
            Preference::Disabled => Choice::Disabled,
            Preference::Fcm { .. } => Choice::Fcm,
            Preference::Unified { .. } => Choice::UnifiedPush,
        };
        Ok(PushState {
            configured: state.configured,
            choice,
            awaiting_endpoint: matches!(
                state.preference,
                Preference::Unified { endpoint: None, .. }
            ),
            remote: state.status.clone(),
            pending: state.pending.is_some(),
            updating: state.reconcile || state.applied != state.generation,
            scheduled_at: schedule.last,
            next_attempt_at: schedule.next,
            failures: schedule.failures,
        })
    }
    pub fn set_fcm_push_token(&mut self, token: &str, now: u64) -> Result<(), Error> {
        if !sigil_protocol::push::valid_token(token) {
            return Err(Error::InvalidEvent);
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let scope = scope(&tx, &self.key)?;
        let (mut state, before) = read(&tx, &self.key, &scope)?;
        time(&state, now)?;
        if matches!(&state.preference,Preference::Fcm{token:old} if old.as_str()==token) {
            return Ok(());
        }
        state.preference = Preference::Fcm {
            token: Zeroizing::new(token.into()),
        };
        changed(&tx, &self.key, &scope, &mut state, before.as_deref(), now)?;
        tx.commit()?;
        Ok(())
    }
    pub fn prepare_unified_push(
        &mut self,
        vapid_key: &str,
        replace: bool,
        now: u64,
    ) -> Result<UnifiedRegistration, Error> {
        public(vapid_key)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let scope = scope(&tx, &self.key)?;
        let (mut state, before) = read(&tx, &self.key, &scope)?;
        time(&state, now)?;
        if !replace {
            if let Preference::Unified {
                connector, vapid, ..
            } = &state.preference
            {
                if vapid == vapid_key {
                    return Ok(UnifiedRegistration {
                        connection: crate::transport::hex(connector),
                        vapid_key: vapid.clone(),
                    });
                }
            }
        }
        let mut connector = [0; 32];
        let mut auth = Zeroizing::new([0; 16]);
        let mut secret = Zeroizing::new([0; 32]);
        getrandom::fill(&mut connector).map_err(|_| sigil_crypto::Error::Entropy)?;
        getrandom::fill(auth.as_mut()).map_err(|_| sigil_crypto::Error::Entropy)?;
        let mut valid = false;
        for _ in 0..16 {
            getrandom::fill(secret.as_mut()).map_err(|_| sigil_crypto::Error::Entropy)?;
            if p256::SecretKey::from_slice(secret.as_slice()).is_ok() {
                valid = true;
                break;
            }
        }
        if !valid {
            return Err(sigil_crypto::Error::Entropy.into());
        }
        state.preference = Preference::Unified {
            connector,
            endpoint: None,
            vapid: vapid_key.into(),
            secret,
            auth,
        };
        changed(&tx, &self.key, &scope, &mut state, before.as_deref(), now)?;
        tx.commit()?;
        Ok(UnifiedRegistration {
            connection: crate::transport::hex(&connector),
            vapid_key: vapid_key.into(),
        })
    }
    pub fn unified_push_registration(&self) -> Result<Option<UnifiedRegistration>, Error> {
        let scope = scope(&self.db, &self.key)?;
        let (state, _) = read(&self.db, &self.key, &scope)?;
        Ok(match &state.preference {
            Preference::Unified {
                connector, vapid, ..
            } => Some(UnifiedRegistration {
                connection: crate::transport::hex(connector),
                vapid_key: vapid.clone(),
            }),
            _ => None,
        })
    }
    pub fn set_unified_push_endpoint(
        &mut self,
        connection: &str,
        endpoint: &str,
        now: u64,
    ) -> Result<(), Error> {
        let connector = decode_id(connection)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let scope = scope(&tx, &self.key)?;
        let (mut state, before) = read(&tx, &self.key, &scope)?;
        time(&state, now)?;
        let Preference::Unified {
            connector: current,
            endpoint: old,
            ..
        } = &mut state.preference
        else {
            return Err(Error::Obsolete);
        };
        if *current != connector {
            return Err(Error::Obsolete);
        }
        if old.as_deref() == Some(endpoint) {
            return Ok(());
        }
        *old = Some(endpoint.into());
        if !state
            .preference
            .target()?
            .as_ref()
            .is_some_and(network::push_target_fields)
        {
            return Err(Error::InvalidEvent);
        }
        changed(&tx, &self.key, &scope, &mut state, before.as_deref(), now)?;
        tx.commit()?;
        Ok(())
    }
    pub fn disable_push(&mut self, now: u64) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let scope = scope(&tx, &self.key)?;
        let (mut state, before) = read(&tx, &self.key, &scope)?;
        time(&state, now)?;
        if state.configured && matches!(state.preference, Preference::Disabled) {
            return Ok(());
        }
        state.preference = Preference::Disabled;
        changed(&tx, &self.key, &scope, &mut state, before.as_deref(), now)?;
        tx.commit()?;
        Ok(())
    }
    pub fn receive_fcm_push(&mut self, data: &str, now: u64) -> Result<ReceivedHint, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let scope = scope(&tx, &self.key)?;
        let (mut state, before) = read(&tx, &self.key, &scope)?;
        time(&state, now)?;
        if !matches!(state.preference, Preference::Fcm { .. }) {
            return Ok(ReceivedHint::Ignored);
        }
        if data.len() > 98 {
            return Err(Error::InvalidEvent);
        }
        let bytes = Zeroizing::new(B64::decode_vec(data).map_err(|_| Error::InvalidEvent)?);
        let hint = receive(
            &tx,
            &self.key,
            &scope,
            &mut state,
            before.as_deref(),
            &bytes,
            now,
        )?;
        tx.commit()?;
        Ok(hint)
    }
    pub fn receive_unified_push(
        &mut self,
        connection: &str,
        bytes: &[u8],
        now: u64,
    ) -> Result<ReceivedHint, Error> {
        let connector = decode_id(connection)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let scope = scope(&tx, &self.key)?;
        let (mut state, before) = read(&tx, &self.key, &scope)?;
        time(&state, now)?;
        let Preference::Unified {
            connector: current,
            secret,
            auth,
            ..
        } = &state.preference
        else {
            return Ok(ReceivedHint::Ignored);
        };
        if *current != connector {
            return Ok(ReceivedHint::Ignored);
        }
        if !(103..=4096).contains(&bytes.len()) || bytes[20] != 65 || bytes[21] != 4 {
            return Err(Error::InvalidEvent);
        }
        let rs =
            u32::from_be_bytes(bytes[16..20].try_into().map_err(|_| Error::InvalidEvent)?) as usize;
        if rs <= bytes.len() - 86 {
            return Err(Error::InvalidEvent);
        }
        let secret =
            p256::SecretKey::from_slice(secret.as_slice()).map_err(|_| Error::InvalidStore)?;
        let decrypted = Zeroizing::new(
            web_push_native::decrypt(bytes.to_vec(), &secret, &Auth::from(**auth))
                .map_err(|_| Error::InvalidEvent)?,
        );
        let hint = receive(
            &tx,
            &self.key,
            &scope,
            &mut state,
            before.as_deref(),
            &decrypted,
            now,
        )?;
        tx.commit()?;
        Ok(hint)
    }
}
fn receive(
    tx: &Transaction<'_>,
    key: &StorageKey,
    scope: &Id,
    state: &mut Local,
    before: Option<&[u8]>,
    bytes: &[u8],
    now: u64,
) -> Result<ReceivedHint, Error> {
    match Payload::from_bytes(bytes).map_err(|_| Error::InvalidEvent)? {
        Payload::Wake => Ok(ReceivedHint::Wake),
        Payload::Challenge { channel, proof } => {
            let Some(desired) = state.preference.target()? else {
                return Ok(ReceivedHint::Ignored);
            };
            let Some(remote) = state.status.as_ref() else {
                return Ok(ReceivedHint::Ignored);
            };
            if remote.state != RemoteState::Pending
                || remote.expires_at.is_none_or(|e| e <= now)
                || remote
                    .channel
                    .as_ref()
                    .is_none_or(|id| decode_id(id).ok().as_ref() != Some(channel))
                || Some(&desired) != state.registered.as_ref()
            {
                return Ok(ReceivedHint::Ignored);
            }
            let confirm = Confirm {
                revision: remote.revision,
                channel: crate::transport::hex(channel),
                proof: crate::transport::hex(proof),
            };
            if let Some(op) = &state.pending {
                match op {
                    Operation::Confirm(old)
                        if old.revision == confirm.revision
                            && old.channel == confirm.channel
                            && old.proof == confirm.proof =>
                    {
                        return Ok(ReceivedHint::ConfirmationQueued)
                    }
                    _ => return Ok(ReceivedHint::Ignored),
                }
            }
            state.pending = Some(Operation::Confirm(confirm));
            write(tx, key, scope, state, before)?;
            crate::schedule::nudge_push(tx, key, scope, now)?;
            Ok(ReceivedHint::ConfirmationQueued)
        }
    }
}

#[cfg(test)]
#[path = "push_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "push_delivery_tests.rs"]
mod delivery_tests;
