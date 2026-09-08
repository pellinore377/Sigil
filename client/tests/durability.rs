use sigil_client::{ClientStore, Error};
mod common;
use sigil_crypto::{
    storage::StorageKey,
    triple::{Packet, Session},
    DhKey, Secret32,
};
use std::path::Path;
fn private_dir() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}
const SESSION: [u8; 32] = [1; 32];
fn key() -> StorageKey {
    StorageKey::new(Secret32::from_bytes([9; 32])).unwrap()
}
fn open(path: &Path) -> ClientStore {
    ClientStore::open(path, key()).unwrap()
}
fn pair() -> (Session, Session) {
    let dh = DhKey::generate().unwrap();
    (
        Session::initiator(Secret32::from_bytes([7; 32]), dh.public_key(), [8; 32]).unwrap(),
        Session::responder(Secret32::from_bytes([7; 32]), dh, [8; 32]).unwrap(),
    )
}

#[test]
fn superseded_ratchet_checkpoint_leaves_no_live_file_copy() {
    fn remains(path: &Path, old: &[u8]) -> bool {
        let file = std::fs::read(path).unwrap();
        old.as_chunks::<64>()
            .0
            .iter()
            .skip(1)
            .any(|chunk| file.windows(64).any(|v| v == chunk))
    }
    let dir = private_dir();
    let path = dir.path().join("erasure.db");
    let (alice, mut bob) = pair();
    let mut store = open(&path);
    store.insert_session(SESSION, alice).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    let old: Vec<u8> = db
        .query_row("SELECT state FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert!(remains(&path, &old));
    let wire = store.send(SESSION, [3; 32], b"synthetic erasure").unwrap();
    assert_eq!(
        bob.receive(&Packet::from_bytes(&wire).unwrap()).unwrap(),
        b"synthetic erasure"
    );
    assert!(!remains(&path, &old));
    assert!(!path.with_extension("db-wal").exists());
    assert!(!path.with_extension("db-journal").exists());
    // Legacy WAL/free pages are purged before the migration is considered complete.
    drop(store);
    db.execute_batch("UPDATE outbox SET rowid=9001;").unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA secure_delete=OFF; CREATE TABLE retired_synthetic(id BLOB PRIMARY KEY, content BLOB); INSERT INTO retired_synthetic VALUES(X'01',randomblob(8000));").unwrap();
    let old: Vec<u8> = db
        .query_row("SELECT content FROM retired_synthetic", [], |r| r.get(0))
        .unwrap();
    db.execute_batch("DELETE FROM retired_synthetic; PRAGMA wal_checkpoint(TRUNCATE); UPDATE storage_cleanup SET pending=1; PRAGMA user_version=67;").unwrap();
    drop(db);
    assert!(remains(&path, &old));
    let store = open(&path);
    assert!(!remains(&path, &old));
    assert_eq!(store.pending(SESSION).unwrap().len(), 1);
    let db = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("SELECT rowid FROM outbox", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        9001
    );
    drop(store);
    db.execute_batch("UPDATE storage_cleanup SET pending=1; CREATE TRIGGER fail_cleanup BEFORE UPDATE ON storage_cleanup BEGIN SELECT RAISE(FAIL,'injected'); END;").unwrap();
    assert!(ClientStore::open(&path, key()).is_err());
    assert_eq!(
        db.query_row("SELECT pending FROM storage_cleanup", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    db.execute_batch("DROP TRIGGER fail_cleanup;").unwrap();
    drop(open(&path));
    assert_eq!(
        db.query_row("SELECT pending FROM storage_cleanup", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn storage_exhaustion_rolls_back_send_and_larger_budget_resumes_same_database() {
    let dir = private_dir();
    let path = dir.path().join("budget.db");
    let (alice, mut bob) = pair();
    drop(open(&path));
    let baseline: u64 = {
        let db = rusqlite::Connection::open(&path).unwrap();
        let pages: u32 = db
            .pragma_query_value(None, "page_count", |r| r.get(0))
            .unwrap();
        let page_size: u32 = db
            .pragma_query_value(None, "page_size", |r| r.get(0))
            .unwrap();
        u64::from(pages) * u64::from(page_size)
    };
    let budget = baseline + 256 * 1024;
    let mut store = ClientStore::open_with_storage_limit(&path, key(), budget).unwrap();
    store.insert_session(SESSION, alice).unwrap();
    let body = vec![42; 60000];
    let failed = (1u32..100)
        .find(|n| {
            let mut id = [0; 32];
            id[..4].copy_from_slice(&n.to_be_bytes());
            match store.send(SESSION, id, &body) {
                Ok(packet) => {
                    assert_eq!(
                        bob.receive(&Packet::from_bytes(&packet).unwrap())
                            .unwrap()
                            .as_slice(),
                        body
                    );
                    false
                }
                Err(Error::Storage(rusqlite::Error::SqliteFailure(error, _)))
                    if error.code == rusqlite::ErrorCode::DiskFull =>
                {
                    true
                }
                other => panic!("unexpected send result: {other:?}"),
            }
        })
        .expect("database must enforce its page budget");
    drop(store);
    let mut store =
        ClientStore::open_with_storage_limit(&path, key(), budget + 1024 * 1024).unwrap();
    let mut id = [0; 32];
    id[..4].copy_from_slice(&failed.to_be_bytes());
    let packet = store.send(SESSION, id, &body).unwrap();
    assert_eq!(
        bob.receive(&Packet::from_bytes(&packet).unwrap())
            .unwrap()
            .as_slice(),
        body
    );
    assert_eq!(store.send(SESSION, id, &body).unwrap(), packet);
    drop(store);
    assert!(matches!(
        ClientStore::open_with_storage_limit(&path, key(), budget),
        Err(Error::Limit)
    ));
}

#[test]
fn executable_schema_is_rejected_before_reopening_live_state() {
    let dir = private_dir();
    let path = dir.path().join("schema.db");
    let (alice, _) = pair();
    let mut store = open(&path);
    store.insert_session(SESSION, alice).unwrap();
    let packet = store.send(SESSION, [2; 32], b"synthetic retained").unwrap();
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    let before: Vec<u8> = db
        .query_row("SELECT state FROM sessions", [], |r| r.get(0))
        .unwrap();
    for (create, remove) in [
        ("CREATE TRIGGER suppress_state BEFORE UPDATE ON sessions BEGIN SELECT RAISE(IGNORE); END;", "DROP TRIGGER suppress_state"),
        ("CREATE VIEW exposed_state AS SELECT state FROM sessions", "DROP VIEW exposed_state"),
    ] {
        db.execute_batch(create).unwrap();
        assert!(matches!(ClientStore::open(&path, key()), Err(Error::InvalidStore)));
        assert_eq!(db.query_row("SELECT state FROM sessions", [], |r| r.get::<_, Vec<u8>>(0)).unwrap(), before);
        db.execute_batch(remove).unwrap();
    }
    let mut store = open(&path);
    assert_eq!(
        store.send(SESSION, [2; 32], b"synthetic retained").unwrap(),
        packet
    );
}

#[test]
fn schema_five_classical_sessions_are_retained_without_fallback() {
    let dir = private_dir();
    let path = dir.path().join("legacy.db");
    drop(open(&path));
    let db = rusqlite::Connection::open(&path).unwrap();
    common::rewind(&db, 5);
    let dh = DhKey::generate().unwrap();
    let mut old = sigil_crypto::ratchet::Session::initiator(
        Secret32::from_bytes([7; 32]),
        dh.public_key(),
        [8; 32],
    )
    .unwrap();
    let packet = old.send(b"legacy pending").unwrap().to_bytes();
    let message = [21; 32];
    let bind = |purpose: u8, record: &[u8]| {
        [b"Sigil/client/v0".as_slice(), &[purpose], &SESSION, record].concat()
    };
    let state = old
        .seal_checkpoint(&key(), &bind(0, &1_i64.to_be_bytes()))
        .unwrap();
    let queued = key().seal(&packet, &bind(3, &message)).unwrap();
    let tag = key()
        .commitment(b"legacy pending", &bind(1, &message))
        .unwrap();
    db.execute(
        "INSERT INTO sessions VALUES(?1,1,?2)",
        (SESSION.as_slice(), &state),
    )
    .unwrap();
    db.execute(
        "INSERT INTO outbox VALUES(?1,?2,?3,?4)",
        (
            SESSION.as_slice(),
            message.as_slice(),
            tag.as_slice(),
            &queued,
        ),
    )
    .unwrap();
    let content = key().seal(b"retained history", &bind(2, &message)).unwrap();
    db.execute(
        "INSERT INTO inbox VALUES(?1,?2,?3,?4)",
        (
            SESSION.as_slice(),
            message.as_slice(),
            tag.as_slice(),
            content,
        ),
    )
    .unwrap();
    let mut store = open(&path);
    assert!(matches!(
        store.send(SESSION, message, b"legacy pending"),
        Err(Error::UnsupportedSession)
    ));
    assert!(matches!(
        store.send(SESSION, [22; 32], b"new"),
        Err(Error::UnsupportedSession)
    ));
    assert!(matches!(
        store.pending(SESSION),
        Err(Error::UnsupportedSession)
    ));
    assert!(matches!(
        store.pending_deliveries(SESSION, 1000),
        Err(Error::UnsupportedSession)
    ));
    assert!(matches!(
        store.prepare_delivery(SESSION, message, [8; 32], 1060, 1000),
        Err(Error::UnsupportedSession)
    ));
    assert_eq!(
        store.message(SESSION, message).unwrap().as_slice(),
        b"retained history"
    );
    assert_eq!(
        db.query_row("SELECT state FROM sessions", [], |r| r.get::<_, Vec<u8>>(0))
            .unwrap(),
        state
    );
    assert_eq!(
        db.query_row("SELECT packet FROM outbox", [], |r| r.get::<_, Vec<u8>>(0))
            .unwrap(),
        queued
    );
    // Changing the database marker cannot turn a classical checkpoint into a hybrid session.
    db.execute("UPDATE sessions SET suite=2", []).unwrap();
    assert!(matches!(
        store.send(SESSION, [22; 32], b"new"),
        Err(Error::Crypto(sigil_crypto::Error::Encoding))
    ));
}

#[test]
fn storage_failures_at_post_quantum_transitions_roll_back_both_ratchets() {
    let dir = private_dir();
    let alice_path = dir.path().join("alice.db");
    let bob_path = dir.path().join("bob.db");
    let (a, b) = pair();
    let mut alice = open(&alice_path);
    let mut bob = open(&bob_path);
    alice.insert_session(SESSION, a).unwrap();
    bob.insert_session(SESSION, b).unwrap();
    let alice_db = rusqlite::Connection::open(&alice_path).unwrap();
    let bob_db = rusqlite::Connection::open(&bob_path).unwrap();
    let snapshot = |db: &rusqlite::Connection| {
        db.query_row("SELECT revision,state FROM sessions", [], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?))
        })
        .unwrap()
    };
    let mut send_fault = false;
    let mut receive_fault = false;
    for n in 1_u32..=70 {
        let mut id = [0; 32];
        id[..4].copy_from_slice(&n.to_be_bytes());
        let packet = alice.send(SESSION, id, b"alice").unwrap();
        bob.receive(SESSION, id, &packet).unwrap();
        // Header's last systematic fragment: Bob's next send samples a KEM
        // ciphertext and advances the post-quantum root/authenticator.
        if packet[48] == 85 && packet[65] == 1 && packet[66..70] == [0, 0, 0, 1] && !send_fault {
            let before = snapshot(&bob_db);
            bob_db.execute_batch("CREATE TRIGGER reject_outbox BEFORE INSERT ON outbox BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
            assert!(matches!(
                bob.send(SESSION, id, b"bob"),
                Err(Error::Storage(_))
            ));
            assert_eq!(snapshot(&bob_db), before);
            bob_db.execute_batch("DROP TRIGGER reject_outbox;").unwrap();
            send_fault = true;
        }
        let packet = bob.send(SESSION, id, b"bob").unwrap();
        // Last ct2 fragment: Alice's receive decapsulates and advances the PQ epoch.
        if packet[48] == 85 && packet[65] == 6 && packet[66..70] == [0, 0, 0, 2] && !receive_fault {
            for statement in ["CREATE TRIGGER reject_receive BEFORE UPDATE ON sessions BEGIN SELECT RAISE(ABORT,'synthetic'); END;",
                "CREATE TRIGGER reject_receive BEFORE INSERT ON inbox BEGIN SELECT RAISE(ABORT,'synthetic'); END;"] {
                let before = snapshot(&alice_db);
                alice_db.execute_batch(statement).unwrap();
                assert!(matches!(alice.receive(SESSION, id, &packet), Err(Error::Storage(_))));
                assert_eq!(snapshot(&alice_db), before);
                alice_db.execute_batch("DROP TRIGGER reject_receive;").unwrap();
            }
            receive_fault = true;
        }
        assert_eq!(
            alice.receive(SESSION, id, &packet).unwrap().as_slice(),
            b"bob"
        );
        drop(alice);
        drop(bob);
        alice = open(&alice_path);
        bob = open(&bob_path);
    }
    assert!(send_fault && receive_fault);
}

#[test]
fn outgoing_history_survives_acknowledgement_and_binds_direction() {
    let dir = private_dir();
    let path = dir.path().join("history.db");
    let (alice, mut bob) = pair();
    let mut store = open(&path);
    store.insert_session(SESSION, alice).unwrap();
    let id = [45; 32];
    let packet = store.send(SESSION, id, b"outgoing history").unwrap();
    bob.receive(&Packet::from_bytes(&packet).unwrap()).unwrap();
    store
        .prepare_delivery(SESSION, id, [8; 32], 1060, 1000)
        .unwrap();
    store
        .acknowledge_sent(
            SESSION,
            id,
            &sigil_protocol::mailbox::Receipt {
                sequence: 1,
                expires_at: 1060,
            },
        )
        .unwrap();
    drop(store);
    let mut store = open(&path);
    assert!(store.pending(SESSION).unwrap().is_empty());
    assert_eq!(
        store.outgoing_message(SESSION, id).unwrap().as_slice(),
        b"outgoing history"
    );
    store
        .receive(
            SESSION,
            id,
            &bob.send(b"incoming history").unwrap().to_bytes(),
        )
        .unwrap();
    assert_eq!(
        store.message(SESSION, id).unwrap().as_slice(),
        b"incoming history"
    );
    let db = rusqlite::Connection::open(&path).unwrap();
    let outgoing: Vec<u8> = db
        .query_row("SELECT content FROM outbox", [], |r| r.get(0))
        .unwrap();
    assert!(!outgoing
        .windows(b"outgoing history".len())
        .any(|bytes| bytes == b"outgoing history"));
    db.execute("UPDATE outbox SET content=(SELECT content FROM inbox)", [])
        .unwrap();
    assert!(matches!(
        store.outgoing_message(SESSION, id),
        Err(Error::Crypto(sigil_crypto::Error::Authentication))
    ));
    db.execute("UPDATE outbox SET content=?1", [&outgoing])
        .unwrap();
    assert_eq!(
        store.outgoing_message(SESSION, id).unwrap().as_slice(),
        b"outgoing history"
    );
}

#[test]
fn schema_six_does_not_invent_missing_outgoing_history() {
    let dir = private_dir();
    let path = dir.path().join("history.db");
    let (alice, _) = pair();
    let mut store = open(&path);
    store.insert_session(SESSION, alice).unwrap();
    let id = [46; 32];
    let packet = store.send(SESSION, id, b"earlier schema").unwrap();
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    common::rewind(&db, 6);
    let mut store = open(&path);
    assert!(matches!(
        store.outgoing_message(SESSION, id),
        Err(Error::NotFound)
    ));
    assert_eq!(store.send(SESSION, id, b"earlier schema").unwrap(), packet);
    assert!(matches!(
        store.outgoing_message(SESSION, id),
        Err(Error::NotFound)
    ));
    store.send(SESSION, [47; 32], b"newly retained").unwrap();
    assert_eq!(
        store
            .outgoing_message(SESSION, [47; 32])
            .unwrap()
            .as_slice(),
        b"newly retained"
    );
}
fn revision(path: &Path) -> i64 {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row("SELECT revision FROM sessions", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn restart_retries_and_acknowledgement_never_reencrypt() {
    let dir = private_dir();
    let path = dir.path().join("client.db");
    let (alice, mut bob) = pair();
    let mut store = open(&path);
    store.insert_session(SESSION, alice).unwrap();
    let packet = store
        .send(SESSION, [2; 32], b"synthetic durable message")
        .unwrap();
    drop(store);
    let mut store = open(&path);
    assert_eq!(
        store.pending(SESSION).unwrap(),
        vec![([2; 32], packet.clone())]
    );
    assert_eq!(
        store
            .send(SESSION, [2; 32], b"synthetic durable message")
            .unwrap(),
        packet
    );
    assert!(matches!(
        store.send(SESSION, [2; 32], b"different"),
        Err(Error::Conflict)
    ));
    assert_eq!(revision(&path), 1);
    assert_eq!(
        bob.receive(&Packet::from_bytes(&packet).unwrap()).unwrap(),
        b"synthetic durable message"
    );
    store
        .prepare_delivery(SESSION, [2; 32], [8; 32], 1060, 1000)
        .unwrap();
    let receipt = sigil_protocol::mailbox::Receipt {
        sequence: 1,
        expires_at: 1060,
    };
    store.acknowledge_sent(SESSION, [2; 32], &receipt).unwrap();
    store.acknowledge_sent(SESSION, [2; 32], &receipt).unwrap();
    drop(store);
    let mut store = open(&path);
    assert!(store.pending(SESSION).unwrap().is_empty());
    assert!(matches!(
        store.send(SESSION, [2; 32], b"synthetic durable message"),
        Err(Error::AlreadyDelivered)
    ));
    let next = store.send(SESSION, [3; 32], b"next").unwrap();
    assert_eq!(
        bob.receive(&Packet::from_bytes(&next).unwrap()).unwrap(),
        b"next"
    );
}

#[test]
fn receive_commit_and_skipped_keys_survive_restart() {
    let dir = private_dir();
    let path = dir.path().join("client.db");
    let (mut alice, bob) = pair();
    let first = alice.send(b"delayed").unwrap().to_bytes();
    let second = alice.send(b"latest").unwrap().to_bytes();
    let mut store = open(&path);
    assert!(matches!(
        store.message(SESSION, [2; 32]),
        Err(Error::NotFound)
    ));
    store.insert_session(SESSION, bob).unwrap();
    assert_eq!(
        store.receive(SESSION, [2; 32], &second).unwrap().as_slice(),
        b"latest"
    );
    drop(store);
    let mut store = open(&path);
    assert_eq!(
        store.receive(SESSION, [2; 32], &second).unwrap().as_slice(),
        b"latest"
    );
    assert_eq!(revision(&path), 1);
    assert_eq!(
        store.receive(SESSION, [3; 32], &first).unwrap().as_slice(),
        b"delayed"
    );
    assert_eq!(
        store.message(SESSION, [2; 32]).unwrap().as_slice(),
        b"latest"
    );
    assert!(store.receive(SESSION, [4; 32], &first).is_err());
    let reply = store.send(SESSION, [5; 32], b"reply").unwrap();
    assert_eq!(
        alice.receive(&Packet::from_bytes(&reply).unwrap()).unwrap(),
        b"reply"
    );
    let mut corrupt = second.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(matches!(
        store.receive(SESSION, [2; 32], &corrupt),
        Err(Error::Conflict)
    ));
}

#[test]
fn failed_commits_and_authentication_leave_no_partial_state() {
    let dir = private_dir();
    let path = dir.path().join("client.db");
    let (mut alice, bob) = pair();
    let mut store = open(&path);
    store.insert_session(SESSION, bob).unwrap();
    let packet = alice.send(b"synthetic atomic inbox").unwrap().to_bytes();
    let mut corrupt = packet.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(store.receive(SESSION, [2; 32], &corrupt).is_err());
    assert_eq!(revision(&path), 0);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_inbox BEFORE INSERT ON inbox BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(store.receive(SESSION, [2; 32], &packet).is_err());
    assert_eq!(revision(&path), 0);
    db.execute_batch("DROP TRIGGER fail_inbox;").unwrap();
    store.receive(SESSION, [2; 32], &packet).unwrap();
    db.execute_batch("CREATE TRIGGER fail_outbox BEFORE INSERT ON outbox BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(store.send(SESSION, [3; 32], b"reply").is_err());
    assert_eq!(revision(&path), 1);
    assert!(store.pending(SESSION).unwrap().is_empty());
    db.execute_batch("DROP TRIGGER fail_outbox;").unwrap();
    let reply = store.send(SESSION, [3; 32], b"reply").unwrap();
    assert_eq!(
        alice.receive(&Packet::from_bytes(&reply).unwrap()).unwrap(),
        b"reply"
    );
}

#[test]
fn concurrent_writers_share_one_committed_send_and_receive() {
    let dir = private_dir();
    let path = dir.path().join("client.db");
    let (alice, mut bob) = pair();
    let mut store = open(&path);
    store.insert_session(SESSION, alice).unwrap();
    let barrier = std::sync::Barrier::new(2);
    let send = || {
        let mut store = open(&path);
        barrier.wait();
        store.send(SESSION, [2; 32], b"concurrent").unwrap()
    };
    let sent = std::thread::scope(|scope| {
        let a = scope.spawn(send);
        let b = scope.spawn(send);
        [a.join().unwrap(), b.join().unwrap()]
    });
    assert_eq!(sent[0], sent[1]);
    assert_eq!(revision(&path), 1);
    bob.receive(&Packet::from_bytes(&sent[0]).unwrap()).unwrap();
    let reply = bob.send(b"reply").unwrap().to_bytes();
    let receive = || {
        let mut store = open(&path);
        barrier.wait();
        store.receive(SESSION, [3; 32], &reply).unwrap()
    };
    let received = std::thread::scope(|scope| {
        let a = scope.spawn(receive);
        let b = scope.spawn(receive);
        [a.join().unwrap(), b.join().unwrap()]
    });
    assert_eq!(received[0].as_slice(), b"reply");
    assert_eq!(received[0], received[1]);
    assert_eq!(revision(&path), 2);
}

#[test]
fn wrong_keys_tampering_and_record_substitution_fail_closed() {
    let dir = private_dir();
    let path = dir.path().join("client.db");
    let (alice, _) = pair();
    let mut store = open(&path);
    store.insert_session(SESSION, alice).unwrap();
    store
        .send(SESSION, [2; 32], b"private synthetic content")
        .unwrap();
    assert!(ClientStore::open(
        &path,
        StorageKey::new(Secret32::from_bytes([10; 32])).unwrap()
    )
    .is_err());
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("UPDATE outbox SET id=?1", [[3_u8; 32].as_slice()])
        .unwrap();
    assert!(store.pending(SESSION).is_err());
    db.execute("UPDATE sessions SET revision=revision+1", [])
        .unwrap();
    assert!(store.send(SESSION, [4; 32], b"blocked").is_err());
    drop(store);
    drop(db);
    let bytes = std::fs::read(&path).unwrap();
    assert!(!bytes
        .windows(b"private synthetic content".len())
        .any(|v| v == b"private synthetic content"));
}

#[test]
fn limits_and_revision_overflow_fail_before_releasing_packets() {
    let dir = private_dir();
    let path = dir.path().join("client.db");
    let (alice, _) = pair();
    let mut store = open(&path);
    store.insert_session(SESSION, alice).unwrap();
    assert!(store.send(SESSION, [2; 32], &vec![0; 65537]).is_err());
    for n in 0_u32..256 {
        let mut id = [0; 32];
        id[..4].copy_from_slice(&n.to_be_bytes());
        store.send(SESSION, id, b"bounded").unwrap();
    }
    assert_eq!(store.pending(SESSION).unwrap().len(), 16);
    assert!(matches!(
        store.send(SESSION, [3; 32], b"full"),
        Err(Error::Limit)
    ));
    store
        .prepare_delivery(SESSION, [0; 32], [8; 32], 1060, 1000)
        .unwrap();
    store
        .acknowledge_sent(
            SESSION,
            [0; 32],
            &sigil_protocol::mailbox::Receipt {
                sequence: 1,
                expires_at: 1060,
            },
        )
        .unwrap();
    store.send(SESSION, [3; 32], b"freed slot").unwrap();
    assert_eq!(revision(&path), 257);
    let db = rusqlite::Connection::open(&path).unwrap();
    let checkpoint: Vec<u8> = db
        .query_row("SELECT state FROM sessions", [], |r| r.get(0))
        .unwrap();
    let bind = |revision: i64| {
        let mut bytes = b"Sigil/client/v0".to_vec();
        bytes.push(0);
        bytes.extend_from_slice(&SESSION);
        bytes.extend_from_slice(&revision.to_be_bytes());
        bytes
    };
    let state = Session::open_checkpoint(&key(), &checkpoint, &bind(257)).unwrap();
    let sealed = state.seal_checkpoint(&key(), &bind(i64::MAX)).unwrap();
    db.execute(
        "UPDATE sessions SET state=?1,revision=?2",
        (sealed, i64::MAX),
    )
    .unwrap();
    store
        .prepare_delivery(SESSION, [3; 32], [8; 32], 1060, 1000)
        .unwrap();
    store
        .acknowledge_sent(
            SESSION,
            [3; 32],
            &sigil_protocol::mailbox::Receipt {
                sequence: 2,
                expires_at: 1060,
            },
        )
        .unwrap();
    assert!(matches!(
        store.send(SESSION, [4; 32], b"overflow"),
        Err(Error::Limit)
    ));
    assert_eq!(revision(&path), i64::MAX);
}

#[test]
fn maximum_message_roundtrips_through_encrypted_storage() {
    let dir = private_dir();
    let (alice, bob) = pair();
    let mut a = open(&dir.path().join("a.db"));
    let mut b = open(&dir.path().join("b.db"));
    a.insert_session(SESSION, alice).unwrap();
    b.insert_session(SESSION, bob).unwrap();
    let text = vec![42; 65536];
    let packet = a.send(SESSION, [2; 32], &text).unwrap();
    assert_eq!(a.pending(SESSION).unwrap()[0].1, packet);
    assert_eq!(
        b.receive(SESSION, [2; 32], &packet).unwrap().as_slice(),
        text
    );
}

#[test]
#[ignore = "invoked as a disposable child by abrupt_exit_preserves_only_committed_work"]
fn process_worker() {
    let path = std::env::var_os("SIGIL_SYNTHETIC_CLIENT_PATH").unwrap();
    let path = Path::new(&path);
    let mut store = open(path);
    store
        .send(SESSION, [2; 32], b"committed before abrupt exit")
        .unwrap();
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute_batch(
        "BEGIN IMMEDIATE; UPDATE sessions SET state=zeroblob(100); DELETE FROM outbox;",
    )
    .unwrap();
    std::process::exit(0); // Deliberately bypasses Rust destructors and SQLite close.
}

#[test]
fn abrupt_exit_preserves_only_committed_work() {
    let dir = private_dir();
    let path = dir.path().join("client.db");
    let (alice, mut bob) = pair();
    open(&path).insert_session(SESSION, alice).unwrap();
    assert!(std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "process_worker"])
        .env("SIGIL_SYNTHETIC_CLIENT_PATH", &path)
        .status()
        .unwrap()
        .success());
    let mut store = open(&path);
    let packet = store
        .send(SESSION, [2; 32], b"committed before abrupt exit")
        .unwrap();
    assert_eq!(revision(&path), 1);
    assert_eq!(
        bob.receive(&Packet::from_bytes(&packet).unwrap()).unwrap(),
        b"committed before abrupt exit"
    );
    let next = store.send(SESSION, [3; 32], b"after restart").unwrap();
    assert_eq!(
        bob.receive(&Packet::from_bytes(&next).unwrap()).unwrap(),
        b"after restart"
    );
}

#[test]
fn unsafe_paths_and_unrelated_databases_are_rejected() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let dir = private_dir();
    let path = dir.path().join("client.db");
    let store = open(&path);
    drop(store);
    let link = dir.path().join("linked.db");
    symlink(&path, &link).unwrap();
    assert!(ClientStore::open(&link, key()).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(ClientStore::open(&path, key()).is_err());
    let unrelated = dir.path().join("unrelated.db");
    let db = rusqlite::Connection::open(&unrelated).unwrap();
    db.execute_batch("CREATE TABLE unrelated(value TEXT);")
        .unwrap();
    drop(db);
    std::fs::set_permissions(&unrelated, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(ClientStore::open(&unrelated, key()).is_err());
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(ClientStore::open(&dir.path().join("public.db"), key()).is_err());
    assert!(!dir.path().join("public.db").exists());
}

#[test]
fn explicit_retirement_is_atomic_blocks_live_use_and_preserves_history() {
    let dir = private_dir();
    let path = dir.path().join("retirement.db");
    let (alice, mut bob) = pair();
    let mut store = open(&path);
    store.insert_session(SESSION, alice).unwrap();
    let outgoing = [2; 32];
    let packet = store
        .send(SESSION, outgoing, b"Synthetic outgoing")
        .unwrap();
    bob.receive(&Packet::from_bytes(&packet).unwrap()).unwrap();
    let reply = bob.send(b"Synthetic incoming").unwrap().to_bytes();
    store.receive(SESSION, [3; 32], &reply).unwrap();
    assert!(matches!(
        store.retire_session(SESSION),
        Err(Error::Conflict)
    ));
    assert_eq!(
        store
            .send(SESSION, outgoing, b"Synthetic outgoing")
            .unwrap(),
        packet
    );
    store
        .prepare_delivery(SESSION, outgoing, [8; 32], 1060, 1000)
        .unwrap();
    let receipt = sigil_protocol::mailbox::Receipt {
        sequence: 1,
        expires_at: 1060,
    };
    store.acknowledge_sent(SESSION, outgoing, &receipt).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    let before: Vec<u8> = db
        .query_row(
            "SELECT state FROM sessions WHERE id=?1",
            [SESSION.as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    db.execute_batch("CREATE TRIGGER fail_retire BEFORE UPDATE ON sessions BEGIN SELECT RAISE(FAIL,'injected'); END;").unwrap();
    assert!(store.retire_session(SESSION).is_err());
    assert_eq!(
        db.query_row(
            "SELECT state FROM sessions WHERE id=?1",
            [SESSION.as_slice()],
            |r| r.get::<_, Vec<u8>>(0)
        )
        .unwrap(),
        before
    );
    assert!(store.pending(SESSION).unwrap().is_empty());
    db.execute_batch("DROP TRIGGER fail_retire;").unwrap();
    store.retire_session(SESSION).unwrap();
    drop(store);
    let mut store = open(&path);
    store.retire_session(SESSION).unwrap();
    assert!(matches!(
        store.send(SESSION, [4; 32], b"new"),
        Err(Error::RetiredSession)
    ));
    let delayed = bob.send(b"Delayed new message").unwrap().to_bytes();
    assert!(matches!(
        store.receive(SESSION, [5; 32], &delayed),
        Err(Error::RetiredSession)
    ));
    assert!(matches!(store.pending(SESSION), Err(Error::RetiredSession)));
    assert_eq!(
        store
            .outgoing_message(SESSION, outgoing)
            .unwrap()
            .as_slice(),
        b"Synthetic outgoing"
    );
    assert_eq!(
        store.message(SESSION, [3; 32]).unwrap().as_slice(),
        b"Synthetic incoming"
    );
    assert_eq!(
        store.delivery_receipt(SESSION, outgoing).unwrap(),
        Some(receipt)
    );
    assert!(store.insert_session(SESSION, pair().0).is_err());
    // Flipping the routing flag cannot turn the sealed tombstone into keys.
    db.execute(
        "UPDATE sessions SET retired=0 WHERE id=?1",
        [SESSION.as_slice()],
    )
    .unwrap();
    assert!(store.send(SESSION, [6; 32], b"new").is_err());
    db.execute(
        "UPDATE sessions SET retired=1,revision=revision+1 WHERE id=?1",
        [SESSION.as_slice()],
    )
    .unwrap();
    assert!(store.retire_session(SESSION).is_err());
}
