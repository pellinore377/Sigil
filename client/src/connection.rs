//! Persist credentials and ambiguous network operations before transmission.
use super::*;
use serde::{Deserialize, Serialize};
use sigil_protocol::accounts;

pub(crate) const MIGRATION: &str = "
CREATE TABLE connection(id INTEGER PRIMARY KEY CHECK(id=1), state BLOB NOT NULL);
CREATE TABLE connection_roots(position INTEGER PRIMARY KEY, data BLOB NOT NULL);
ALTER TABLE archive_pages ADD COLUMN complete INTEGER NOT NULL DEFAULT 1 CHECK(complete IN (0,1));
PRAGMA user_version=9;";
const AAD: &[u8] = b"Sigil/client/connection/v0";
/// Progress in one outbox batch. Expiry means retries stopped, not non-delivery.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SendProgress {
    pub accepted: usize,
    pub expired: usize,
}
#[cfg(test)]
#[path = "connection_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "oidc_tests.rs"]
mod oidc_tests;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    #[serde(default)]
    password: bool,
    #[serde(default)]
    oidc: Option<Oidc>,
    server: String,
    port: u16,
    credential: Zeroizing<String>,
    roots: usize,
    invitation: Option<Zeroizing<String>>,
    label: String,
    reauthorize: bool,
    session: Option<accounts::Session>,
    rotation: Option<Zeroizing<String>>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Oidc {
    #[serde(default)]
    username_needed: bool,
    request_id: String,
    username: Option<String>,
    #[serde(default)]
    completion: Option<Zeroizing<String>>,
    #[serde(default)]
    link_secret: Option<Zeroizing<String>>,
}
fn fresh_credential() -> Result<Zeroizing<String>, Error> {
    let mut bytes = Zeroizing::new([0; 32]);
    getrandom::fill(bytes.as_mut()).map_err(|_| sigil_crypto::Error::Entropy)?;
    Ok(Zeroizing::new(super::transport::hex(bytes.as_ref())))
}
fn validate(profile: &Profile) -> Result<(), Error> {
    if profile.oidc.as_ref().is_some_and(|o| {
        !accounts::valid_credential(&o.request_id)
            || o.username
                .as_ref()
                .is_some_and(|u| !accounts::valid_username(u))
            || profile.session.is_some() != o.link_secret.is_some()
            || o.completion
                .as_ref()
                .is_some_and(|s| !accounts::valid_credential(s))
            || o.link_secret
                .as_ref()
                .is_some_and(|s| !accounts::valid_credential(s) || o.username.is_some())
    }) {
        return Err(Error::InvalidStore);
    }
    super::recovery::account_scope(&profile.server, [0; 32])?;
    if profile.port == 0
        || profile.roots > 16
        || !accounts::valid_credential(&profile.credential)
        || profile.label.is_empty()
        || profile.label.len() > 80
        || profile.label.chars().any(char::is_control)
        || profile
            .invitation
            .as_ref()
            .is_some_and(|v| !accounts::valid_credential(v))
        || profile
            .rotation
            .as_ref()
            .is_some_and(|v| !accounts::valid_credential(v) || v == &profile.credential)
        || profile.session.is_some() == profile.invitation.is_some()
        || (profile.rotation.is_some() && profile.session.is_none())
    {
        return Err(Error::InvalidStore);
    }
    if let Some(session) = &profile.session {
        validate_session(profile, session)?;
    }
    Ok(())
}
fn validate_session(profile: &Profile, session: &accounts::Session) -> Result<(), Error> {
    let address = session
        .address
        .strip_prefix('@')
        .and_then(|s| s.split_once(':'));
    if !accounts::valid_credential(&session.account_id)
        || !accounts::valid_credential(&session.device_id)
        || !address.is_some_and(|(name, server)| {
            accounts::valid_username(name) && server == profile.server
        })
        || session.device_label != profile.label
        || session.expires_at == 0
        || session.expires_at > i64::MAX as u64
    {
        return Err(Error::Conflict);
    }
    if profile.session.as_ref().is_some_and(|old| {
        old.account_id != session.account_id
            || old.device_id != session.device_id
            || old.address != session.address
    }) {
        return Err(Error::Conflict);
    }
    Ok(())
}
fn load(db: &Connection, key: &StorageKey) -> Result<(Profile, Vec<u8>), Error> {
    let sealed: Vec<u8> = db
        .query_row(
            "SELECT state FROM connection WHERE id=1 AND length(state)<=4096",
            [],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    let bytes = key.open(&sealed, AAD)?;
    let profile: Profile = serde_json::from_slice(&bytes).map_err(|_| Error::InvalidStore)?;
    validate(&profile)?;
    Ok((profile, sealed))
}
fn save(
    db: &Connection,
    key: &StorageKey,
    profile: &Profile,
    expected: Option<&[u8]>,
) -> Result<(), Error> {
    validate(profile)?;
    let bytes = Zeroizing::new(serde_json::to_vec(profile).map_err(|_| Error::InvalidStore)?);
    if bytes.len() > 4060 {
        return Err(Error::Limit);
    }
    let sealed = key.seal(&bytes, AAD)?;
    if let Some(expected) = expected {
        if db.execute(
            "UPDATE connection SET state=?1 WHERE id=1 AND state=?2",
            (sealed, expected),
        )? != 1
        {
            return Err(Error::Conflict);
        }
    } else {
        db.execute("INSERT INTO connection VALUES(1,?1)", [sealed])?;
    }
    Ok(())
}
fn root_binding(profile: &Profile, index: usize) -> Result<Vec<u8>, Error> {
    let scope = super::recovery::account_scope(&profile.server, [0; 32])?;
    Ok(binding(11, &scope, &(index as u32).to_be_bytes()))
}
fn client(
    db: &Connection,
    key: &StorageKey,
    profile: &Profile,
    credential: &str,
) -> Result<network::HttpsClient, Error> {
    let roots = roots(db, key, profile)?;
    Ok(network::HttpsClient::discover(
        &profile.server,
        profile.port,
        credential,
        &roots,
    )?)
}
fn roots(db: &Connection, key: &StorageKey, profile: &Profile) -> Result<Vec<Vec<u8>>, Error> {
    let mut roots = Vec::new();
    for index in 0..profile.roots {
        let sealed: Vec<u8> = db.query_row(
            "SELECT data FROM connection_roots WHERE position=?1 AND length(data)<=16420",
            [index as i64],
            |r| r.get(0),
        )?;
        roots.push(key.open(&sealed, &root_binding(profile, index)?)?.to_vec());
    }
    Ok(roots)
}

pub(crate) fn persisted_account_scope(
    db: &Connection,
    key: &StorageKey,
) -> Result<super::Id, Error> {
    let (profile, _) = load(db, key)?;
    let session = profile.session.as_ref().ok_or(Error::Unprepared)?;
    super::recovery::account_scope(&profile.server, decode_id(&session.account_id)?)
}
impl ClientStore {
    #[cfg(feature = "rtc-client")]
    pub(crate) fn connection_roots(&self) -> Result<Vec<Vec<u8>>, Error> {
        let (profile, _) = load(&self.db, &self.key)?;
        roots(&self.db, &self.key, &profile)
    }
    pub(crate) fn connected_account_scope(&self) -> Result<super::Id, Error> {
        let (profile, _) = load(&self.db, &self.key)?;
        let session = profile.session.as_ref().ok_or(Error::Unprepared)?;
        if profile.rotation.is_some() {
            return Err(Error::Unprepared);
        }
        super::recovery::account_scope(&profile.server, decode_id(&session.account_id)?)
    }
    fn recovery_client(&self) -> Result<network::HttpsClient, Error> {
        let scope = self.connected_account_scope()?;
        if self.recovery_scope()? != scope {
            return Err(Error::Conflict);
        }
        self.connected_client()
    }
    pub fn account_storage_online(&self) -> Result<sigil_protocol::recovery::StorageStatus, Error> {
        Ok(self.connected_client()?.account_storage()?)
    }
    /// Explicit repair authorization using the connected account's authenticated
    /// head. Only the trusted checkpoint, exact pending successor or proven
    /// ancestor is eligible.
    /// No rollback or unresolved upload is discarded. Prepare/resume afterward.
    pub fn reconcile_restored_recovery(&mut self) -> Result<(), Error> {
        let response = self.recovery_client()?.recovery_head()?;
        self.reconcile_restored_recovery_head(&response)
    }
    /// Bounded online upload; a failed request leaves prior object acknowledgements
    /// durable. The caller respects Retry-After and resumes this same operation.
    pub fn upload_recovery_step(&mut self) -> Result<Option<sigil_crypto::recovery::Head>, Error> {
        let network = self.recovery_client()?;
        let status = self.recovery_status()?;
        let head = match status.pending {
            Some((super::recovery::Operation::Upload, head)) => head,
            _ => return Err(Error::Unprepared),
        };
        for object in self.pending_recovery_objects()? {
            network.upload_recovery_object(&object)?;
            self.acknowledge_recovery_object(head, object.id())?;
        }
        if !self.pending_recovery_objects()?.is_empty() {
            return Ok(None);
        }
        let published = network.publish_recovery_head(&self.recovery_publication()?)?;
        self.acknowledge_recovery_head(&published)?;
        Ok(Some(head))
    }
    /// Downloads at most one ancestry manifest, one page or 16 records per call
    /// after fetching the initial head/manifest. Each validated object
    /// commits separately so an interruption never restarts a large page.
    pub fn download_recovery_step(
        &mut self,
        accept_unanchored: bool,
    ) -> Result<Option<sigil_crypto::recovery::Head>, Error> {
        use super::recovery::{Download, Operation};
        let network = self.recovery_client()?;
        if self.recovery_competition()?.is_some() {
            if let Some(head) = self.next_recovery_competition_manifest()? {
                let object = network.download_recovery_object(head.manifest)?;
                self.stage_recovery_competition_manifest(&object)?;
                return Ok(None);
            }
            self.finish_recovery_competition()?;
        }
        if !matches!(
            self.recovery_status()?.pending,
            Some((Operation::Import, _))
        ) {
            let response = network.recovery_head()?;
            let id = decode_id(response.manifest.as_deref().ok_or(Error::NotFound)?)?;
            let object = network.download_recovery_object(id)?;
            match self.begin_recovery_import(&response, &object, accept_unanchored) {
                Ok(()) => {}
                Err(Error::Conflict) => {
                    self.begin_recovery_competition(&response, &object)?;
                    return Ok(None);
                }
                Err(error) => return Err(error),
            }
        }
        match self.next_recovery_download()? {
            Download::Manifest { head } => {
                let object = network.download_recovery_object(head.manifest)?;
                self.stage_recovery_manifest(&object)?;
            }
            Download::Page { index, object } => {
                let page = network.download_recovery_object(object)?;
                self.stage_recovery_page(index, &page)?;
            }
            Download::Records { index, records } => {
                for reference in records {
                    let object = network.download_recovery_object(reference.object)?;
                    self.stage_recovery_record(index, reference.id, &object)?;
                }
            }
            Download::Complete => return Ok(Some(self.finish_recovery_import()?)),
        }
        Ok(None)
    }
    /// Persist a fresh credential before enrollment or lost-phone reauthorization.
    /// Use a fresh live-state database; restored history may be configured separately.
    pub fn prepare_enrollment(
        &mut self,
        server: &str,
        port: u16,
        roots: &[Vec<u8>],
        invitation: &str,
        label: &str,
        reauthorize: bool,
    ) -> Result<(), Error> {
        self.prepare_connection(server, port, roots, invitation, label, reauthorize, None)
    }
    pub fn prepare_oidc_enrollment(
        &mut self,
        server: &str,
        port: u16,
        roots: &[Vec<u8>],
        username: Option<&str>,
        label: &str,
        replace_devices: bool,
    ) -> Result<(), Error> {
        let secret = fresh_credential()?;
        self.prepare_connection(
            server,
            port,
            roots,
            &secret,
            label,
            replace_devices,
            Some(Oidc {
                username_needed: false,
                request_id: fresh_credential()?.to_string(),
                username: username.map(str::to_owned),
                completion: None,
                link_secret: None,
            }),
        )
    }
    pub fn start_oidc_online(&self) -> Result<sigil_protocol::oidc::Started, Error> {
        let (profile, _) = load(&self.db, &self.key)?;
        let oidc = profile.oidc.as_ref().ok_or(Error::Unprepared)?;
        let network = client(&self.db, &self.key, &profile, &profile.credential)?;
        let request = sigil_protocol::oidc::Start {
            request_id: oidc.request_id.clone(),
            secret: oidc
                .link_secret
                .as_ref()
                .or(profile.invitation.as_ref())
                .ok_or(Error::Unprepared)?
                .to_string(),
            username: oidc.username.clone(),
            replace_devices: oidc.link_secret.is_none() && profile.reauthorize,
        };
        Ok(if oidc.link_secret.is_some() {
            network.link_oidc(&request)?
        } else {
            network.start_oidc(&request)?
        })
    }
    pub fn restart_oidc_enrollment(&mut self, username: Option<&str>) -> Result<(), Error> {
        self.restart_oidc_enrollment_mode(username, false)
    }
    pub(crate) fn restart_oidc_recovery(&mut self) -> Result<(), Error> {
        let (profile, _) = load(&self.db, &self.key)?;
        let username = profile.oidc.as_ref().and_then(|flow| flow.username.clone());
        self.restart_oidc_enrollment_mode(username.as_deref(), true)
    }
    fn restart_oidc_enrollment_mode(
        &mut self,
        username: Option<&str>,
        replace: bool,
    ) -> Result<(), Error> {
        let (mut profile, expected) = load(&self.db, &self.key)?;
        let oidc = profile.oidc.as_mut().ok_or(Error::Unprepared)?;
        if oidc.link_secret.is_some() {
            return Err(Error::Conflict);
        }
        oidc.request_id = fresh_credential()?.to_string();
        oidc.username = username.map(str::to_owned);
        oidc.completion = None;
        profile.invitation = Some(fresh_credential()?);
        profile.credential = fresh_credential()?;
        profile.reauthorize |= replace;
        save(&self.db, &self.key, &profile, Some(&expected))
    }
    /// Callers serialize enrollment requests. Never discard a live/uncertain
    /// credential, a committed session, or a device with local messaging keys.
    pub(crate) fn cancel_unused_enrollment(&mut self) -> Result<(), Error> {
        if self.enrollment_kind()? == "new" {
            return Ok(());
        }
        let (profile, expected) = load(&self.db, &self.key)?;
        if profile.session.is_some() || profile.rotation.is_some() || self.db.query_row("SELECT EXISTS(SELECT 1 FROM identity) OR EXISTS(SELECT 1 FROM archive) OR EXISTS(SELECT 1 FROM sessions) OR EXISTS(SELECT 1 FROM prekeys)", [], |r| r.get::<_,bool>(0))? { return Err(Error::Conflict); }
        match client(&self.db, &self.key, &profile, &profile.credential)?.session() {
            Err(network::Error::Status { code: 401, .. }) => (),
            Ok(_) => return Err(Error::Conflict),
            Err(error) => return Err(error.into()),
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if load(&tx, &self.key)?.1 != expected || tx.query_row("SELECT EXISTS(SELECT 1 FROM identity) OR EXISTS(SELECT 1 FROM archive) OR EXISTS(SELECT 1 FROM sessions) OR EXISTS(SELECT 1 FROM prekeys)", [], |r| r.get::<_,bool>(0))? { return Err(Error::Conflict); }
        tx.execute("DELETE FROM connection_roots", [])?;
        tx.execute("DELETE FROM connection", [])?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn needs_history_recovery(&self) -> Result<bool, Error> {
        let (profile, _) = load(&self.db, &self.key)?;
        Ok(profile.reauthorize && profile.session.is_some() && !recovery::configured(&self.db)?)
    }
    pub fn accept_oidc_callback(
        &mut self,
        request_id: &str,
        completion: &str,
    ) -> Result<(), Error> {
        let (mut profile, expected) = load(&self.db, &self.key)?;
        let oidc = profile.oidc.as_mut().ok_or(Error::Unprepared)?;
        if oidc.request_id != request_id
            || !accounts::valid_credential(completion)
            || oidc
                .completion
                .as_ref()
                .is_some_and(|old| old.as_str() != completion)
        {
            return Err(Error::Conflict);
        }
        oidc.completion = Some(Zeroizing::new(completion.into()));
        save(&self.db, &self.key, &profile, Some(&expected))
    }
    pub fn prepare_oidc_link(&mut self) -> Result<(), Error> {
        self.connected_client()?;
        let (mut profile, expected) = load(&self.db, &self.key)?;
        if profile.oidc.is_some() {
            return Err(Error::Conflict);
        }
        profile.oidc = Some(Oidc {
            username_needed: false,
            request_id: fresh_credential()?.to_string(),
            username: None,
            completion: None,
            link_secret: Some(fresh_credential()?),
        });
        save(&self.db, &self.key, &profile, Some(&expected))
    }
    pub(crate) fn oidc_link_pending(&self) -> Result<bool, Error> {
        let (profile, _) = load(&self.db, &self.key)?;
        Ok(profile.oidc.is_some_and(|flow| flow.link_secret.is_some()))
    }
    pub fn cancel_oidc_link(&mut self) -> Result<(), Error> {
        let (mut profile, expected) = load(&self.db, &self.key)?;
        if profile
            .oidc
            .as_ref()
            .is_none_or(|o| o.link_secret.is_none())
        {
            return Err(Error::Unprepared);
        }
        profile.oidc = None;
        save(&self.db, &self.key, &profile, Some(&expected))
    }
    pub fn finish_oidc_link_online(&mut self) -> Result<bool, Error> {
        let (mut profile, expected) = load(&self.db, &self.key)?;
        let oidc = profile.oidc.as_ref().ok_or(Error::Unprepared)?;
        let secret = oidc.link_secret.as_ref().ok_or(Error::Unprepared)?;
        let progress = self
            .connected_client()?
            .finish_oidc(&sigil_protocol::oidc::Finish {
                request_id: oidc.request_id.clone(),
                secret: secret.to_string(),
                completion: oidc.completion.as_ref().map(|s| s.to_string()),
            })?;
        match progress {
            sigil_protocol::oidc::Progress::Pending => return Ok(false),
            sigil_protocol::oidc::Progress::Linked => {}
            _ => return Err(Error::Cancelled),
        }
        profile.oidc = None;
        save(&self.db, &self.key, &profile, Some(&expected))?;
        Ok(true)
    }
    pub fn finish_oidc_online(&mut self) -> Result<Option<accounts::Session>, Error> {
        let (mut profile, expected) = load(&self.db, &self.key)?;
        if let Some(oidc) = &profile.oidc {
            if oidc.link_secret.is_some() {
                return Err(Error::Conflict);
            }
            let progress = client(&self.db, &self.key, &profile, &profile.credential)?
                .finish_oidc(&sigil_protocol::oidc::Finish {
                    request_id: oidc.request_id.clone(),
                    completion: oidc.completion.as_ref().map(|s| s.to_string()),
                    secret: profile
                        .invitation
                        .as_ref()
                        .ok_or(Error::Unprepared)?
                        .to_string(),
                })?;
            match progress {
                sigil_protocol::oidc::Progress::Pending => return Ok(None),
                sigil_protocol::oidc::Progress::UsernameRequired => {
                    profile
                        .oidc
                        .as_mut()
                        .ok_or(Error::Unprepared)?
                        .username_needed = true;
                    save(&self.db, &self.key, &profile, Some(&expected))?;
                    return Ok(None);
                }
                sigil_protocol::oidc::Progress::Access { .. } => {
                    profile.reauthorize = true;
                }
                sigil_protocol::oidc::Progress::Ready { reauthorize, .. }
                    if reauthorize == profile.reauthorize => {}
                _ => return Err(Error::Cancelled),
            }
            profile.oidc = None;
            let tx = self
                .db
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            save(&tx, &self.key, &profile, Some(&expected))?;
            tx.commit()?;
        }
        self.enroll_online().map(Some)
    }
    #[allow(clippy::too_many_arguments)]
    fn prepare_connection(
        &mut self,
        server: &str,
        port: u16,
        roots: &[Vec<u8>],
        invitation: &str,
        label: &str,
        reauthorize: bool,
        oidc: Option<Oidc>,
    ) -> Result<(), Error> {
        let credential = fresh_credential()?;
        network::HttpsClient::new(server, port, &credential, roots)?;
        let profile = Profile {
            password: false,
            oidc,
            server: server.into(),
            port,
            credential,
            roots: roots.len(),
            invitation: Some(Zeroizing::new(invitation.into())),
            label: label.into(),
            reauthorize,
            session: None,
            rotation: None,
        };
        validate(&profile)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.query_row("SELECT EXISTS(SELECT 1 FROM connection) OR EXISTS(SELECT 1 FROM identity) OR EXISTS(SELECT 1 FROM sessions) OR EXISTS(SELECT 1 FROM prekeys)", [], |r| r.get::<_, bool>(0))? { return Err(Error::Conflict); }
        save(&tx, &self.key, &profile, None)?;
        for (index, root) in roots.iter().enumerate() {
            tx.execute(
                "INSERT INTO connection_roots VALUES(?1,?2)",
                (
                    index as i64,
                    self.key.seal(root, &root_binding(&profile, index)?)?,
                ),
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn connection_session(&self) -> Result<Option<accounts::Session>, Error> {
        session_in(&self.db, &self.key)
    }
    pub(crate) fn enrollment_kind(&self) -> Result<&'static str, Error> {
        match load(&self.db, &self.key) {
            Ok((p, _)) => Ok(if p.session.is_some() {
                "connected"
            } else if p.oidc.as_ref().is_some_and(|o| o.username_needed) {
                "username"
            } else if p.oidc.is_some() {
                "oidc"
            } else if p.password {
                "password"
            } else {
                "invitation"
            }),
            Err(Error::NotFound) => Ok("new"),
            Err(e) => Err(e),
        }
    }
    pub(crate) fn enrollment_server(&self) -> Result<Option<String>, Error> {
        match load(&self.db, &self.key) {
            Ok((p, _)) => Ok(Some(p.server)),
            Err(Error::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }
    pub fn choose_registration_username(&mut self, username: &str) -> Result<(), Error> {
        let (mut p, expected) = load(&self.db, &self.key)?;
        let oidc = p.oidc.as_ref().ok_or(Error::Unprepared)?;
        if !oidc.username_needed {
            return Err(Error::Conflict);
        }
        client(&self.db, &self.key, &p, &p.credential)?.oidc_registration_name(
            &sigil_protocol::oidc::RegistrationName {
                finish: sigil_protocol::oidc::Finish {
                    request_id: oidc.request_id.clone(),
                    secret: p.invitation.as_ref().ok_or(Error::Unprepared)?.to_string(),
                    completion: oidc.completion.as_ref().map(|s| s.to_string()),
                },
                username: username.into(),
            },
        )?;
        p.oidc.as_mut().ok_or(Error::Unprepared)?.username_needed = false;
        save(&self.db, &self.key, &p, Some(&expected))?;
        self.finish_oidc_online()?;
        Ok(())
    }
    pub fn sign_in_password_online(
        &mut self,
        server: &str,
        port: u16,
        roots: &[Vec<u8>],
        username: &str,
        password: &str,
    ) -> Result<accounts::Session, Error> {
        if self.enrollment_kind()? == "new" {
            self.prepare_connection(
                server,
                port,
                roots,
                &fresh_credential()?,
                "Android",
                true,
                None,
            )?;
            let (mut p, expected) = load(&self.db, &self.key)?;
            p.password = true;
            save(&self.db, &self.key, &p, Some(&expected))?;
        }
        let (mut p, expected) = load(&self.db, &self.key)?;
        if !p.password || p.server != server || p.port != port {
            return Err(Error::Conflict);
        }
        let network = client(&self.db, &self.key, &p, &p.credential)?;
        let session = match network.session() {
            Ok(s) => s,
            Err(network::Error::Status { code: 401, .. }) => {
                network.password_sign_in(username, password, &p.label)?
            }
            Err(e) => return Err(e.into()),
        };
        validate_session(&p, &session)?;
        if session.address != format!("@{username}:{server}") {
            return Err(Error::Conflict);
        }
        p.session = Some(session.clone());
        p.invitation = None;
        save(&self.db, &self.key, &p, Some(&expected))?;
        Ok(session)
    }
    /// Exposes transport only for the durably bound account. Returns Unprepared
    /// while enrollment or credential rotation still needs reconciliation.
    pub fn connected_client(&self) -> Result<network::HttpsClient, Error> {
        let (profile, state) = load(&self.db, &self.key)?;
        let mut cached = self.connection.borrow_mut();
        if profile.session.is_none() || profile.rotation.is_some() {
            *cached = None;
            return Err(Error::Unprepared);
        }
        if cached.as_ref().is_none_or(|(previous, created, _)| previous != &state || created.elapsed().as_secs() >= 60) {
            *cached = None;
            *cached = Some((state, std::time::Instant::now(), client(&self.db, &self.key, &profile, &profile.credential)?));
        }
        Ok(cached.as_ref().ok_or(Error::Unprepared)?.2.clone())
    }
    /// No database write lock is held across a network call. On an ambiguous
    /// enrollment response, GET session recovers the original committed identity.
    pub fn enroll_online(&mut self) -> Result<accounts::Session, Error> {
        let (mut profile, expected) = load(&self.db, &self.key)?;
        if profile.rotation.is_some() || profile.oidc.is_some() {
            return Err(Error::Conflict);
        }
        let network = client(&self.db, &self.key, &profile, &profile.credential)?;
        let session = match network.session() {
            Ok(session) => session,
            Err(network::Error::Status { code: 401, .. }) if profile.invitation.is_some() => {
                network.enroll(
                    profile.invitation.as_deref().ok_or(Error::Unprepared)?,
                    &profile.label,
                    profile.reauthorize,
                )?
            }
            Err(error) => return Err(error.into()),
        };
        validate_session(&profile, &session)?;
        profile.invitation = None;
        profile.session = Some(session);
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        save(&tx, &self.key, &profile, Some(&expected))?;
        tx.commit()?;
        profile.session.take().ok_or(Error::InvalidStore)
    }
    pub fn prepare_credential_rotation(&mut self) -> Result<(), Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (mut profile, expected) = load(&tx, &self.key)?;
        if profile.session.is_none() {
            return Err(Error::Unprepared);
        }
        if profile.rotation.is_none() {
            profile.rotation = Some(fresh_credential()?);
            save(&tx, &self.key, &profile, Some(&expected))?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn rotate_credential_online(&mut self) -> Result<accounts::Session, Error> {
        let (mut profile, expected) = load(&self.db, &self.key)?;
        let next = profile.rotation.as_deref().ok_or(Error::Unprepared)?;
        let network = client(&self.db, &self.key, &profile, next)?;
        let session = match network.session() {
            Ok(session) => session,
            Err(network::Error::Status { code: 401, .. }) => {
                client(&self.db, &self.key, &profile, &profile.credential)?
                    .rotate_credential(next)?
            }
            Err(error) => return Err(error.into()),
        };
        validate_session(&profile, &session)?;
        profile.credential = profile.rotation.take().ok_or(Error::Unprepared)?;
        profile.session = Some(session);
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        save(&tx, &self.key, &profile, Some(&expected))?;
        tx.commit()?;
        profile.session.take().ok_or(Error::InvalidStore)
    }
    /// Fetch account inventory without changing peer verification or session state.
    pub fn devices_online(&self, after: Option<&str>) -> Result<accounts::DevicePage, Error> {
        let session = self.connection_session()?.ok_or(Error::Unprepared)?;
        let page = self.connected_client()?.devices(after)?;
        if page.account_id != session.account_id {
            return Err(Error::Conflict);
        }
        Ok(page)
    }
    /// Revoke an account device's server authorization; retain local history and
    /// encryption trust. Only a successful server response confirms revocation.
    /// Self-revocation makes subsequent authenticated requests fail, including
    /// retries after a lost response. Authentication failure is not confirmation.
    pub fn revoke_device_online(&self, device: &str) -> Result<(), Error> {
        self.connected_client()?.revoke_device(device)?;
        Ok(())
    }
    /// Process at most 16 queued packets using a caller-supplied trusted clock.
    /// Expiry commits before network I/O; remaining exact packets survive transport
    /// failure. Errors may follow durable expiry or acceptance; retry resumes them.
    pub fn send_pending_online(&mut self, session: Id, now: u64) -> Result<SendProgress, Error> {
        self.send_pending_limit(session, now, 16)
    }
    pub(super) fn send_pending_limit(
        &mut self,
        session: Id,
        now: u64,
        limit: usize,
    ) -> Result<SendProgress, Error> {
        let network = self.connected_client()?;
        let (pending, expired) = self.outgoing_batch(session, now, limit)?;
        let mut progress = SendProgress {
            accepted: 0,
            expired,
        };
        for request in pending {
            super::load(&self.db, &self.key, &session)?;
            let id: Id = decode_id(&request.message_id)?;
            let destination = self.delivery_destination(session, id, &request.recipient_device)?;
            let own = self.connection_session()?.ok_or(Error::Unprepared)?;
            let Some(receipt) =
                crate::federation::submit(&network, &own, &destination, &request, || {
                    self.check_retry_send(id, now)?;
                    self.check_group_distribution_send(session, id, now)?;
                    self.check_group_invitation_send(session, id, now)?;
                    let raw: Option<Vec<u8>> = self.db.query_row(
                        "SELECT content FROM outbox WHERE session=?1 AND id=?2",
                        (session.as_slice(), id.as_slice()),
                        |r| r.get(0),
                    )?;
                    if let Some(raw) = raw {
                        let raw = self.key.open(&raw, &binding(9, &session, &id))?;
                        crate::conversations::check_send(&self.db, &self.key, &raw, now)?;
                    }
                    Ok(())
                })?
            else {
                break;
            };
            self.acknowledge_sent(session, id, &receipt)?;
            progress.accepted += 1;
        }
        Ok(progress)
    }
}
pub(super) fn session_in(
    db: &Connection,
    key: &StorageKey,
) -> Result<Option<accounts::Session>, Error> {
    Ok(load(db, key)?.0.session)
}
pub(super) fn decode_id(value: &str) -> Result<Id, Error> {
    if !accounts::valid_credential(value) {
        return Err(Error::InvalidStore);
    }
    let mut id = [0; 32];
    for (out, pair) in id.iter_mut().zip(value.as_bytes().as_chunks::<2>().0) {
        *out = u8::from_str_radix(
            std::str::from_utf8(pair).map_err(|_| Error::InvalidStore)?,
            16,
        )
        .map_err(|_| Error::InvalidStore)?;
    }
    Ok(id)
}

pub(super) fn install_linked_connection(
    tx: &Transaction<'_>,
    key: &StorageKey,
    session: accounts::Session,
    port: u16,
    roots: &[Vec<u8>],
    credential: Zeroizing<String>,
) -> Result<(), Error> {
    if tx.query_row("SELECT EXISTS(SELECT 1 FROM connection)", [], |r| {
        r.get::<_, bool>(0)
    })? {
        return Err(Error::Conflict);
    }
    let server = session
        .address
        .rsplit_once(':')
        .ok_or(Error::InvalidStore)?
        .1
        .to_owned();
    let profile = Profile {
        password: false,
        oidc: None,
        server,
        port,
        credential,
        roots: roots.len(),
        invitation: None,
        label: session.device_label.clone(),
        reauthorize: false,
        session: Some(session),
        rotation: None,
    };
    validate(&profile)?;
    save(tx, key, &profile, None)?;
    for (index, root) in roots.iter().enumerate() {
        tx.execute(
            "INSERT INTO connection_roots VALUES(?1,?2)",
            (
                index as i64,
                key.seal(root, &root_binding(&profile, index)?)?,
            ),
        )?;
    }
    Ok(())
}
