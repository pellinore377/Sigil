//! Shared private-file and bounded-page setup; callers own schema/key validation.
use crate::Error;
use rusqlite::Connection;
#[cfg(not(target_arch = "wasm32"))]
use std::fs;
use std::{path::Path, time::Duration};

pub(crate) fn open(path: &Path, bytes: u64) -> Result<Connection, Error> {
    if !(1024 * 1024..=1024 * 1024 * 1024 * 1024).contains(&bytes) {
        return Err(Error::Limit);
    }
    let db = connection(path)?;
    #[cfg(target_arch = "wasm32")]
    db.busy_timeout(Duration::from_secs(5))?;
    #[cfg(not(target_arch = "wasm32"))]
    db.busy_handler(Some(retry_busy))?;
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

#[cfg(not(target_arch = "wasm32"))]
fn retry_busy(attempt: i32) -> bool {
    // SQLite's default backoff reaches 100 ms. A short writer between video frames
    // must not leave the reader asleep for several more frames after it commits.
    thread_local! {
        static START: std::cell::Cell<std::time::Instant> = std::cell::Cell::new(std::time::Instant::now());
    }
    START.with(|start| {
        if attempt == 0 { start.set(std::time::Instant::now()); }
        if start.get().elapsed() >= Duration::from_secs(5) { return false; }
        std::thread::sleep(Duration::from_millis(1));
        true
    })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod contention_tests {
    use super::*;
    #[test]
    fn competing_writer_is_observed_and_a_stuck_writer_times_out() {
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        { use std::os::unix::fs::PermissionsExt;
          std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap(); }
        let path = directory.path().join("synthetic.db");
        let reader = open(&path, 1024 * 1024).unwrap();
        reader.execute_batch("CREATE TABLE item(value INTEGER); INSERT INTO item VALUES(1)").unwrap();
        let writer = open(&path, 1024 * 1024).unwrap();
        writer.execute_batch("BEGIN EXCLUSIVE; UPDATE item SET value=2").unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(35));
            writer.execute_batch("COMMIT").unwrap();
            writer
        });
        assert_eq!(reader.query_row("SELECT value FROM item", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
        let writer = release.join().unwrap();
        writer.execute_batch("BEGIN EXCLUSIVE; UPDATE item SET value=3").unwrap();
        let started = std::time::Instant::now();
        let error = reader.query_row("SELECT value FROM item", [], |r| r.get::<_, i64>(0)).unwrap_err();
        assert!(matches!(error, rusqlite::Error::SqliteFailure(e, _) if e.code == rusqlite::ErrorCode::DatabaseBusy));
        assert!(started.elapsed() >= Duration::from_secs(5));
        assert!(started.elapsed() < Duration::from_secs(7));
        writer.execute_batch("ROLLBACK").unwrap();
        assert_eq!(reader.query_row("SELECT value FROM item", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
        assert_eq!(reader.query_row("PRAGMA synchronous", [], |r| r.get::<_, i64>(0)).unwrap(), 3);
        assert_eq!(reader.query_row("PRAGMA secure_delete", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(reader.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0)).unwrap(), "delete");
    }
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

#[cfg(not(target_arch = "wasm32"))]
fn connection(path: &Path) -> Result<Connection, Error> {
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
    Ok(Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
    )?)
}

#[cfg(target_arch = "wasm32")]
fn connection(path: &Path) -> Result<Connection, Error> {
    if !matches!(
        path.to_str(),
        Some("/sigil/messages.db" | "/sigil/attachments.db")
    ) {
        return Err(Error::InvalidStore);
    }
    Ok(Connection::open_with_flags_and_vfs(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE,
        "sigil-opfs",
    )?)
}
