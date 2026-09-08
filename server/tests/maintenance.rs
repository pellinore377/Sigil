use sigil_server::store::Store;
fn seeded(path: &std::path::Path) -> (Store, rusqlite::Connection) {
    let store = Store::open(path).unwrap();
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute_batch("INSERT INTO accounts(id,username) VALUES('account','synthetic'); INSERT INTO devices(id,account_id,label,expires_at) VALUES('device','account','Synthetic',999999);
    WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<66)
    INSERT INTO mailbox(sender,message_id,recipient,payload,payload_hash,expires_at) SELECT 'device',printf('%064x',x),'device','abab',zeroblob(32),CASE WHEN x=66 THEN 2000 ELSE 1000 END FROM n;
    WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<66)
    INSERT INTO prekeys(id,device_id,bundle,bundle_hash,expires_at,claimant,request_id) SELECT printf('%064x',x),'device',x'abab',zeroblob(32),CASE WHEN x=66 THEN 2000 ELSE 1000 END,'device',printf('%064x',x) FROM n;
    WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<66)
    INSERT INTO invitations(id,token_hash,username,expires_at) SELECT printf('%064x',x),printf('%064x',x),printf('user%d',x),CASE WHEN x=66 THEN 2000 ELSE 1000 END FROM n;
    INSERT INTO contact_invitations(id,owner,expires_at) VALUES('contact','device',1000);").unwrap();
    db.execute(
        "INSERT INTO retained_storage VALUES('account',?1)",
        [2048 + 66 * (512 + 514)],
    )
    .unwrap();
    (store, db)
}
#[test]
fn batches_are_bounded_preserve_live_payloads_and_keep_retry_tombstones() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, db) = seeded(&dir.path().join("sigil.db"));
    assert_eq!(store.expire_batch(999).unwrap(), 0);
    assert_eq!(store.expire_batch(1000).unwrap(), 192);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM mailbox WHERE payload IS NOT NULL",
            [],
            |r| r.get::<_, u32>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(store.expire_batch(1000).unwrap(), 3);
    assert_eq!(store.expire_batch(1000).unwrap(), 0);
    assert_eq!(
        db.query_row("SELECT count(*) FROM mailbox", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        66
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM prekeys WHERE claimant='device' AND request_id IS NOT NULL",
            [],
            |r| r.get::<_, u32>(0)
        )
        .unwrap(),
        66
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM invitations", [], |r| r
            .get::<_, u32>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM contact_invitations", [], |r| r
            .get::<_, u32>(0))
            .unwrap(),
        1
    );
    assert!(store.expire_batch(u64::MAX).is_err());
}
#[test]
fn failed_batch_rolls_back_all_tables() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, db) = seeded(&dir.path().join("sigil.db"));
    db.execute_batch("CREATE TRIGGER maintenance_fault BEFORE UPDATE ON prekeys BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    let before: i64 = db
        .query_row("SELECT bytes FROM retained_storage", [], |r| r.get(0))
        .unwrap();
    assert!(store.expire_batch(1000).is_err());
    assert_eq!(
        db.query_row("SELECT bytes FROM retained_storage", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        before
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM mailbox WHERE payload IS NOT NULL",
            [],
            |r| r.get::<_, u32>(0)
        )
        .unwrap(),
        66
    );
    db.execute_batch("DROP TRIGGER maintenance_fault;").unwrap();
    assert_eq!(store.expire_batch(1000).unwrap(), 192);
    assert_eq!(
        db.query_row("SELECT bytes FROM retained_storage", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        before - 128
    );
}
#[test]
fn schema_six_migration_preserves_payloads_until_expiry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let (store, db) = seeded(&path);
    drop(store);
    db.execute_batch(
        "DROP TABLE operation_uploads; DROP TABLE operations; DROP TABLE operation_configuration; DROP TABLE oidc_grants; DROP TABLE oidc_bindings; DROP TABLE oidc_flows; DROP TABLE oidc_configuration; DROP TABLE registration_usage; DROP TABLE account_policy; DROP TABLE admin_policy; DROP TABLE call_connections; DROP TABLE calls; DROP TABLE call_configuration; DROP TABLE service_budgets; DROP TABLE service_configuration; DROP TABLE map_configuration; DROP TABLE private_group_invitations; DROP TABLE private_group_proposals; DROP TABLE private_group_nonces; DROP TABLE private_group_commits; DROP TABLE private_group_members; DROP TABLE private_groups; DROP TABLE group_credential_uids; DROP TABLE group_authority; DROP INDEX prekeys_available; DROP INDEX prekeys_remote_claim; ALTER TABLE prekeys DROP COLUMN remote_request; ALTER TABLE prekeys DROP COLUMN remote_device; ALTER TABLE prekeys DROP COLUMN remote_account; ALTER TABLE prekeys DROP COLUMN remote_server; CREATE INDEX prekeys_available ON prekeys(device_id,expires_at) WHERE bundle IS NOT NULL AND claimant IS NULL; DROP TABLE federation_outbox; DROP TABLE federation_revocations; DROP TABLE federation_senders; DROP TABLE federation_nonces; DROP TABLE federation_admission; DROP TABLE federation_usage; DROP TABLE federation_peers; DROP TABLE federation_configuration; DROP TABLE push_jobs; DROP TABLE push_channels; DROP TABLE push_configuration; DROP TABLE attachment_chunks; DROP TABLE attachments; DROP TABLE retained_storage; DROP INDEX mailbox_live_recipient; DROP INDEX prekeys_available; DROP INDEX devices_active_account; DROP TABLE cancelled_device_links; DROP TABLE device_links; DROP INDEX devices_account_id; DROP TABLE device_bindings; DROP TABLE recovery_heads; DROP TABLE recovery_objects; DROP INDEX prekeys_expiry; DROP INDEX invitations_expiry; PRAGMA user_version=6;",
    )
    .unwrap();
    let mut store = Store::open(&path).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        27
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM mailbox WHERE payload IS NOT NULL",
            [],
            |r| r.get::<_, u32>(0)
        )
        .unwrap(),
        66
    );
    assert_eq!(store.expire_batch(1000).unwrap(), 192);
}
