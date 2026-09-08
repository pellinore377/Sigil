use crate::{
    auth::{digest, random_secret},
    egress,
    enrollment::now,
    push_config::{sql, unsigned},
    store::{private_file, Store, StoreError},
    with_store, AppState,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(crate) const MIGRATION:&str="
CREATE TABLE operation_configuration(id INTEGER PRIMARY KEY CHECK(id=1),value TEXT NOT NULL);
INSERT INTO operation_configuration VALUES(1,'{\"revision\":0,\"max_backup_bytes\":68719476736,\"release_url\":null,\"release_key\":null,\"exceptions\":[]}');
CREATE TABLE operations(id TEXT PRIMARY KEY,actor BLOB NOT NULL,bootstrap INTEGER NOT NULL,action TEXT NOT NULL,created INTEGER NOT NULL,expires INTEGER NOT NULL,status INTEGER NOT NULL DEFAULT 0,lease INTEGER NOT NULL DEFAULT 0,result TEXT,probe TEXT NOT NULL);
CREATE TABLE operation_uploads(id TEXT PRIMARY KEY,bytes INTEGER NOT NULL,sha256 TEXT NOT NULL,received INTEGER NOT NULL DEFAULT 0);
";
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub revision: u64,
    pub max_backup_bytes: u64,
    pub release_url: Option<String>,
    pub release_key: Option<String>,
    pub exceptions: Vec<egress::Exception>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Release {
    pub version: String,
    pub image_digest: String,
    pub minimum_schema: u32,
    pub target_schema: u32,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Backup,
    Inspect { file: String },
    Import { file: String },
    Restore { file: String },
    CheckUpdate,
    CheckEndpoint,
    Upgrade { release: Release },
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub id: String,
    pub action: Action,
    pub effect: String,
    pub expires_at: u64,
    pub state: String,
    pub result: Option<serde_json::Value>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Upload {
    pub bytes: u64,
    pub sha256: String,
}
fn invalid() -> StoreError {
    StoreError::Invalid("invalid maintenance request or private backup")
}
fn io<T>(value: std::io::Result<T>) -> Result<T, StoreError> {
    value.map_err(|_| StoreError::InvalidData)
}
fn encode<T: Serialize>(value: &T) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(|_| StoreError::InvalidData)
}
fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, StoreError> {
    serde_json::from_str(value).map_err(|_| StoreError::InvalidData)
}
fn directory(db: &Connection) -> Result<PathBuf, StoreError> {
    let path = Path::new(db.path().ok_or(StoreError::InvalidData)?);
    let parent = io(fs::canonicalize(
        path.parent().ok_or(StoreError::InvalidData)?,
    ))?;
    let path = parent.join("maintenance");
    if !path.exists() {
        io(fs::create_dir(&path))?;
        io(fs::set_permissions(
            &path,
            fs::Permissions::from_mode(0o700),
        ))?;
        io(fs::File::open(&parent))?
            .sync_all()
            .map_err(|_| StoreError::InvalidData)?;
    }
    let meta = io(fs::symlink_metadata(&path))?;
    if !meta.is_dir() || meta.permissions().mode() & 0o077 != 0 {
        return Err(invalid());
    }
    Ok(path)
}
fn file(dir: &Path, id: &str, extension: &str) -> Result<PathBuf, StoreError> {
    if !sigil_protocol::accounts::valid_credential(id) {
        return Err(invalid());
    }
    let path = dir.join(format!("{id}.{extension}"));
    if path.is_symlink() {
        return Err(invalid());
    }
    Ok(path)
}
fn artifact(name: &str) -> Option<(&str, bool)> {
    let (id, extension) = name.split_once('.')?;
    if !sigil_protocol::accounts::valid_credential(id) {
        return None;
    }
    if extension == "db" {
        return Some((id, false));
    }
    let stem = ["-journal", "-wal", "-shm"]
        .into_iter()
        .find_map(|suffix| extension.strip_suffix(suffix))
        .unwrap_or(extension);
    (matches!(stem, "part" | "backup" | "restore")
        || extension == "marker"
        || stem
            .strip_prefix("restore-")
            .is_some_and(sigil_protocol::accounts::valid_credential))
    .then_some((id, true))
}

// Called under Store serialization. The worker's status protects its files
// while filesystem work runs outside the Store lock.
fn reclaim(db: &Connection, selected: Option<&str>) -> Result<(), StoreError> {
    if selected.is_some_and(|id| !sigil_protocol::accounts::valid_credential(id)) {
        return Err(invalid());
    }
    let dir = directory(db)?;
    let mut protected = db.prepare("SELECT id FROM operations WHERE status IN (1,2) UNION SELECT json_extract(action,'$.file') FROM operations WHERE status IN (1,2) AND json_extract(action,'$.file') IS NOT NULL LIMIT 129")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
    if protected.len() > 128 {
        return Err(invalid());
    }
    if selected.is_none() {
        let uploads = db
            .prepare("SELECT id FROM operation_uploads LIMIT 3")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        if uploads.len() > 2 {
            return Err(invalid());
        }
        protected.extend(uploads);
    }
    let marker = dir.parent().ok_or_else(invalid)?.join("restore.pending");
    match fs::symlink_metadata(&marker) {
        Ok(meta) => {
            if !meta.is_file() || meta.len() != 64 || meta.permissions().mode() & 0o077 != 0 {
                return Err(invalid());
            }
            let id = io(fs::read_to_string(marker))?;
            if !sigil_protocol::accounts::valid_credential(&id) {
                return Err(invalid());
            }
            protected.insert(id);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(invalid()),
    }
    if selected.is_some_and(|id| protected.contains(id)) {
        return Err(StoreError::Busy);
    }
    let mut changed = false;
    let mut entries = io(fs::read_dir(&dir))?;
    for entry in entries.by_ref().take(256) {
        let entry = io(entry)?;
        let name = entry.file_name();
        let Some((id, auxiliary)) = name.to_str().and_then(artifact) else {
            continue;
        };
        if protected.contains(id) || selected.map_or(!auxiliary, |wanted| wanted != id) {
            continue;
        }
        if !io(fs::symlink_metadata(entry.path()))?.is_file() {
            return Err(invalid());
        }
        io(fs::remove_file(entry.path()))?;
        changed = true;
    }
    if changed {
        io(io(fs::File::open(dir))?.sync_all())?;
    }
    if selected.is_some() && entries.next().is_some() {
        return Err(StoreError::Busy);
    }
    Ok(())
}
fn authorize(
    db: &Connection,
    actor: &[u8],
    bootstrap: bool,
    root: &[u8],
    now: u64,
) -> Result<(), StoreError> {
    if bootstrap {
        return if actor == root {
            Ok(())
        } else {
            Err(StoreError::Unauthorized)
        };
    }
    let account:Option<String>=db.query_row("SELECT account_id FROM devices d JOIN accounts a ON a.id=d.account_id WHERE d.token_hash=?1 AND d.revoked=0 AND d.expires_at>?2 AND a.disabled=0",(actor,sql(now)?),|r|r.get(0)).optional()?;
    if account.is_some_and(|a| {
        matches!(
            crate::admin::role(db, &a),
            Ok(sigil_protocol::admin::Role::Administrator)
        )
    }) {
        Ok(())
    } else {
        Err(StoreError::Unauthorized)
    }
}
fn configuration(db: &Connection) -> Result<Configuration, StoreError> {
    decode(&db.query_row(
        "SELECT value FROM operation_configuration WHERE id=1",
        [],
        |r| r.get::<_, String>(0),
    )?)
}
impl Action {
    fn effect(&self) -> &'static str {
        match self {
        Self::Backup=>"Create a private snapshot containing server keys and metadata; client-held encryption keys are excluded.",
        Self::Inspect{..}=>"Validate the selected backup without changing live data.",Self::Import{..}=>"Validate the uploaded backup and retain it privately.",
        Self::Restore{..}=>"Back up the current database and stage replacement for restart. Revoke restored sessions and disable restored external credentials; retained history still needs client recovery keys.",
        Self::CheckUpdate=>"Fetch and verify the operator-configured signed release manifest.",
        Self::CheckEndpoint=>"Check the configured public HTTPS origin through TLS and the reverse proxy using a temporary server proof.",
        Self::Upgrade{..}=>"Create a backup and verify the selected release. The deployment manager must recreate the container with the returned image digest; older schemas require the preserved backup for rollback."
    }
    }
    fn validate(&self) -> Result<(), StoreError> {
        match self {
            Self::Inspect { file } | Self::Import { file } | Self::Restore { file }
                if !sigil_protocol::accounts::valid_credential(file) =>
            {
                Err(invalid())
            }
            Self::Upgrade { release } => {
                validate_release(release)?;
                if version(&release.version) <= version(env!("CARGO_PKG_VERSION")) {
                    return Err(invalid());
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
impl Store {
    pub fn operation_configuration(&self) -> Result<Configuration, StoreError> {
        configuration(&self.0)
    }
    pub fn configure_operations(
        &mut self,
        mut update: Configuration,
    ) -> Result<Configuration, StoreError> {
        if !(1024 * 1024..=64 * 1024u64.pow(3)).contains(&update.max_backup_bytes)
            || update.release_key.is_some() != update.release_url.is_some()
        {
            return Err(invalid());
        }
        egress::Policy::new(update.exceptions.clone()).map_err(|_| invalid())?;
        if let Some(url) = &update.release_url {
            egress::endpoint(url).map_err(|_| invalid())?;
        }
        if update
            .release_key
            .as_ref()
            .is_some_and(|k| !sigil_protocol::accounts::valid_credential(k))
        {
            return Err(invalid());
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old = configuration(&tx)?;
        if old.revision != update.revision {
            return Err(StoreError::Conflict);
        }
        update.revision = crate::push_config::next(old.revision)?;
        tx.execute(
            "UPDATE operation_configuration SET value=?1 WHERE id=1",
            [encode(&update)?],
        )?;
        tx.commit()?;
        Ok(update)
    }
    pub fn prepare_operation(
        &mut self,
        credential: &str,
        bootstrap: bool,
        action: Action,
        now: u64,
    ) -> Result<Operation, StoreError> {
        action.validate()?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM operations WHERE status IN (0,3,4) AND expires<=?1",
            [sql(now)?],
        )?;
        if tx.query_row("SELECT count(*) FROM operations", [], |r| {
            r.get::<_, u32>(0)
        })? >= 64
        {
            return Err(StoreError::Busy);
        }
        let id = random_secret().map_err(|_| StoreError::InvalidData)?;
        let expires = crate::push::deadline(now, 300)?;
        tx.execute("INSERT INTO operations(id,actor,bootstrap,action,created,expires,probe) VALUES(?1,?2,?3,?4,?5,?6,?7)",(&id,digest(credential).as_slice(),bootstrap,encode(&action)?,sql(now)?,sql(expires)?,random_secret().map_err(|_|StoreError::InvalidData)?))?;
        tx.commit()?;
        self.operation(&id)
    }
    pub fn confirm_operation(
        &mut self,
        id: &str,
        credential: &str,
        now: u64,
    ) -> Result<Operation, StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (actor, status, expires): (Vec<u8>, u8, u64) = tx
            .query_row(
                "SELECT actor,status,expires FROM operations WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, unsigned(r, 2)?)),
            )
            .optional()?
            .ok_or(StoreError::NotFound)?;
        if actor != digest(credential) || expires <= now {
            return Err(StoreError::Unauthorized);
        }
        if status == 0 {
            tx.execute(
                "UPDATE operations SET status=1,expires=?2 WHERE id=?1",
                (id, sql(crate::push::deadline(now, 86400)?)?),
            )?;
        }
        tx.commit()?;
        self.operation(id)
    }
    pub fn operation(&self, id: &str) -> Result<Operation, StoreError> {
        let (value, expires, state, result): (String, u64, u8, Option<String>) = self
            .0
            .query_row(
                "SELECT action,expires,status,result FROM operations WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, unsigned(r, 1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?
            .ok_or(StoreError::NotFound)?;
        let action: Action = decode(&value)?;
        let state = match state {
            0 => "confirmation_required",
            1 => "queued",
            2 => "running",
            3 => "complete",
            4 => "failed",
            _ => return Err(StoreError::InvalidData),
        };
        Ok(Operation {
            id: id.into(),
            effect: action.effect().into(),
            action,
            expires_at: expires,
            state: state.into(),
            result: result.map(|s| decode(&s)).transpose()?,
        })
    }
    pub fn operations(&self) -> Result<Vec<Operation>, StoreError> {
        let ids = self
            .0
            .prepare("SELECT id FROM operations ORDER BY created DESC,id LIMIT 64")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.iter().map(|id| self.operation(id)).collect()
    }
    pub fn operation_diagnostics(&self) -> Result<serde_json::Value, StoreError> {
        let jobs: u32 = self.0.query_row(
            "SELECT count(*) FROM operations WHERE status IN (1,2)",
            [],
            |r| r.get(0),
        )?;
        let failed: u32 =
            self.0
                .query_row("SELECT count(*) FROM operations WHERE status=4", [], |r| {
                    r.get(0)
                })?;
        let backup:Option<i64>=self.0.query_row("SELECT max(created) FROM operations WHERE status=3 AND json_extract(action,'$.kind') IN ('backup','restore','upgrade')",[],|r|r.get(0))?;
        Ok(
            serde_json::json!({"queued_or_running":jobs,"failed":failed,"last_backup_at":backup,"updates_configured":configuration(&self.0)?.release_url.is_some(),"restore_pending":directory(&self.0)?.parent().ok_or_else(invalid)?.join("restore.pending").exists()}),
        )
    }
    pub(crate) fn endpoint_probe(&self, id: &str, now: u64) -> Result<String, StoreError> {
        self.0.query_row("SELECT probe FROM operations WHERE id=?1 AND status=2 AND expires>?2 AND json_extract(action,'$.kind')='check_endpoint'",(id,sql(now)?),|r|r.get(0)).optional()?.ok_or(StoreError::NotFound)
    }
    pub fn backup_upload(&self, id: &str) -> Result<serde_json::Value, StoreError> {
        self.0.query_row("SELECT bytes,sha256,received FROM operation_uploads WHERE id=?1",[id],|r|Ok(serde_json::json!({"bytes":unsigned(r,0)?,"sha256":r.get::<_,String>(1)?,"received":unsigned(r,2)?}))).optional()?.ok_or(StoreError::NotFound)
    }
    pub fn prepare_backup_upload(&mut self, upload: Upload) -> Result<String, StoreError> {
        reclaim(&self.0, None)?;
        if io(fs::read_dir(directory(&self.0)?))?.take(120).count() >= 120 {
            return Err(StoreError::Busy);
        }
        let config = configuration(&self.0)?;
        if upload.bytes < 4096
            || upload.bytes > config.max_backup_bytes
            || !sigil_protocol::accounts::valid_credential(&upload.sha256)
        {
            return Err(invalid());
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.query_row("SELECT count(*) FROM operation_uploads", [], |r| {
            r.get::<_, u32>(0)
        })? >= 2
        {
            return Err(StoreError::Busy);
        }
        let id = random_secret().map_err(|_| StoreError::InvalidData)?;
        tx.execute(
            "INSERT INTO operation_uploads(id,bytes,sha256) VALUES(?1,?2,?3)",
            (&id, sql(upload.bytes)?, upload.sha256),
        )?;
        tx.commit()?;
        Ok(id)
    }
    pub fn upload_backup_chunk(
        &mut self,
        id: &str,
        offset: u64,
        bytes: &[u8],
    ) -> Result<u64, StoreError> {
        if bytes.is_empty() || bytes.len() > 4 * 1024 * 1024 {
            return Err(invalid());
        }
        let dir = directory(&self.0)?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (total, received): (u64, u64) = tx
            .query_row(
                "SELECT bytes,received FROM operation_uploads WHERE id=?1",
                [id],
                |r| Ok((unsigned(r, 0)?, unsigned(r, 1)?)),
            )
            .optional()?
            .ok_or(StoreError::NotFound)?;
        let end = offset
            .checked_add(bytes.len() as u64)
            .filter(|n| *n <= total)
            .ok_or_else(invalid)?;
        if offset > received {
            return Err(StoreError::Conflict);
        }
        let path = file(&dir, id, "part")?;
        if !path.exists() {
            io(private_file(&path))?;
            io(fs::File::open(&dir))?
                .sync_all()
                .map_err(|_| StoreError::InvalidData)?;
        }
        use std::io::{Seek, SeekFrom};
        let mut output = io(fs::OpenOptions::new().read(true).write(true).open(&path))?;
        if offset < received {
            if end > received {
                return Err(StoreError::Conflict);
            }
            io(output.seek(SeekFrom::Start(offset)))?;
            let mut old = vec![0; bytes.len()];
            io(output.read_exact(&mut old))?;
            return if old == bytes {
                Ok(received)
            } else {
                Err(StoreError::Conflict)
            };
        }
        io(output.set_len(received))?;
        io(output.seek(SeekFrom::Start(received)))?;
        io(output.write_all(bytes))?;
        io(output.sync_all())?;
        tx.execute(
            "UPDATE operation_uploads SET received=?2 WHERE id=?1",
            (id, sql(end)?),
        )?;
        tx.commit()?;
        Ok(end)
    }
    pub fn delete_backup(&mut self, id: &str) -> Result<(), StoreError> {
        reclaim(&self.0, Some(id))?;
        self.0
            .execute("DELETE FROM operation_uploads WHERE id=?1", [id])?;
        Ok(())
    }
    pub fn backup_files(&self) -> Result<Vec<serde_json::Value>, StoreError> {
        let dir = directory(&self.0)?;
        let mut result = Vec::new();
        for item in io(fs::read_dir(dir))?.take(256) {
            let item = io(item)?;
            let path = item.path();
            if path.extension().and_then(|s| s.to_str()) != Some("db") {
                continue;
            }
            let Some(id) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .filter(|s| sigil_protocol::accounts::valid_credential(s))
            else {
                continue;
            };
            let meta = io(fs::symlink_metadata(&path))?;
            if !meta.is_file() {
                return Err(invalid());
            }
            result.push(serde_json::json!({"id":id,"bytes":meta.len()}));
        }
        result.sort_by_key(|v| v["id"].as_str().unwrap_or("").to_owned());
        Ok(result)
    }
    pub fn backup_chunk(&self, id: &str, offset: u64) -> Result<Vec<u8>, StoreError> {
        use std::io::{Seek, SeekFrom};
        let path = file(&directory(&self.0)?, id, "db")?;
        let mut input = io(fs::File::open(path))?;
        if offset > io(input.metadata())?.len() {
            return Err(invalid());
        }
        io(input.seek(SeekFrom::Start(offset)))?;
        let mut bytes = Vec::new();
        io(input.take(4 * 1024 * 1024).read_to_end(&mut bytes))?;
        Ok(bytes)
    }
    fn claim_operation(&mut self, root: &[u8], now: u64) -> Result<Option<Job>, StoreError> {
        reclaim(&self.0, None)?;
        let directory = directory(&self.0)?;
        let source = io(fs::canonicalize(
            self.0.path().ok_or(StoreError::InvalidData)?,
        ))?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("UPDATE operations SET status=4,result='\"expired\"' WHERE status IN (1,2) AND expires<=?1",[sql(now)?])?;
        let row:Option<(String,Vec<u8>,bool,String)>=tx.query_row("SELECT id,actor,bootstrap,action FROM operations WHERE status IN (1,2) AND lease<=?1 ORDER BY created,id LIMIT 1",[sql(now)?],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let Some((id, actor, bootstrap, value)) = row else {
            tx.commit()?;
            return Ok(None);
        };
        if authorize(&tx, &actor, bootstrap, root, now).is_err() {
            tx.execute(
                "UPDATE operations SET status=4,result='\"authorization_revoked\"' WHERE id=?1",
                [id],
            )?;
            tx.commit()?;
            return Ok(None);
        }
        let action: Action = decode(&value)?;
        let config = configuration(&tx)?;
        let upload = if let Action::Import { file } = &action {
            tx.query_row(
                "SELECT bytes,sha256,received FROM operation_uploads WHERE id=?1",
                [file],
                |r| Ok((unsigned(r, 0)?, r.get::<_, String>(1)?, unsigned(r, 2)?)),
            )
            .optional()?
        } else {
            None
        };
        let origin = crate::admin::policy(&tx)?.public_origin;
        let probe: String =
            tx.query_row("SELECT probe FROM operations WHERE id=?1", [&id], |r| {
                r.get(0)
            })?;
        tx.execute(
            "UPDATE operations SET status=2,lease=?2 WHERE id=?1",
            (&id, sql(crate::push::deadline(now, 120)?)?),
        )?;
        tx.commit()?;
        Ok(Some(Job {
            id,
            directory,
            source,
            action,
            config,
            upload,
            origin,
            probe,
        }))
    }
    fn finish_operation(
        &mut self,
        id: &str,
        result: Result<serde_json::Value, StoreError>,
    ) -> Result<(), StoreError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (status, value) = match result {
            Ok(v) => (3, encode(&v)?),
            Err(StoreError::Busy) => (4, "\"resource_or_provider_unavailable\"".into()),
            Err(StoreError::Conflict) => (4, "\"state_changed\"".into()),
            Err(_) => (4, "\"validation_failed\"".into()),
        };
        tx.execute(
            "UPDATE operations SET status=?2,result=?3,lease=0 WHERE id=?1 AND status=2",
            (id, status, value),
        )?;
        if status == 3 {
            tx.execute("DELETE FROM operation_uploads WHERE id=(SELECT json_extract(action,'$.file') FROM operations WHERE id=?1 AND json_extract(action,'$.kind')='import')",[id])?;
        }
        tx.commit()?;
        Ok(())
    }
}
struct Job {
    id: String,
    directory: PathBuf,
    source: PathBuf,
    action: Action,
    config: Configuration,
    upload: Option<(u64, String, u64)>,
    origin: Option<String>,
    probe: String,
}
fn checksum(path: &Path, max: u64) -> Result<(u64, String), StoreError> {
    let meta = io(fs::symlink_metadata(path))?;
    if !meta.is_file() || meta.len() > max || meta.permissions().mode() & 0o077 != 0 {
        return Err(invalid());
    }
    let mut input = io(fs::File::open(path))?;
    let mut hash = Sha256::new();
    let mut bytes = [0u8; 65536];
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if Instant::now() >= deadline {
            return Err(StoreError::Busy);
        }
        let n = io(input.read(&mut bytes))?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    Ok((meta.len(), crate::federation_auth::hex(&hash.finalize())))
}
fn inspect(path: &Path, max: u64) -> Result<serde_json::Value, StoreError> {
    let (bytes, hash) = checksum(path, max)?;
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    db.progress_handler(10000, Some(move || Instant::now() >= deadline))?;
    crate::store::schema_guard(&db)?;
    let id: i64 = db.query_row("PRAGMA application_id", [], |r| r.get(0))?;
    let schema: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if id != 0x5349474c || !(1..=crate::store::SCHEMA_VERSION).contains(&schema) {
        return Err(invalid());
    }
    let check: String = db.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    if check != "ok" {
        return Err(invalid());
    }
    Ok(serde_json::json!({"bytes":bytes,"sha256":hash,"schema":schema}))
}
impl Job {
    fn backup(&self) -> Result<serde_json::Value, StoreError> {
        let output = file(&self.directory, &self.id, "db")?;
        if !output.exists() {
            if io(fs::read_dir(&self.directory))?.take(120).count() >= 120 {
                return Err(StoreError::Busy);
            }
            let source = Store(Connection::open_with_flags(
                &self.source,
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?);
            let staged = file(&self.directory, &self.id, "backup")?;
            if staged.exists() {
                io(fs::remove_file(&staged))?;
            }
            source.backup_bounded(&staged, self.config.max_backup_bytes)?;
            io(fs::rename(staged, &output))?;
            io(fs::File::open(&self.directory))?
                .sync_all()
                .map_err(|_| StoreError::InvalidData)?;
        }
        let mut result = inspect(&output, self.config.max_backup_bytes)?;
        result["file"] = self.id.clone().into();
        Ok(result)
    }
    fn perform(&self) -> Result<serde_json::Value, StoreError> {
        match &self.action {
            Action::Backup => self.backup(),
            Action::Inspect { file: id } => inspect(
                &file(&self.directory, id, "db")?,
                self.config.max_backup_bytes,
            ),
            Action::Import { file: id } => {
                let (bytes, expected, received) = self.upload.as_ref().ok_or_else(invalid)?;
                if bytes != received {
                    return Err(StoreError::Conflict);
                }
                let destination = file(&self.directory, id, "db")?;
                let source = if destination.exists() {
                    destination.clone()
                } else {
                    file(&self.directory, id, "part")?
                };
                let result = inspect(&source, self.config.max_backup_bytes)?;
                if result["sha256"].as_str() != Some(expected)
                    || result["bytes"].as_u64() != Some(*bytes)
                {
                    return Err(invalid());
                }
                if source != destination {
                    io(fs::rename(source, destination))?;
                }
                io(fs::File::open(&self.directory))?
                    .sync_all()
                    .map_err(|_| StoreError::InvalidData)?;
                Ok(result)
            }
            Action::Restore { file: id } => {
                let source = file(&self.directory, id, "db")?;
                inspect(&source, self.config.max_backup_bytes)?;
                let backup = self.backup()?;
                let staged = file(&self.directory, &self.id, "restore")?;
                if !staged.exists() {
                    Store::restore(&source, &staged)?;
                }
                let marker = self
                    .directory
                    .parent()
                    .ok_or_else(invalid)?
                    .join("restore.pending");
                if marker.exists() {
                    if io(fs::read_to_string(&marker))? != self.id {
                        return Err(StoreError::Conflict);
                    }
                } else {
                    let temporary = file(&self.directory, &self.id, "marker")?;
                    if temporary.exists() {
                        io(fs::remove_file(&temporary))?;
                    }
                    let mut output = io(private_file(&temporary))?;
                    io(output.write_all(self.id.as_bytes()))?;
                    io(output.sync_all())?;
                    io(fs::hard_link(&temporary, &marker))?;
                    io(fs::remove_file(temporary))?;
                }
                io(fs::File::open(marker.parent().ok_or_else(invalid)?))?
                    .sync_all()
                    .map_err(|_| StoreError::InvalidData)?;
                Ok(serde_json::json!({"restart_required":true,"backup":backup}))
            }
            Action::CheckUpdate => {
                let release = release(&self.config)?;
                Ok(
                    serde_json::json!({"available":version(&release.version)>version(env!("CARGO_PKG_VERSION")),"release":release}),
                )
            }
            Action::CheckEndpoint => {
                let origin = self.origin.as_ref().ok_or_else(invalid)?;
                let url = format!("{origin}/setup/v0/probe/{}", self.id);
                let request = ureq::http::Request::get(url)
                    .body(&[][..])
                    .map_err(|_| invalid())?;
                let response = egress::Policy::new(self.config.exceptions.clone())
                    .and_then(|p| p.service(request))
                    .map_err(|_| StoreError::Busy)?;
                if response.status != 200 || response.body.as_slice() != self.probe.as_bytes() {
                    return Err(invalid());
                }
                Ok(serde_json::json!({"https_reaches_this_server":true,"origin":origin}))
            }
            Action::Upgrade { release: chosen } => {
                if release(&self.config)? != *chosen {
                    return Err(StoreError::Conflict);
                }
                let backup = self.backup()?;
                Ok(
                    serde_json::json!({"release":chosen,"backup":backup,"deployment_action":"recreate_container_at_verified_digest","rollback":"restore preserved backup into the previous image; never open a newer schema with an older binary"}),
                )
            }
        }
    }
}
fn version(value: &str) -> Option<[u32; 3]> {
    if value.len() > 32 {
        return None;
    }
    let values = value
        .split('.')
        .map(|s| {
            if s.is_empty()
                || !s.bytes().all(|b| b.is_ascii_digit())
                || (s.len() > 1 && s.starts_with('0'))
            {
                None
            } else {
                s.parse().ok()
            }
        })
        .collect::<Option<Vec<u32>>>()?;
    values.try_into().ok()
}
fn validate_release(value: &Release) -> Result<(), StoreError> {
    if version(&value.version).is_none()
        || !value
            .image_digest
            .strip_prefix("sha256:")
            .is_some_and(sigil_protocol::accounts::valid_credential)
        || value.minimum_schema > crate::store::SCHEMA_VERSION as u32
        || value.target_schema < crate::store::SCHEMA_VERSION as u32
    {
        return Err(invalid());
    }
    Ok(())
}
fn release(config: &Configuration) -> Result<Release, StoreError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Envelope {
        release: Release,
        signature: String,
    }
    let url = config
        .release_url
        .as_ref()
        .ok_or(StoreError::Invalid("configure a trusted release source"))?;
    let request = ureq::http::Request::get(url)
        .body(&[][..])
        .map_err(|_| invalid())?;
    let response = egress::Policy::new(config.exceptions.clone())
        .and_then(|p| p.service(request))
        .map_err(|_| StoreError::Busy)?;
    if response.status != 200 {
        return Err(StoreError::Busy);
    }
    let envelope: Envelope = serde_json::from_slice(&response.body).map_err(|_| invalid())?;
    let key = crate::push::bytes(config.release_key.as_deref().ok_or_else(invalid)?)?;
    let signature = crate::group_authority::decode(&envelope.signature, 64)?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
        .verify(encode(&envelope.release)?.as_bytes(), &signature)
        .map_err(|_| invalid())?;
    validate_release(&envelope.release)?;
    Ok(envelope.release)
}
pub(crate) async fn run(state: AppState) {
    loop {
        let root = state.token.fingerprint();
        if let Ok(Some(job)) =
            with_store(state.clone(), move |s| s.claim_operation(&root, now()?)).await
        {
            let result = tokio::task::spawn_blocking(move || {
                let result = job.perform();
                (job.id, result)
            })
            .await;
            if let Ok((id, result)) = result {
                let _ = with_store(state.clone(), move |s| s.finish_operation(&id, result)).await;
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
/// Called only while holding the installation lock, before opening the live database.
pub fn activate_restore(directory: &Path) -> Result<(), StoreError> {
    let marker = directory.join("restore.pending");
    if !marker.exists() {
        return Ok(());
    }
    let meta = io(fs::symlink_metadata(&marker))?;
    if !meta.is_file() || meta.len() != 64 || meta.permissions().mode() & 0o077 != 0 {
        return Err(invalid());
    }
    let id = io(fs::read_to_string(&marker))?;
    let maintenance = directory.join("maintenance");
    let meta = io(fs::symlink_metadata(&maintenance))?;
    if !meta.is_dir() || meta.permissions().mode() & 0o077 != 0 {
        return Err(invalid());
    }
    let staged = file(&maintenance, &id, "restore")?;
    inspect(&staged, 64 * 1024u64.pow(3))?;
    let previous = directory.join(format!("sigil.pre-restore-{id}.db"));
    let current = directory.join("sigil.db");
    if !previous.exists() {
        io(fs::rename(&current, &previous))?;
    }
    for suffix in ["-journal", "-wal", "-shm"] {
        let old = directory.join(format!("sigil.db{suffix}"));
        if old.exists() {
            io(fs::rename(
                old,
                directory.join(format!("sigil.pre-restore-{id}.db{suffix}")),
            ))?;
        }
    }
    io(fs::File::open(directory))?
        .sync_all()
        .map_err(|_| StoreError::InvalidData)?;
    if !current.exists() {
        io(fs::hard_link(&staged, &current))?;
    } else {
        let a = io(fs::symlink_metadata(&staged))?;
        let b = io(fs::symlink_metadata(&current))?;
        if a.ino() != b.ino() || a.dev() != b.dev() {
            return Err(invalid());
        }
    }
    io(fs::File::open(directory))?
        .sync_all()
        .map_err(|_| StoreError::InvalidData)?;
    io(fs::remove_file(marker))?;
    io(fs::File::open(directory))?
        .sync_all()
        .map_err(|_| StoreError::InvalidData)?;
    io(fs::remove_file(staged))?;
    Ok(())
}

#[cfg(test)]
#[path = "operation_tests.rs"]
mod tests;
