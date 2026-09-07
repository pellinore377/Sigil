//! Index active work separately from retained history and authorization evidence.
use super::*;
pub(super) fn migrate(tx: &Transaction<'_>, key: &StorageKey) -> Result<(), Error> {
    tx.execute_batch("CREATE INDEX prekeys_live ON prekeys(id) WHERE state IS NOT NULL;
        CREATE INDEX claims_pending ON prekey_claims(id) WHERE phase=0;
        CREATE INDEX sessions_live ON sessions(peer,id) WHERE retired=0;
        CREATE INDEX incoming_pending ON incoming(sequence) WHERE acknowledged=0;
        CREATE INDEX retry_incoming_pending ON retry_incoming(sequence) WHERE acknowledged=0;
        CREATE INDEX recovered_pending ON recovered_deliveries(sequence) WHERE acknowledged=0;
        CREATE TABLE control_dependencies(kind INTEGER NOT NULL CHECK(kind IN (0,1)),id BLOB NOT NULL,parent BLOB NOT NULL,expires INTEGER NOT NULL CHECK(expires>0),PRIMARY KEY(kind,id));
        CREATE INDEX control_children ON control_dependencies(kind,parent,id);
        CREATE INDEX control_deadlines ON control_dependencies(kind,expires,id);
        CREATE TABLE control_journals(sequence INTEGER PRIMARY KEY,control BLOB NOT NULL);
        CREATE INDEX control_journal_parent ON control_journals(control,sequence);
        CREATE TABLE control_cleanup_cursor(kind INTEGER PRIMARY KEY CHECK(kind IN (0,1)),state BLOB NOT NULL);
        ALTER TABLE peers ADD COLUMN obsolete INTEGER NOT NULL DEFAULT 0 CHECK(obsolete IN (0,1));
        CREATE INDEX peers_current ON peers(id) WHERE obsolete=0;")?;
    peers::migrate_lifetime(tx, key)?;
    retry::migrate_lifetime(tx, key)?;
    Ok(())
}
