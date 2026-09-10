use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use sigil_protocol::{Configuration, Configure, Settings};
use std::{
    fs::{self, File, OpenOptions},
    io,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
    time::{Duration, Instant},
};

const APPLICATION_ID: i64 = 0x5349474c;
pub const SCHEMA_VERSION: i64 = 34;

#[derive(Debug)]
pub enum StoreError {
    Database(rusqlite::Error),
    InvalidData,
    Conflict,
    Busy,
    Unauthorized,
    AlreadyExists,
    NotFound,
    Forbidden,
    DeviceLinkRequired,
    Invalid(&'static str),
}
impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

pub struct Store(pub(crate) Connection);

pub fn private_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

pub fn lock_directory(path: &Path) -> io::Result<File> {
    if !path.exists() {
        fs::create_dir_all(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
        return Err(io::Error::other(
            "data directory must be a private directory (0700)",
        ));
    }
    let lock_path = path.join("server.lock");
    if lock_path.is_symlink() {
        return Err(io::Error::other("invalid lock file"));
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(lock_path)?;
    file.try_lock()
        .map_err(|_| io::Error::other("data directory is already in use"))?;
    Ok(file)
}

pub(crate) fn schema_guard(db: &Connection) -> Result<(), StoreError> {
    // No supported Sigil schema uses either feature. A backup trigger could
    // otherwise silently undo or skip restore-time credential invalidation.
    db.pragma_update(None, "trusted_schema", false)?;
    db.set_db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    let executable: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type IN ('trigger','view'))",
        [],
        |row| row.get(0),
    )?;
    if executable {
        return Err(StoreError::InvalidData);
    }
    Ok(())
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if path.is_symlink() {
            return Err(StoreError::InvalidData);
        }
        if !path.exists() {
            private_file(path).map_err(|_| StoreError::InvalidData)?;
        }
        let metadata = fs::symlink_metadata(path).map_err(|_| StoreError::InvalidData)?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 {
            return Err(StoreError::InvalidData);
        }
        let mut db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        schema_guard(&db)?;
        db.busy_timeout(Duration::from_secs(2))?;
        db.pragma_update(None, "foreign_keys", true)?;
        db.execute_batch("PRAGMA synchronous=EXTRA; PRAGMA secure_delete=ON;")?;
        let transaction = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id: i64 = transaction.query_row("PRAGMA application_id", [], |r| r.get(0))?;
        let version: i64 = transaction.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if id == 0 && version == 0 {
            let tables: i64 = transaction.query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get(0),
            )?;
            if tables != 0 {
                return Err(StoreError::InvalidData);
            }
            transaction.execute_batch("CREATE TABLE configuration (id INTEGER PRIMARY KEY CHECK(id = 1), revision INTEGER NOT NULL CHECK(revision >= 0), settings TEXT); INSERT INTO configuration VALUES(1, 0, NULL);")?;
            transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
        } else if id != APPLICATION_ID || !(1..=SCHEMA_VERSION).contains(&version) {
            return Err(StoreError::InvalidData);
        }
        if version < 2 {
            transaction.execute_batch(crate::accounts::MIGRATION)?;
        }
        if version < 3 {
            transaction.execute_batch(crate::prekeys::MIGRATION)?;
        }
        if version < 4 {
            transaction.execute_batch(crate::mailbox::MIGRATION)?;
        }
        if version < 5 {
            transaction.execute_batch(crate::admission::MIGRATION)?;
        }
        if version < 6 {
            transaction.execute_batch(crate::contacts::MIGRATION)?;
        }
        if version < 7 {
            transaction.execute_batch(crate::maintenance::MIGRATION)?;
        }
        if version < 8 {
            transaction.execute_batch(crate::recovery::MIGRATION)?;
        }
        if version < 9 {
            transaction.execute_batch(crate::device::MIGRATION)?;
        }
        if version < 10 {
            transaction
                .execute_batch("CREATE INDEX devices_account_id ON devices(account_id,id);")?;
        }
        if version < 11 {
            transaction.execute_batch(crate::link::MIGRATION)?;
        }
        if version < 12 {
            transaction.execute_batch(crate::storage_budget::MIGRATION)?;
        }
        if version < 13 {
            transaction.execute_batch(crate::attachments::MIGRATION)?;
        }
        if version < 14 {
            transaction.execute_batch(crate::push_config::MIGRATION)?;
            transaction.execute_batch(crate::push::MIGRATION)?;
        }
        if version < 15 {
            transaction.execute_batch(crate::federation_config::MIGRATION)?;
        }
        if version < 16 {
            transaction.execute_batch(crate::federation_mailbox::MIGRATION)?;
        }
        if version < 17 {
            transaction.execute_batch(crate::federation_outbox::MIGRATION)?;
        }
        if version < 18 {
            transaction.execute_batch(crate::federation_lookup::MIGRATION)?;
        }
        if version < 19 {
            transaction.execute_batch(crate::group_authority::MIGRATION)?;
        }
        if version < 20 {
            transaction.execute_batch(crate::group_authority::RELAY_MIGRATION)?;
        }
        if version < 21 {
            transaction.execute_batch(crate::group_authority::INVITATION_MIGRATION)?;
        }
        if version < 22 {
            transaction.execute_batch(crate::maps::MIGRATION)?;
        }
        if version < 23 {
            transaction.execute_batch(crate::service_config::MIGRATION)?;
        }
        if version < 24 {
            transaction.execute_batch(crate::call_config::MIGRATION)?;
        }
        if version < 25 {
            transaction.execute_batch(crate::admin::MIGRATION)?;
            transaction.execute_batch(crate::oidc::MIGRATION)?;
            transaction.execute_batch(crate::operations::MIGRATION)?;
        }
        if version < 26 {
            transaction.execute_batch("CREATE TABLE IF NOT EXISTS storage_cleanup(id INTEGER PRIMARY KEY CHECK(id=1),pending INTEGER NOT NULL CHECK(pending IN(0,1))); INSERT INTO storage_cleanup VALUES(1,1) ON CONFLICT(id) DO UPDATE SET pending=1;")?;
        }
        if version < 27 {
            crate::storage_budget::rebuild(&transaction)?;
        }
        if version < 28 {
            transaction.execute_batch(crate::web_admin::MIGRATION)?;
            transaction.execute_batch(crate::admin_storage::MIGRATION)?;
        }
        if version < 29 {
            transaction.execute_batch(crate::profile::MIGRATION)?;
            transaction.execute_batch(crate::oidc_transition::MIGRATION)?;
        }
        if version < 30 {
            transaction.execute_batch(crate::password_login::MIGRATION)?;
        }
        if version < 31 {
            transaction.execute_batch(crate::contact_requests::MIGRATION)?;
        }
        if version < 32 {
            transaction.execute_batch(crate::profile_photos::MIGRATION)?;
        }
        if version < 33 {
            transaction
                .execute_batch("ALTER TABLE contact_requests ADD COLUMN invitation TEXT;")?;
        }
        if version < 34 {
            transaction.execute_batch(crate::push_android::MIGRATION)?;
        }
        transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        transaction.commit()?;
        let mode: String = db.query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))?;
        if mode != "delete" {
            return Err(StoreError::InvalidData);
        }
        if db.query_row("SELECT pending FROM storage_cleanup WHERE id=1", [], |r| {
            r.get::<_, bool>(0)
        })? {
            db.execute_batch("VACUUM; UPDATE storage_cleanup SET pending=0 WHERE id=1;")?;
        }
        let store = Self(db);
        store.configuration()?;
        Ok(store)
    }

    pub fn configuration(&self) -> Result<Configuration, StoreError> {
        read_configuration(&self.0)
    }

    pub fn configure(&mut self, update: Configure) -> Result<Configuration, StoreError> {
        update.settings.validate().map_err(StoreError::Invalid)?;
        let transaction = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous = read_configuration(&transaction)?;
        if previous.revision != update.expected_revision {
            return Err(StoreError::Conflict);
        }
        if previous
            .settings
            .as_ref()
            .is_some_and(|s| s.server_name != update.settings.server_name)
        {
            return Err(StoreError::Invalid(
                "server_name cannot change after initial configuration",
            ));
        }
        let revision = previous
            .revision
            .checked_add(1)
            .filter(|r| *r <= i64::MAX as u64)
            .ok_or(StoreError::InvalidData)?;
        let json = serde_json::to_string(&update.settings).map_err(|_| StoreError::InvalidData)?;
        transaction.execute(
            "UPDATE configuration SET revision = ?1, settings = ?2 WHERE id = 1",
            (revision as i64, json),
        )?;
        transaction.commit()?;
        Ok(Configuration {
            revision,
            settings: Some(update.settings),
        })
    }

    pub fn backup(&self, destination: &Path) -> Result<(), StoreError> {
        self.backup_bounded(destination, u64::MAX)
    }
    pub(crate) fn backup_bounded(&self, destination: &Path, max: u64) -> Result<(), StoreError> {
        schema_guard(&self.0)?;
        let page_size: u64 = self.0.query_row("PRAGMA page_size", [], |r| {
            crate::push_config::unsigned(r, 0)
        })?;
        let pages: u64 = self.0.query_row("PRAGMA page_count", [], |r| {
            crate::push_config::unsigned(r, 0)
        })?;
        if pages.saturating_mul(page_size) > max {
            return Err(StoreError::Busy);
        }
        private_file(destination).map_err(|_| StoreError::InvalidData)?;
        let result = (|| {
            let mut target =
                Connection::open_with_flags(destination, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
            let backup = rusqlite::backup::Backup::new(&self.0, &mut target)?;
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                if Instant::now() >= deadline {
                    return Err(StoreError::InvalidData);
                }
                match backup.step(64)? {
                    rusqlite::backup::StepResult::Done => {
                        if fs::metadata(destination)
                            .map_err(|_| StoreError::InvalidData)?
                            .len()
                            > max
                        {
                            return Err(StoreError::Busy);
                        }
                        break;
                    }
                    rusqlite::backup::StepResult::Busy | rusqlite::backup::StepResult::Locked => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    _ => {}
                }
                if (backup.progress().pagecount as u64).saturating_mul(page_size) > max {
                    return Err(StoreError::Busy);
                }
            }
            drop(backup);
            // A portable backup must open from a read-only mount without WAL
            // sidecars. SQLite's backup API copies the source journal mode.
            let mode: String = target.query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))?;
            if mode != "delete" {
                return Err(StoreError::InvalidData);
            }
            target
                .close()
                .map_err(|(_, error)| StoreError::Database(error))?;
            File::open(destination)
                .map_err(|_| StoreError::InvalidData)?
                .sync_all()
                .map_err(|_| StoreError::InvalidData)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(destination);
        }
        result
    }

    pub fn restore(source: &Path, destination: &Path) -> Result<(), StoreError> {
        let db = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let deadline = Instant::now() + Duration::from_secs(30);
        db.progress_handler(10000, Some(move || Instant::now() >= deadline))?;
        schema_guard(&db)?;
        let id: i64 = db.query_row("PRAGMA application_id", [], |r| r.get(0))?;
        let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if id != APPLICATION_ID || !(1..=SCHEMA_VERSION).contains(&version) {
            return Err(StoreError::InvalidData);
        }
        let check: String = db.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        if check != "ok" {
            return Err(StoreError::InvalidData);
        }
        let store = Self(db);
        store.configuration()?;
        if destination.exists() {
            return Err(StoreError::InvalidData);
        }
        let suffix = crate::auth::random_secret().map_err(|_| StoreError::InvalidData)?;
        let staged = destination.with_extension(format!("restore-{suffix}"));
        let result = (|| {
            store.backup(&staged)?;
            let mut restored = Self::open(&staged)?;
            let tx = restored
                .0
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(
                "DELETE FROM invitations; UPDATE devices SET revoked = 1, token_hash = NULL; UPDATE prekeys SET bundle = NULL; UPDATE mailbox SET payload = NULL; UPDATE contact_invitations SET revoked=1; UPDATE recovery_heads SET restored_checkpoint=1; UPDATE attachments SET state=2,reserved_bytes=0 WHERE state=0; UPDATE attachments SET restored_checkpoint=1 WHERE state=1;",
            )?;
            tx.execute_batch("DELETE FROM push_jobs; UPDATE push_channels SET state=3,target=NULL,proof=NULL,proof_hash=NULL,expires_at=NULL;")?;
            crate::push_config::reset_after_restore(&tx)?;
            crate::push_android::reset(&tx)?;
            tx.execute_batch("DELETE FROM oidc_flows; DELETE FROM oidc_grants; UPDATE oidc_configuration SET revision=revision+1,value=NULL;")?;
            crate::oidc_transition::reset(&tx)?;
            tx.execute_batch("DELETE FROM account_passwords; UPDATE password_policy SET enabled=0,revision=revision+1,login_after=0;")?;
            tx.execute_batch(
                "DELETE FROM contact_requests WHERE state!=3; DELETE FROM profile_shares;",
            )?;
            tx.execute_batch("DELETE FROM web_sessions; DELETE FROM web_oidc; UPDATE web_owner SET password=NULL,setup_hash=NULL,issuer=NULL,subject=NULL,picture=NULL,suggested=NULL,suggested_name=NULL,oidc_revision=NULL,password_login=1;")?;
            tx.execute_batch("DELETE FROM operations; DELETE FROM operation_uploads; UPDATE operation_configuration SET value='{\"revision\":0,\"max_backup_bytes\":68719476736,\"release_url\":null,\"release_key\":null,\"exceptions\":[]}';")?;
            crate::group_authority::reset_after_restore(&tx)?;
            crate::federation_config::reset_after_restore(&tx)?;
            crate::federation_mailbox::rebuild(&tx)?;
            crate::federation_outbox::reset_after_restore(&tx)?;
            crate::storage_budget::rebuild(&tx)?;
            tx.commit()?;
            let mode: String = restored
                .0
                .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))?;
            if mode != "delete" {
                return Err(StoreError::InvalidData);
            }
            restored
                .0
                .close()
                .map_err(|(_, error)| StoreError::Database(error))?;
            File::open(&staged)
                .and_then(|file| file.sync_all())
                .map_err(|_| StoreError::InvalidData)?;
            fs::hard_link(&staged, destination).map_err(|_| StoreError::InvalidData)?;
            let parent = destination
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            File::open(parent)
                .and_then(|file| file.sync_all())
                .map_err(|_| StoreError::InvalidData)?;
            Ok(())
        })();
        let _ = fs::remove_file(&staged);
        result
    }
}

pub(crate) fn read_configuration(db: &Connection) -> Result<Configuration, StoreError> {
    let (revision, raw): (i64, Option<String>) = db.query_row(
        "SELECT revision, settings FROM configuration WHERE id = 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let revision = u64::try_from(revision).map_err(|_| StoreError::InvalidData)?;
    let settings: Option<Settings> = raw
        .map(|json| serde_json::from_str(&json))
        .transpose()
        .map_err(|_| StoreError::InvalidData)?;
    if let Some(settings) = &settings {
        settings.validate().map_err(StoreError::Invalid)?;
    }
    if (revision == 0) != settings.is_none() {
        return Err(StoreError::InvalidData);
    }
    Ok(Configuration { revision, settings })
}
