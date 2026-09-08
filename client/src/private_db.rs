//! Shared private-file and bounded-page setup; callers own schema/key validation.
use crate::Error;
use rusqlite::Connection;
use std::{fs, path::Path, time::Duration};

pub(crate) fn open(path: &Path, bytes: u64) -> Result<Connection, Error> {
    if !(1024 * 1024..=1024 * 1024 * 1024 * 1024).contains(&bytes) {
        return Err(Error::Limit);
    }
    let parent = path.parent().ok_or(Error::InvalidStore)?;
    let metadata = fs::symlink_metadata(parent)?;
    if !metadata.is_dir() {
        return Err(Error::InvalidStore);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(Error::InvalidStore);
        }
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
        {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    #[cfg(not(unix))]
    return Err(Error::InvalidStore); // A platform ACL adapter is still required.
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() {
        return Err(Error::InvalidStore);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(Error::InvalidStore);
        }
    }
    let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    db.busy_timeout(Duration::from_secs(5))?;
    db.execute_batch(
        "PRAGMA foreign_keys=ON; PRAGMA trusted_schema=OFF; PRAGMA synchronous=EXTRA; PRAGMA secure_delete=ON;",
    )?;
    db.set_db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    let page_size: i64 = db.query_row("PRAGMA page_size", [], |r| r.get(0))?;
    if page_size <= 0 {
        return Err(Error::InvalidStore);
    }
    let pages = bytes as i64 / page_size;
    db.pragma_update(None, "max_page_count", pages)?;
    if db.query_row("PRAGMA max_page_count", [], |r| r.get::<_, i64>(0))? > pages {
        return Err(Error::Limit);
    }
    Ok(db)
}

pub(crate) const MIGRATION: &str = "CREATE TABLE IF NOT EXISTS storage_cleanup(id INTEGER PRIMARY KEY CHECK(id=1),pending INTEGER NOT NULL CHECK(pending IN(0,1))); INSERT INTO storage_cleanup VALUES(1,1) ON CONFLICT(id) DO UPDATE SET pending=1;";

pub(crate) fn finish(db: &Connection) -> Result<(), Error> {
    let mode: String = db.query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))?;
    if mode != "delete" {
        return Err(Error::InvalidStore);
    }
    if db.query_row("SELECT pending FROM storage_cleanup WHERE id=1", [], |r| {
        r.get::<_, bool>(0)
    })? {
        // Retry after interruption; older free pages may predate secure_delete.
        db.execute_batch("VACUUM; UPDATE storage_cleanup SET pending=0 WHERE id=1;")?;
    }
    Ok(())
}
