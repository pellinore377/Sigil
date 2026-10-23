use super::*;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;

const DAY: u64 = 86400;

struct Keys {
    access: String,
    manage: String,
}
impl Keys {
    fn new(seed: u8) -> Self {
        Self { access: format!("{seed:02x}").repeat(32), manage: format!("{:02x}", seed.wrapping_add(1)).repeat(32) }
    }
    fn hash(token: &str) -> String {
        hex(&Token::parse(token).unwrap().hash())
    }
    fn access(&self) -> Token {
        Token::parse(&self.access).unwrap()
    }
    fn manage(&self) -> Token {
        Token::parse(&self.manage).unwrap()
    }
}
fn sealed(byte: u8, len: usize) -> String {
    format!("{byte:02x}").repeat(len)
}
fn page_id(seed: u8) -> String {
    format!("{seed:02x}").repeat(32)
}
fn create(store: &mut Store, credential: &str, id: &str, keys: &Keys, expires_at: Option<u64>, now: u64) -> std::result::Result<PageInfo, PageError> {
    store.page_create(
        credential,
        CreatePage {
            id: id.into(),
            access_hash: Keys::hash(&keys.access),
            manage_hash: Keys::hash(&keys.manage),
            expires_at,
            snapshot: sealed(1, 40),
        },
        now,
    )
}
fn name(label: &str) -> String {
    hex(&Sha256::digest(label)[..16])
}
fn op(epoch: u64, base: u64, request: &str, byte: u8) -> AppendOp {
    AppendOp { epoch, base, request: name(request), body: sealed(byte, 32) }
}
fn append(store: &mut Store, credential: &str, id: &str, keys: &Keys, request: AppendOp, now: u64) -> std::result::Result<Appended, PageError> {
    store.page_append(credential, id, &keys.access(), request, now, |_| true)
}
fn retained(store: &Store, account: &str) -> u64 {
    store.0.query_row("SELECT bytes FROM retained_storage WHERE account_id=?1", [account], |r| unsigned(r, 0)).unwrap()
}
fn account(store: &Store, credential: &str, now: u64) -> String {
    store.session(credential, now).unwrap().account_id
}
fn rebuilt(store: &mut Store, account: &str) -> u64 {
    let tx = store.0.transaction().unwrap();
    crate::storage_budget::rebuild(&tx).unwrap();
    let value = tx.query_row("SELECT bytes FROM retained_storage WHERE account_id=?1", [account], |r| unsigned(r, 0)).unwrap();
    tx.rollback().unwrap();
    value
}
fn cleanup_at(store: &mut Store, now: u64) -> usize {
    let tx = store.0.transaction().unwrap();
    let changed = cleanup(&tx, now).unwrap();
    tx.commit().unwrap();
    changed
}

#[test]
fn create_is_retry_safe_charges_the_owner_and_ids_never_return() {
    let (_dir, mut store, alice, bob, now) = crate::admin::tests::setup();
    let owner = account(&store, &alice, now);
    let before = retained(&store, &owner);
    let (id, keys) = (page_id(7), Keys::new(1));
    let info = create(&mut store, &alice, &id, &keys, None, now).unwrap();
    assert_eq!(info, PageInfo { epoch: 1, head: 0, snapshot_seq: 0, bytes: 40, expires_at: None });
    assert_eq!(create(&mut store, &alice, &id, &keys, None, now).unwrap(), info);
    assert_eq!(retained(&store, &owner), before + PAGE + ROW + 40);
    assert!(matches!(create(&mut store, &bob, &id, &keys, None, now), Err(PageError::Exists)));
    assert!(matches!(create(&mut store, &alice, &id, &Keys::new(3), None, now), Err(PageError::Exists)));
    let same = CreatePage { id: page_id(8), access_hash: Keys::hash(&keys.access), manage_hash: Keys::hash(&keys.access), expires_at: None, snapshot: sealed(1, 40) };
    assert!(matches!(store.page_create(&alice, same, now), Err(PageError::Invalid(_))));
    let short = CreatePage { id: page_id(8), access_hash: Keys::hash(&keys.access), manage_hash: Keys::hash(&keys.manage), expires_at: None, snapshot: sealed(1, 8) };
    assert!(matches!(store.page_create(&alice, short, now), Err(PageError::Invalid(_))));
    assert!(matches!(create(&mut store, &alice, &page_id(8), &keys, Some(now), now), Err(PageError::Invalid(_))));
    // Another member reads with the access token alone.
    assert_eq!(store.page_info(&bob, &id, &keys.access(), now).unwrap(), info);
    store.page_delete(&alice, &id, &keys.manage(), now).unwrap();
    assert!(matches!(create(&mut store, &alice, &id, &keys, None, now), Err(PageError::Exists)));
}

#[test]
fn append_is_idempotent_by_request_name() {
    let (_dir, mut store, alice, bob, now) = crate::admin::tests::setup();
    let owner = account(&store, &alice, now);
    let (id, keys) = (page_id(7), Keys::new(1));
    create(&mut store, &alice, &id, &keys, None, now).unwrap();
    let first = append(&mut store, &alice, &id, &keys, op(1, 0, "a-1", 2), now).unwrap();
    assert_eq!(first, Appended { seq: 1, missed: vec![] });
    // Bob's concurrent batch on the same base learns what it missed.
    let second = append(&mut store, &bob, &id, &keys, op(1, 0, "b-1", 3), now).unwrap();
    assert_eq!(second.seq, 2);
    assert_eq!(second.missed, vec![Op { seq: 1, base: 0, epoch: 1, created: now, request: name("a-1"), body: sealed(2, 32) }]);
    let charged = retained(&store, &owner);
    // A lost reply retried answers what the first attempt produced, without a second copy.
    assert_eq!(append(&mut store, &bob, &id, &keys, op(1, 0, "b-1", 3), now).unwrap(), second);
    assert_eq!(append(&mut store, &alice, &id, &keys, op(1, 0, "a-1", 2), now).unwrap(), first);
    assert_eq!(retained(&store, &owner), charged);
    assert_eq!(store.page_info(&alice, &id, &keys.access(), now).unwrap().head, 2);
    assert!(matches!(append(&mut store, &alice, &id, &keys, op(1, 0, "a-1", 9), now), Err(PageError::RequestReused)));
    assert!(matches!(append(&mut store, &alice, &id, &keys, op(1, 3, "a-2", 2), now), Err(PageError::BaseAhead)));
    assert!(matches!(append(&mut store, &alice, &id, &keys, op(2, 2, "a-2", 2), now), Err(PageError::WrongEpoch)));
    assert!(matches!(append(&mut store, &alice, &id, &keys, AppendOp { request: "a-2".into(), ..op(1, 2, "a-2", 2) }, now), Err(PageError::Invalid(_))));
    assert!(matches!(append(&mut store, &alice, &id, &keys, AppendOp { request: name("a-2").to_uppercase(), ..op(1, 2, "a-2", 2) }, now), Err(PageError::Invalid(_))));
    let large = AppendOp { epoch: 1, base: 2, request: name("a-2"), body: sealed(1, MAX_OP + 1) };
    assert!(matches!(append(&mut store, &alice, &id, &keys, large, now), Err(PageError::Invalid(_))));
    assert!(matches!(store.page_append(&alice, &id, &keys.access(), op(1, 2, "a-2", 2), now, |_| false), Err(PageError::RateLimited)));
    // Charges are ROW plus ciphertext per op, and a rebuild agrees with them.
    assert_eq!(charged, rebuilt(&mut store, &owner));
    let ops = store.page_ops(&bob, &id, &keys.access(), 0, now).unwrap();
    assert_eq!((ops.epoch, ops.head, ops.ops.len()), (1, 2, 2));
    assert!(store.page_ops(&bob, &id, &keys.access(), 2, now).unwrap().ops.is_empty());
}

#[test]
fn appends_below_the_newest_snapshot_are_refused() {
    let (_dir, mut store, alice, _bob, now) = crate::admin::tests::setup();
    let (id, keys) = (page_id(7), Keys::new(1));
    create(&mut store, &alice, &id, &keys, None, now).unwrap();
    for n in 0..3 {
        append(&mut store, &alice, &id, &keys, op(1, n, &format!("r{n}"), 2), now).unwrap();
    }
    let put = |seq| PutSnapshot { epoch: 1, seq, body: sealed(4, 64) };
    let version = store.page_put_snapshot(&alice, &id, &keys.access(), put(3), now).unwrap().unwrap();
    assert_eq!((version.version, version.seq, version.bytes), (2, 3, 64));
    assert!(matches!(append(&mut store, &alice, &id, &keys, op(1, 2, "late", 2), now), Err(PageError::BaseBeforeSnapshot)));
    assert_eq!(append(&mut store, &alice, &id, &keys, op(1, 3, "late", 2), now).unwrap().seq, 4);
    // Equal or older snapshots are ignored; one beyond the head is refused.
    assert!(store.page_put_snapshot(&alice, &id, &keys.access(), put(3), now).unwrap().is_none());
    assert!(store.page_put_snapshot(&alice, &id, &keys.access(), put(1), now).unwrap().is_none());
    assert!(matches!(store.page_put_snapshot(&alice, &id, &keys.access(), put(9), now), Err(PageError::BaseAhead)));
    let wrong = PutSnapshot { epoch: 2, seq: 4, body: sealed(4, 64) };
    assert!(matches!(store.page_put_snapshot(&alice, &id, &keys.access(), wrong, now), Err(PageError::WrongEpoch)));
    assert_eq!(store.page_info(&alice, &id, &keys.access(), now).unwrap().snapshot_seq, 3);
    assert_eq!(store.page_snapshot(&alice, &id, &keys.access(), None, now).unwrap().body, sealed(4, 64));
    assert_eq!(store.page_snapshot(&alice, &id, &keys.access(), Some(1), now).unwrap().body, sealed(1, 40));
}

#[test]
fn snapshots_that_would_orphan_a_later_op_are_refused() {
    let (_dir, mut store, alice, bob, now) = crate::admin::tests::setup();
    let owner = account(&store, &alice, now);
    let (id, keys) = (page_id(7), Keys::new(1));
    create(&mut store, &alice, &id, &keys, None, now).unwrap();
    append(&mut store, &alice, &id, &keys, op(1, 0, "a", 2), now).unwrap();
    append(&mut store, &bob, &id, &keys, op(1, 0, "b", 3), now).unwrap();
    let before = retained(&store, &owner);
    let put = |seq| PutSnapshot { epoch: 1, seq, body: sealed(4, 64) };
    assert!(matches!(store.page_put_snapshot(&alice, &id, &keys.access(), put(1), now), Err(PageError::Orphans)));
    assert_eq!(retained(&store, &owner), before);
    assert_eq!(store.page_versions(&alice, &id, &keys.access(), now).unwrap().len(), 1);
    assert!(store.page_put_snapshot(&alice, &id, &keys.access(), put(2), now).unwrap().is_some());
}

#[test]
fn rotation_is_compare_and_swap_and_revokes_old_tokens() {
    let (_dir, mut store, alice, bob, now) = crate::admin::tests::setup();
    let (id, keys) = (page_id(7), Keys::new(1));
    create(&mut store, &alice, &id, &keys, None, now).unwrap();
    append(&mut store, &bob, &id, &keys, op(1, 0, "b", 3), now).unwrap();
    let (winner, loser) = (Keys::new(11), Keys::new(21));
    let rotate = |next: &Keys, epoch, seq| Rotate { epoch, seq, access_hash: Keys::hash(&next.access), manage_hash: Keys::hash(&next.manage), snapshot: sealed(5, 50) };
    // Only the manage token rotates.
    assert!(matches!(store.page_rotate(&alice, &id, &keys.access(), rotate(&winner, 1, 1), now), Err(PageError::Denied)));
    assert!(matches!(store.page_rotate(&alice, &id, &keys.manage(), rotate(&winner, 1, 0), now), Err(PageError::HeadMoved)));
    let info = store.page_rotate(&alice, &id, &keys.manage(), rotate(&winner, 1, 1), now).unwrap();
    assert_eq!((info.epoch, info.head, info.snapshot_seq), (2, 1, 1));
    // The racing rotation still names epoch 1 and loses, even with the old manage token.
    assert!(matches!(store.page_rotate(&bob, &id, &keys.manage(), rotate(&loser, 1, 1), now), Err(PageError::Denied)));
    assert!(matches!(store.page_rotate(&bob, &id, &winner.manage(), rotate(&loser, 1, 1), now), Err(PageError::WrongEpoch)));
    assert!(matches!(store.page_ops(&bob, &id, &keys.access(), 0, now), Err(PageError::Denied)));
    assert!(matches!(append(&mut store, &bob, &id, &keys, op(1, 1, "late", 3), now), Err(PageError::Denied)));
    let late = store.page_append(&bob, &id, &winner.access(), op(1, 1, "late", 3), now, |_| true);
    assert!(matches!(late, Err(PageError::WrongEpoch)));
    assert!(matches!(store.page_append(&bob, &id, &winner.access(), op(2, 0, "late", 3), now, |_| true), Err(PageError::BaseBeforeSnapshot)));
    assert_eq!(store.page_append(&bob, &id, &winner.access(), op(2, 1, "late", 3), now, |_| true).unwrap().seq, 2);
    let newest = store.page_snapshot(&bob, &id, &winner.access(), None, now).unwrap();
    assert_eq!((newest.epoch, newest.seq, newest.body), (2, 1, sealed(5, 50)));
}

#[test]
fn tokens_credentials_and_deletion_gate_every_route() {
    let (_dir, mut store, alice, bob, now) = crate::admin::tests::setup();
    let (id, keys) = (page_id(7), Keys::new(1));
    create(&mut store, &alice, &id, &keys, None, now).unwrap();
    let stranger = Keys::new(41);
    assert!(matches!(store.page_info(&bob, &id, &stranger.access(), now), Err(PageError::Denied)));
    assert!(matches!(store.page_info(&bob, &id, &keys.manage(), now), Err(PageError::Denied)));
    assert!(matches!(store.page_info(&bob, &page_id(9), &keys.access(), now), Err(PageError::NotFound)));
    assert!(matches!(store.page_info(&"00".repeat(32), &id, &keys.access(), now), Err(PageError::Store(StoreError::Unauthorized))));
    assert!(matches!(store.page_delete(&bob, &id, &keys.access(), now), Err(PageError::Denied)));
    let presence = Presence { slot: sealed(9, 16), body: sealed(3, 40) };
    store.page_put_presence(&bob, &id, &keys.access(), presence, now).unwrap();
    // Either manager may delete; access ends at once and a repeat is harmless.
    store.page_delete(&bob, &id, &keys.manage(), now).unwrap();
    store.page_delete(&alice, &id, &keys.manage(), now).unwrap();
    assert!(matches!(store.page_delete(&alice, &id, &stranger.manage(), now), Err(PageError::Denied)));
    assert!(matches!(store.page_info(&bob, &id, &keys.access(), now), Err(PageError::Removed)));
    assert!(matches!(append(&mut store, &bob, &id, &keys, op(1, 0, "x", 2), now), Err(PageError::Removed)));
    assert!(matches!(store.page_snapshot(&bob, &id, &keys.access(), None, now), Err(PageError::Removed)));
    assert!(matches!(store.page_presence(&bob, &id, &keys.access(), now), Err(PageError::Removed)));
    let rows: i64 = store.0.query_row("SELECT count(*) FROM page_presence", [], |r| r.get(0)).unwrap();
    assert_eq!(rows, 0);
    // A revoked device credential fails before any token is checked.
    let (id, keys) = (page_id(8), Keys::new(1));
    create(&mut store, &alice, &id, &keys, None, now).unwrap();
    store.0.execute("UPDATE devices SET revoked=1 WHERE account_id=(SELECT account_id FROM devices WHERE token_hash=?1)", [crate::auth::digest(&bob).as_slice()]).unwrap();
    assert!(matches!(store.page_info(&bob, &id, &keys.access(), now), Err(PageError::Store(StoreError::Unauthorized))));
}

#[test]
fn owner_quota_bounds_appends_and_snapshots_without_deleting_history() {
    let (_dir, mut store, alice, bob, now) = crate::admin::tests::setup();
    let owner = account(&store, &alice, now);
    let (id, keys) = (page_id(7), Keys::new(1));
    create(&mut store, &alice, &id, &keys, None, now).unwrap();
    let used = crate::recovery::used(&store.0, &owner, now).unwrap();
    // Room for one more op row; Bob's edits are charged to the owner.
    store.0.execute("INSERT INTO account_policy(account,quota) VALUES(?1,?2)", (&owner, (used + ROW + 32) as i64)).unwrap();
    append(&mut store, &bob, &id, &keys, op(1, 0, "b1", 2), now).unwrap();
    let full = retained(&store, &owner);
    assert!(matches!(append(&mut store, &bob, &id, &keys, op(1, 1, "b2", 2), now), Err(PageError::Quota)));
    let put = PutSnapshot { epoch: 1, seq: 1, body: sealed(4, 64) };
    assert!(matches!(store.page_put_snapshot(&alice, &id, &keys.access(), put, now), Err(PageError::Quota)));
    assert_eq!(retained(&store, &owner), full);
    assert_eq!(store.page_ops(&bob, &id, &keys.access(), 0, now).unwrap().ops.len(), 1);
    let bob_account = account(&store, &bob, now);
    assert_eq!(rebuilt(&mut store, &bob_account), retained(&store, &bob_account));
}

#[test]
fn live_page_count_is_bounded_per_account() {
    let (_dir, mut store, alice, _bob, now) = crate::admin::tests::setup();
    let owner = account(&store, &alice, now);
    store.0.execute("INSERT INTO account_policy(account,quota) VALUES(?1,?2)", (&owner, i64::MAX)).unwrap();
    let insert = "INSERT INTO pages SELECT printf('%064x',value),?1,x'00',x'01',1,0,0,0,0,NULL,0 FROM generate_series(1,?2)";
    let generated = store.0.execute(insert, (&owner, MAX_LIVE));
    if generated.is_err() {
        // Without generate_series, fill through a recursive query.
        store.0.execute("WITH RECURSIVE n(v) AS (SELECT 1 UNION ALL SELECT v+1 FROM n WHERE v<?2) INSERT INTO pages SELECT printf('%064x',v),?1,x'00',x'01',1,0,0,0,0,NULL,0 FROM n", (&owner, MAX_LIVE)).unwrap();
    }
    assert!(matches!(create(&mut store, &alice, &page_id(0xee), &Keys::new(1), None, now), Err(PageError::PageLimit)));
    store.0.execute("UPDATE pages SET deleted=1 WHERE id=printf('%064x',1)", []).unwrap();
    create(&mut store, &alice, &page_id(0xee), &Keys::new(1), None, now).unwrap();
}

#[test]
fn compaction_drops_only_ops_long_behind_the_newest_snapshot() {
    let (_dir, mut store, alice, _bob, now) = crate::admin::tests::setup();
    let owner = account(&store, &alice, now);
    let (id, keys) = (page_id(7), Keys::new(1));
    let old = now - 40 * DAY;
    create(&mut store, &alice, &id, &keys, None, old).unwrap();
    for n in 0..4 {
        append(&mut store, &alice, &id, &keys, op(1, n, &format!("old{n}"), 2), old).unwrap();
    }
    append(&mut store, &alice, &id, &keys, op(1, 4, "recent", 2), now - DAY).unwrap();
    let before = retained(&store, &owner);
    // Without a snapshot covering them, old ops stay.
    cleanup_at(&mut store, now);
    assert_eq!(store.page_ops(&alice, &id, &keys.access(), 0, now).unwrap().ops.len(), 5);
    store.page_put_snapshot(&alice, &id, &keys.access(), PutSnapshot { epoch: 1, seq: 5, body: sealed(4, 64) }, now).unwrap().unwrap();
    let charged = retained(&store, &owner);
    assert_eq!(charged, before + ROW + 64);
    assert_eq!(cleanup_at(&mut store, now), 4);
    assert_eq!(retained(&store, &owner), charged - 4 * (ROW + 32));
    assert_eq!(retained(&store, &owner), rebuilt(&mut store, &owner));
    assert!(matches!(store.page_ops(&alice, &id, &keys.access(), 0, now), Err(PageError::Compacted)));
    let remaining = store.page_ops(&alice, &id, &keys.access(), 4, now).unwrap();
    assert_eq!(remaining.ops.iter().map(|o| o.seq).collect::<Vec<_>>(), vec![5]);
    assert_eq!(cleanup_at(&mut store, now), 0);
    assert_eq!(store.page_info(&alice, &id, &keys.access(), now).unwrap().bytes, 40 + 32 + 64);
}

#[test]
fn versions_keep_the_newest_fifty_and_every_labelled_one() {
    let (_dir, mut store, alice, _bob, now) = crate::admin::tests::setup();
    let owner = account(&store, &alice, now);
    let (id, keys) = (page_id(7), Keys::new(1));
    create(&mut store, &alice, &id, &keys, None, now).unwrap();
    store.page_label(&alice, &id, &keys.access(), 1, Label { label: sealed(8, 30) }, now).unwrap();
    assert!(matches!(store.page_label(&alice, &id, &keys.access(), 99, Label { label: sealed(8, 30) }, now), Err(PageError::NotFound)));
    for n in 0..60 {
        append(&mut store, &alice, &id, &keys, op(1, n, &format!("r{n}"), 2), now).unwrap();
        store.page_put_snapshot(&alice, &id, &keys.access(), PutSnapshot { epoch: 1, seq: n + 1, body: sealed(4, 64) }, now).unwrap().unwrap();
    }
    let versions = store.page_versions(&alice, &id, &keys.access(), now).unwrap();
    assert_eq!(versions.len(), 51);
    assert_eq!(versions[0].version, 61);
    assert_eq!(versions[49].version, 12);
    assert_eq!((versions[50].version, versions[50].label.clone()), (1, sealed(8, 30)));
    assert_eq!(retained(&store, &owner), rebuilt(&mut store, &owner));
    // Unnaming the old version lets it go.
    store.page_label(&alice, &id, &keys.access(), 1, Label { label: String::new() }, now).unwrap();
    assert_eq!(store.page_versions(&alice, &id, &keys.access(), now).unwrap().len(), 50);
    assert_eq!(retained(&store, &owner), rebuilt(&mut store, &owner));
}

#[test]
fn presence_is_sealed_slotted_and_short_lived() {
    let (_dir, mut store, alice, bob, now) = crate::admin::tests::setup();
    let (id, keys) = (page_id(7), Keys::new(1));
    create(&mut store, &alice, &id, &keys, None, now).unwrap();
    let put = |slot: u8, body: u8| Presence { slot: sealed(slot, 16), body: sealed(body, 40) };
    store.page_put_presence(&alice, &id, &keys.access(), put(1, 2), now).unwrap();
    store.page_put_presence(&bob, &id, &keys.access(), put(2, 3), now - 20).unwrap();
    store.page_put_presence(&alice, &id, &keys.access(), put(1, 4), now).unwrap();
    let seen = store.page_presence(&bob, &id, &keys.access(), now).unwrap();
    assert_eq!(seen, vec![PresenceEntry { slot: sealed(1, 16), body: sealed(4, 40), updated: now }, PresenceEntry { slot: sealed(2, 16), body: sealed(3, 40), updated: now - 20 }]);
    assert_eq!(store.page_presence(&bob, &id, &keys.access(), now + 15).unwrap().len(), 1);
    assert!(matches!(store.page_put_presence(&alice, &id, &keys.access(), Presence { slot: sealed(1, 8), body: sealed(4, 40) }, now), Err(PageError::Invalid(_))));
    assert!(matches!(store.page_put_presence(&alice, &id, &keys.access(), Presence { slot: sealed(1, 16), body: sealed(4, MAX_PRESENCE + 1) }, now), Err(PageError::Invalid(_))));
    for slot in 3..(MAX_SLOTS as u8 + 1) {
        store.page_put_presence(&alice, &id, &keys.access(), put(slot, 2), now).unwrap();
    }
    assert!(matches!(store.page_put_presence(&alice, &id, &keys.access(), put(200, 2), now), Err(PageError::Busy)));
    // Stale slots free room and maintenance clears them.
    store.page_put_presence(&alice, &id, &keys.access(), put(200, 2), now + 31).unwrap();
    assert_eq!(cleanup_at(&mut store, now + 62), 1);
    let rows: i64 = store.0.query_row("SELECT count(*) FROM page_presence", [], |r| r.get(0)).unwrap();
    assert_eq!(rows, 0);
}

#[test]
fn expiry_deletion_and_account_removal_purge_in_bounded_batches() {
    let (_dir, mut store, alice, _bob, now) = crate::admin::tests::setup();
    let owner = account(&store, &alice, now);
    let before = retained(&store, &owner);
    let (id, keys) = (page_id(7), Keys::new(1));
    create(&mut store, &alice, &id, &keys, Some(now + 60), now).unwrap();
    for n in 0..600 {
        append(&mut store, &alice, &id, &keys, op(1, n, &format!("r{n}"), 2), now).unwrap();
    }
    assert!(matches!(store.page_info(&alice, &id, &keys.access(), now + 60), Err(PageError::Removed)));
    // The first batch marks and purges at most one bounded slice; the next finishes.
    assert!(cleanup_at(&mut store, now + 60) >= 512);
    let left: i64 = store.0.query_row("SELECT count(*) FROM page_ops", [], |r| r.get(0)).unwrap();
    assert_eq!(left, 600 - 512);
    cleanup_at(&mut store, now + 61);
    let (ops, snapshots, bytes, deleted): (i64, i64, i64, bool) = store.0.query_row("SELECT (SELECT count(*) FROM page_ops),(SELECT count(*) FROM page_snapshots),bytes,deleted FROM pages", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap();
    assert_eq!((ops, snapshots, bytes, deleted), (0, 0, 0, true));
    // Only the tombstone stays charged.
    assert_eq!(retained(&store, &owner), before + PAGE);
    assert_eq!(retained(&store, &owner), rebuilt(&mut store, &owner));
    assert_eq!(cleanup_at(&mut store, now + 62), 0);
    let second = page_id(8);
    create(&mut store, &alice, &second, &keys, None, now).unwrap();
    let tx = store.0.transaction().unwrap();
    delete_account(&tx, &owner).unwrap();
    tx.commit().unwrap();
    let live: i64 = store.0.query_row("SELECT count(*) FROM pages WHERE deleted=0", [], |r| r.get(0)).unwrap();
    assert_eq!(live, 0);
}

#[test]
fn append_buckets_bound_each_page_and_device() {
    let runtime = crate::page_routes::Runtime::default();
    let start = std::time::Instant::now();
    assert!((0..40).all(|_| runtime.admit("p", "d", start)));
    assert!(!runtime.admit("p", "d", start));
    assert!(runtime.admit("p", "other", start));
    assert!(runtime.admit("q", "d", start));
    assert!(runtime.admit("p", "d", start + std::time::Duration::from_millis(100)));
}

#[test]
fn schema_forty_migrates_and_keeps_existing_data() {
    let (dir, store, alice, _bob, now) = crate::admin::tests::setup();
    let prior = store.session(&alice, now).unwrap();
    store.0.execute_batch("DROP TABLE page_presence; DROP TABLE page_snapshots; DROP TABLE page_ops; DROP TABLE pages; PRAGMA user_version=40;").unwrap();
    drop(store);
    let mut store = Store::open(&dir.path().join("sigil.db")).unwrap();
    assert_eq!(store.session(&alice, now).unwrap(), prior);
    let version: i64 = store.0.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(version, crate::store::SCHEMA_VERSION);
    let (id, keys) = (page_id(7), Keys::new(1));
    create(&mut store, &alice, &id, &keys, None, now).unwrap();
    drop(store);
    // Reopening at the current version keeps pages.
    let mut store = Store::open(&dir.path().join("sigil.db")).unwrap();
    assert_eq!(store.page_info(&alice, &id, &keys.access(), now).unwrap().epoch, 1);
}

fn request(method: &str, path: &str, credential: &str, headers: &[(&str, &str)], body: Option<serde_json::Value>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(path).header("authorization", format!("Bearer {credential}"));
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    match body {
        Some(v) => builder.header("content-type", "application/json").body(Body::from(v.to_string())).unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}
async fn json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

#[tokio::test]
async fn routes_carry_tokens_in_headers_and_long_polls_wake_on_append() {
    let (dir, store, alice, bob, _now) = crate::admin::tests::setup();
    let token = crate::auth::AdminToken::load_or_create(&dir.path().join("admin.token")).unwrap();
    let app = crate::router(store, token);
    let (id, keys) = (page_id(7), Keys::new(1));
    let base = format!("/client/v0/pages/{id}");
    let access = [(crate::page_routes::ACCESS, keys.access.as_str())];
    let manage = [(crate::page_routes::MANAGE, keys.manage.as_str())];
    // The largest sealed snapshot fits the body limit.
    let create = serde_json::json!({"id": id, "access_hash": Keys::hash(&keys.access), "manage_hash": Keys::hash(&keys.manage), "snapshot": sealed(1, MAX_SNAPSHOT)});
    let response = app.clone().oneshot(request("POST", "/client/v0/pages", &alice, &[], Some(create))).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json(response).await["bytes"], MAX_SNAPSHOT as u64);
    let response = app.clone().oneshot(request("GET", &base, &bob, &[], None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = app.clone().oneshot(request("GET", &base, &bob, &manage, None)).await.unwrap();
    assert_eq!(json(response).await["code"], "page_access_denied");
    let response = app.clone().oneshot(request("GET", &base, &bob, &access, None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    // A waiter holds until Bob's append lands, past the ordinary 5 s deadline when needed.
    let waiter = tokio::spawn(app.clone().oneshot(request("GET", &format!("{base}/ops?after=0&wait=20"), &alice, &access, None)));
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(!waiter.is_finished());
    let append = serde_json::json!({"epoch": 1, "base": 0, "request": name("b-1"), "body": sealed(3, 32)});
    let response = app.clone().oneshot(request("POST", &format!("{base}/ops"), &bob, &access, Some(append.clone()))).await.unwrap();
    assert_eq!(json(response).await, serde_json::json!({"seq": 1, "missed": []}));
    let woken = tokio::time::timeout(std::time::Duration::from_secs(5), waiter).await.unwrap().unwrap().unwrap();
    assert_eq!(woken.status(), StatusCode::OK);
    assert_eq!(json(woken).await["ops"][0]["body"], sealed(3, 32));
    let started = std::time::Instant::now();
    let response = app.clone().oneshot(request("GET", &format!("{base}/ops?after=1&wait=6"), &alice, &access, None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(started.elapsed() >= std::time::Duration::from_secs(6));
    assert_eq!(json(response).await["ops"], serde_json::json!([]));
    let response = app.clone().oneshot(request("GET", &format!("{base}/ops?after=0&wait=26"), &alice, &access, None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    // An oversized op is refused before it reaches storage.
    let large = serde_json::json!({"epoch": 1, "base": 1, "request": name("b-2"), "body": sealed(3, MAX_OP * 2)});
    let response = app.clone().oneshot(request("POST", &format!("{base}/ops"), &bob, &access, Some(large))).await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let response = app.clone().oneshot(request("PUT", &format!("{base}/snapshot"), &bob, &access, Some(serde_json::json!({"epoch": 1, "seq": 0, "body": sealed(2, 40)})))).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = app.clone().oneshot(request("DELETE", &base, &alice, &access, None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = app.clone().oneshot(request("DELETE", &base, &alice, &manage, None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = app.clone().oneshot(request("POST", &format!("{base}/ops"), &bob, &access, Some(append))).await.unwrap();
    assert_eq!(response.status(), StatusCode::GONE);
}
