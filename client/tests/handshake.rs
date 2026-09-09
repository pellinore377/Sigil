use sigil_client::{ClientStore, Error};
mod common;
use sigil_crypto::{handshake::Bundle, storage::StorageKey, triple::Packet, IdentityKey, Secret32};
use std::{os::unix::fs::PermissionsExt, path::Path};
const SLOT: [u8; 32] = [1; 32];
const SESSION: [u8; 32] = [2; 32];
const MESSAGE: [u8; 32] = [3; 32];
fn dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}
fn open(path: &Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
fn live(path: &Path) -> i64 {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row("SELECT count(state) FROM prekeys", [], |r| r.get(0))
        .unwrap()
}

fn initiate(
    identity: &IdentityKey,
    recipient: &[u8; 32],
    bundle: &Bundle,
    text: &[u8],
) -> Result<(sigil_crypto::triple::Session, Vec<u8>), sigil_crypto::Error> {
    let (mut state, initial) =
        sigil_crypto::handshake::initiate_session(identity, recipient, bundle, b"")?;
    let packet =
        sigil_protocol::initial::encode(&initial.to_bytes(), &state.send(text)?.to_bytes())
            .unwrap();
    Ok((state, packet))
}

#[test]
fn persisted_slots_accept_once_and_restore_a_usable_session() {
    for ec in [false, true] {
        let dir = dir();
        let path = dir.path().join("client.db");
        let mut store = open(&path);
        let recipient = store.identity().unwrap();
        let encoded = store.create_prekey(SLOT, ec).unwrap();
        drop(store);
        let mut store = open(&path);
        assert_eq!(store.identity().unwrap(), recipient);
        assert_eq!(store.create_prekey(SLOT, ec).unwrap(), encoded);
        assert!(matches!(
            store.create_prekey(SLOT, !ec),
            Err(Error::Conflict)
        ));
        let alice = IdentityKey::generate().unwrap();
        let bundle = Bundle::from_bytes(&encoded, &recipient).unwrap();
        let (mut sender, initial) =
            initiate(&alice, &recipient, &bundle, b"initial synthetic message").unwrap();
        let packet = initial;
        assert_eq!(
            store
                .accept_initial(SLOT, SESSION, MESSAGE, alice.public_key(), &packet)
                .unwrap()
                .as_slice(),
            b"initial synthetic message"
        );
        assert_eq!(live(&path), 0);
        drop(store);
        let mut store = open(&path);
        assert_eq!(
            store
                .accept_initial(SLOT, SESSION, MESSAGE, alice.public_key(), &packet)
                .unwrap()
                .as_slice(),
            b"initial synthetic message"
        );
        assert_eq!(
            store.message(SESSION, MESSAGE).unwrap().as_slice(),
            b"initial synthetic message"
        );
        assert!(matches!(
            store.create_prekey(SLOT, ec),
            Err(Error::AlreadyDelivered)
        ));
        assert!(store
            .accept_initial(SLOT, [8; 32], MESSAGE, alice.public_key(), &packet)
            .is_err());
        assert!(store
            .accept_initial(SLOT, SESSION, [8; 32], alice.public_key(), &packet)
            .is_err());
        assert!(store
            .accept_initial(SLOT, SESSION, MESSAGE, [8; 32], &packet)
            .is_err());
        assert!(store
            .accept_initial([8; 32], SESSION, MESSAGE, alice.public_key(), &packet)
            .is_err());
        // Bob can reply after only the initial delivery, including after restart.
        let reply = store.send(SESSION, [5; 32], b"reply").unwrap();
        assert_eq!(
            sender
                .receive(&Packet::from_bytes(&reply).unwrap())
                .unwrap(),
            b"reply"
        );
        let next = sender.send(b"ratchet after restart").unwrap().to_bytes();
        assert_eq!(
            store.receive(SESSION, [4; 32], &next).unwrap().as_slice(),
            b"ratchet after restart"
        );
    }
}

#[test]
fn authentication_and_storage_failures_do_not_consume_prekeys() {
    let dir = dir();
    let path = dir.path().join("client.db");
    let mut store = open(&path);
    let recipient = store.identity().unwrap();
    let encoded = store.create_prekey(SLOT, true).unwrap();
    let alice = IdentityKey::generate().unwrap();
    let (_, initial) = initiate(
        &alice,
        &recipient,
        &Bundle::from_bytes(&encoded, &recipient).unwrap(),
        b"atomic",
    )
    .unwrap();
    let packet = initial;
    let mut bad = packet.clone();
    *bad.last_mut().unwrap() ^= 1;
    assert!(store
        .accept_initial(SLOT, SESSION, MESSAGE, alice.public_key(), &bad)
        .is_err());
    assert!(store
        .accept_initial(
            SLOT,
            SESSION,
            MESSAGE,
            IdentityKey::generate().unwrap().public_key(),
            &packet
        )
        .is_err());
    let db = rusqlite::Connection::open(&path).unwrap();
    for trigger in [
        "CREATE TRIGGER fail BEFORE INSERT ON inbox BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;",
        "CREATE TRIGGER fail BEFORE UPDATE ON prekeys BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;",
    ] {
        db.execute_batch(trigger).unwrap();
        assert!(store.accept_initial(SLOT,SESSION,MESSAGE,alice.public_key(),&packet).is_err());
        assert_eq!(live(&path),1);
        assert_eq!(db.query_row("SELECT count(*) FROM sessions",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(db.query_row("SELECT count(*) FROM inbox",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        db.execute_batch("DROP TRIGGER fail;").unwrap();
    }
    store
        .accept_initial(SLOT, SESSION, MESSAGE, alice.public_key(), &packet)
        .unwrap();
}

#[test]
fn initial_confirmation_rejects_substitution_and_accepts_reordering() {
    let dir = dir();
    let path = dir.path().join("client.db");
    let mut bob = open(&path);
    let recipient = bob.identity().unwrap();
    let bundle = Bundle::from_bytes(&bob.create_prekey(SLOT, true).unwrap(), &recipient).unwrap();
    let alice = IdentityKey::generate().unwrap();
    let (mut state, initial) =
        sigil_crypto::handshake::initiate_session(&alice, &recipient, &bundle, b"").unwrap();
    let raw = initial.to_bytes();
    let confirmation = state.send(b"confirmed initial").unwrap().to_bytes();
    let packet = sigil_protocol::initial::encode(&raw, &confirmation).unwrap();
    let (_, other) = initiate(&alice, &recipient, &bundle, b"confirmed initial").unwrap();
    let (_, other_confirmation) = sigil_protocol::initial::decode(&other).unwrap();
    let changed = sigil_protocol::initial::encode(&raw, other_confirmation).unwrap();
    let advanced =
        sigil_protocol::initial::encode(&raw, &state.send(b"later").unwrap().to_bytes()).unwrap();
    for invalid in [&raw, &changed] {
        assert!(bob
            .accept_initial(SLOT, SESSION, MESSAGE, alice.public_key(), invalid)
            .is_err());
        assert_eq!(live(&path), 1);
    }
    for index in packet.len() - confirmation.len()..packet.len() {
        let mut changed = packet.clone();
        changed[index] ^= 1;
        assert!(bob
            .accept_initial(SLOT, SESSION, MESSAGE, alice.public_key(), &changed)
            .is_err());
        assert_eq!(live(&path), 1);
    }
    assert!(matches!(
        bob.message(SESSION, MESSAGE),
        Err(Error::NotFound)
    ));
    assert_eq!(
        bob.accept_initial(SLOT, SESSION, [5; 32], alice.public_key(), &advanced)
            .unwrap()
            .as_slice(),
        b"later"
    );
    bob.accept_initial(SLOT, SESSION, MESSAGE, alice.public_key(), &packet)
        .unwrap();
    let reply = bob.send(SESSION, [4; 32], b"immediate reply").unwrap();
    assert_eq!(
        state.receive(&Packet::from_bytes(&reply).unwrap()).unwrap(),
        b"immediate reply"
    );
}

#[test]
fn legacy_initial_outbox_is_preserved_but_never_retransmitted() {
    let dir = dir();
    let path = dir.path().join("alice.db");
    let mut alice = open(&path);
    let mut bob = open(&dir.path().join("bob.db"));
    let recipient = bob.identity().unwrap();
    let bundle = bob.create_prekey(SLOT, true).unwrap();
    let packet = alice
        .start_initial(SESSION, MESSAGE, recipient, &bundle, b"retained")
        .unwrap();
    alice
        .prepare_delivery(SESSION, MESSAGE, [8; 32], 1060, 1000)
        .unwrap();
    let raw = sigil_protocol::initial::decode(&packet).unwrap().0;
    let key = StorageKey::new(Secret32::from_bytes([9; 32])).unwrap();
    let aad = [b"Sigil/client/v0".as_slice(), &[3], &SESSION, &MESSAGE].concat();
    let sealed = key.seal(raw, &aad).unwrap();
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute("UPDATE outbox SET packet=?1", [&sealed])
        .unwrap();
    assert!(matches!(
        alice.pending(SESSION),
        Err(Error::UnsupportedSession)
    ));
    assert!(matches!(
        alice.pending_deliveries(SESSION, 1000),
        Err(Error::UnsupportedSession)
    ));
    assert!(matches!(
        alice.prepare_delivery(SESSION, MESSAGE, [8; 32], 1060, 1000),
        Err(Error::UnsupportedSession)
    ));
    assert!(matches!(
        alice.start_initial(SESSION, MESSAGE, recipient, &bundle, b"retained"),
        Err(Error::UnsupportedSession)
    ));
    assert_eq!(
        alice.outgoing_message(SESSION, MESSAGE).unwrap().as_slice(),
        b"retained"
    );
    assert!(matches!(
        alice.start_initial(
            [7; 32],
            [7; 32],
            recipient,
            &bundle,
            &vec![0; sigil_protocol::initial::MAX_PLAINTEXT + 1]
        ),
        Err(Error::Limit)
    ));
}

#[test]
fn concurrent_handshakes_cannot_share_a_one_time_slot() {
    let dir = dir();
    let path = dir.path().join("client.db");
    let mut store = open(&path);
    let recipient = store.identity().unwrap();
    let encoded = store.create_prekey(SLOT, false).unwrap();
    let alice = IdentityKey::generate().unwrap();
    let bundle = Bundle::from_bytes(&encoded, &recipient).unwrap();
    let (_, a) = initiate(&alice, &recipient, &bundle, b"first").unwrap();
    let (_, b) = initiate(&alice, &recipient, &bundle, b"second").unwrap();
    let barrier = std::sync::Barrier::new(2);
    let run = |session, packet| {
        let mut store = open(&path);
        barrier.wait();
        store.accept_initial(SLOT, session, MESSAGE, alice.public_key(), packet)
    };
    let results = std::thread::scope(|scope| {
        let a = scope.spawn(|| run(SESSION, &a));
        let b = scope.spawn(|| run([7; 32], &b));
        [a.join().unwrap(), b.join().unwrap()]
    });
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(live(&path), 0);
    assert_eq!(
        rusqlite::Connection::open(&path)
            .unwrap()
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn concurrent_identical_delivery_recovers_the_same_commit() {
    let dir = dir();
    let path = dir.path().join("client.db");
    let mut store = open(&path);
    let recipient = store.identity().unwrap();
    let bundle =
        Bundle::from_bytes(&store.create_prekey(SLOT, false).unwrap(), &recipient).unwrap();
    let alice = IdentityKey::generate().unwrap();
    let (_, initial) = initiate(&alice, &recipient, &bundle, b"same delivery").unwrap();
    let packet = initial;
    let barrier = std::sync::Barrier::new(2);
    let run = || {
        let mut store = open(&path);
        barrier.wait();
        store
            .accept_initial(SLOT, SESSION, MESSAGE, alice.public_key(), &packet)
            .unwrap()
    };
    let results = std::thread::scope(|scope| {
        let a = scope.spawn(run);
        let b = scope.spawn(run);
        [a.join().unwrap(), b.join().unwrap()]
    });
    assert_eq!(results[0], results[1]);
    assert_eq!(live(&path), 0);
}

#[test]
fn migration_and_prekey_limits_preserve_existing_sessions() {
    let dir = dir();
    let path = dir.path().join("client.db");
    let mut store = open(&path);
    let dh = sigil_crypto::DhKey::generate().unwrap();
    let session = sigil_crypto::triple::Session::initiator(
        Secret32::from_bytes([7; 32]),
        dh.public_key(),
        [8; 32],
    )
    .unwrap();
    store.insert_session(SESSION, session).unwrap();
    let packet = store.send(SESSION, MESSAGE, b"pre-migration").unwrap();
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    common::rewind(&db, 1);
    let mut store = open(&path);
    assert!(matches!(
        store.pending(SESSION),
        Err(Error::UnsupportedSession)
    ));
    assert!(matches!(
        store.send(SESSION, MESSAGE, b"pre-migration"),
        Err(Error::UnsupportedSession)
    ));
    let sealed: Vec<u8> = db
        .query_row("SELECT packet FROM outbox", [], |row| row.get(0))
        .unwrap();
    let binding = [b"Sigil/client/v0".as_slice(), &[3], &SESSION, &MESSAGE].concat();
    assert_eq!(
        StorageKey::new(Secret32::from_bytes([9; 32]))
            .unwrap()
            .open(&sealed, &binding)
            .unwrap()
            .as_slice(),
        packet
    );
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        i64::from(sigil_client::DATABASE_VERSION)
    );
    for n in 0_u8..64 {
        store.create_prekey([n; 32], false).unwrap();
    }
    assert!(matches!(
        store.create_prekey([64; 32], false),
        Err(Error::Limit)
    ));
    assert!(store.create_prekey([0; 32], false).is_ok());
    db.execute_batch("UPDATE prekeys SET state=NULL;").unwrap();
    let tx = db.unchecked_transaction().unwrap();
    for n in 64_u32..4096 {
        let mut id = [0; 32];
        id[..4].copy_from_slice(&n.to_be_bytes());
        tx.execute("INSERT INTO prekeys VALUES(?1,NULL)", [id.as_slice()])
            .unwrap();
    }
    tx.commit().unwrap();
    let fresh = store.create_prekey([255; 32], false).unwrap();
    assert_eq!(store.create_prekey([255; 32], false).unwrap(), fresh);
    assert!(matches!(
        store.create_prekey([0; 32], false),
        Err(Error::AlreadyDelivered)
    ));
}

#[test]
fn maximum_initial_message_is_committed_without_truncation() {
    let dir = dir();
    let mut store = open(&dir.path().join("client.db"));
    let recipient = store.identity().unwrap();
    let bundle = Bundle::from_bytes(&store.create_prekey(SLOT, true).unwrap(), &recipient).unwrap();
    let alice = IdentityKey::generate().unwrap();
    let text = vec![42; sigil_protocol::initial::MAX_PLAINTEXT];
    let (_, initial) = initiate(&alice, &recipient, &bundle, &text).unwrap();
    assert_eq!(
        store
            .accept_initial(SLOT, SESSION, MESSAGE, alice.public_key(), &initial)
            .unwrap()
            .as_slice(),
        text
    );
}

#[test]
#[ignore = "invoked by handshake_commit_survives_abrupt_exit"]
fn handshake_worker() {
    let path = std::env::var_os("SIGIL_SYNTHETIC_HANDSHAKE_DB").unwrap();
    let file = std::env::var_os("SIGIL_SYNTHETIC_HANDSHAKE_PACKET").unwrap();
    let bytes = std::fs::read(file).unwrap();
    open(Path::new(&path))
        .accept_initial(
            SLOT,
            SESSION,
            MESSAGE,
            bytes[..32].try_into().unwrap(),
            &bytes[32..],
        )
        .unwrap();
    std::process::exit(0);
}

#[test]
fn handshake_commit_survives_abrupt_exit() {
    let dir = dir();
    let path = dir.path().join("client.db");
    let mut store = open(&path);
    let recipient = store.identity().unwrap();
    let bundle = Bundle::from_bytes(&store.create_prekey(SLOT, true).unwrap(), &recipient).unwrap();
    let alice = IdentityKey::generate().unwrap();
    let (_, initial) = initiate(&alice, &recipient, &bundle, b"durable initial").unwrap();
    let packet = initial;
    let mut input = alice.public_key().to_vec();
    input.extend_from_slice(&packet);
    let file = dir.path().join("synthetic-packet");
    std::fs::write(&file, input).unwrap();
    drop(store);
    assert!(std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "handshake_worker"])
        .env("SIGIL_SYNTHETIC_HANDSHAKE_DB", &path)
        .env("SIGIL_SYNTHETIC_HANDSHAKE_PACKET", &file)
        .status()
        .unwrap()
        .success());
    let mut store = open(&path);
    assert_eq!(live(&path), 0);
    assert_eq!(
        store.message(SESSION, MESSAGE).unwrap().as_slice(),
        b"durable initial"
    );
    assert_eq!(
        store
            .accept_initial(SLOT, SESSION, MESSAGE, alice.public_key(), &packet)
            .unwrap()
            .as_slice(),
        b"durable initial"
    );
    assert!(store
        .accept_initial(SLOT, [8; 32], MESSAGE, alice.public_key(), &packet)
        .is_err());
}

#[test]
fn outgoing_restart_retry_and_ack_preserve_the_initial_session() {
    for ec in [false, true] {
        let dir = dir();
        let path = dir.path().join("alice.db");
        let mut bob = open(&dir.path().join("bob.db"));
        let recipient = bob.identity().unwrap();
        let bundle = bob.create_prekey(SLOT, ec).unwrap();
        let mut alice = open(&path);
        let sender = alice.identity().unwrap();
        let plaintext = vec![
            42;
            if ec {
                sigil_protocol::initial::MAX_PLAINTEXT
            } else {
                17
            }
        ];
        let packet = alice
            .start_initial(SESSION, MESSAGE, recipient, &bundle, &plaintext)
            .unwrap();
        drop(alice);
        let mut alice = open(&path);
        assert_eq!(
            alice.pending(SESSION).unwrap(),
            vec![(MESSAGE, packet.clone())]
        );
        assert_eq!(
            alice.outgoing_message(SESSION, MESSAGE).unwrap().as_slice(),
            plaintext
        );
        assert_eq!(
            alice
                .start_initial(SESSION, MESSAGE, recipient, &bundle, &plaintext)
                .unwrap(),
            packet
        );
        assert!(matches!(
            alice.start_initial(SESSION, MESSAGE, recipient, &bundle, b"changed"),
            Err(Error::Conflict)
        ));
        assert!(alice
            .start_initial(SESSION, MESSAGE, [8; 32], &bundle, &plaintext)
            .is_err());
        assert!(matches!(
            alice.send(SESSION, MESSAGE, &plaintext),
            Err(Error::Conflict)
        ));
        assert!(matches!(
            alice.start_initial([8; 32], [8; 32], recipient, &bundle, &plaintext),
            Err(Error::Conflict)
        ));
        assert_eq!(
            bob.accept_initial(SLOT, SESSION, MESSAGE, sender, &packet)
                .unwrap()
                .as_slice(),
            plaintext
        );
        let next = alice.send(SESSION, [4; 32], b"ratchet").unwrap();
        assert_eq!(
            alice
                .start_initial(SESSION, MESSAGE, recipient, &bundle, &plaintext)
                .unwrap(),
            packet
        );
        bob.receive(SESSION, [4; 32], &next).unwrap();
        let reply = bob.send(SESSION, [5; 32], b"reply").unwrap();
        assert_eq!(
            alice.receive(SESSION, [5; 32], &reply).unwrap().as_slice(),
            b"reply"
        );
        alice
            .prepare_delivery(SESSION, MESSAGE, [8; 32], 1060, 1000)
            .unwrap();
        alice
            .acknowledge_sent(
                SESSION,
                MESSAGE,
                &sigil_protocol::mailbox::Receipt {
                    sequence: 1,
                    expires_at: 1060,
                },
            )
            .unwrap();
        drop(alice);
        let mut alice = open(&path);
        assert!(matches!(
            alice.start_initial(SESSION, MESSAGE, recipient, &bundle, &plaintext),
            Err(Error::AlreadyDelivered)
        ));
        assert!(matches!(
            alice.start_initial([9; 32], [9; 32], recipient, &bundle, &plaintext),
            Err(Error::Conflict)
        ));
        let other = bob.create_prekey([8; 32], ec).unwrap();
        assert!(matches!(
            alice.start_initial(SESSION, [8; 32], recipient, &other, &plaintext),
            Err(Error::Conflict)
        ));
    }
}

#[test]
fn outgoing_storage_failures_roll_back_every_record() {
    let dir = dir();
    let path = dir.path().join("alice.db");
    let mut alice = open(&path);
    let mut bob = open(&dir.path().join("bob.db"));
    let recipient = bob.identity().unwrap();
    let bundle = bob.create_prekey(SLOT, true).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    for trigger in [
        "CREATE TRIGGER fail BEFORE INSERT ON outbox BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;",
        "CREATE TRIGGER fail BEFORE INSERT ON initiations BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;",
    ] {
        db.execute_batch(trigger).unwrap();
        assert!(alice.start_initial(SESSION,MESSAGE,recipient,&bundle,b"atomic initial").is_err());
        for table in ["sessions","outbox","initiations","identity"] {
            assert_eq!(db.query_row(&format!("SELECT count(*) FROM {table}"),[],|r|r.get::<_,i64>(0)).unwrap(),0);
        }
        db.execute_batch("DROP TRIGGER fail;").unwrap();
    }
    let packet = alice
        .start_initial(SESSION, MESSAGE, recipient, &bundle, b"atomic initial")
        .unwrap();
    bob.accept_initial(SLOT, SESSION, MESSAGE, alice.identity().unwrap(), &packet)
        .unwrap();
}

#[test]
fn outgoing_concurrent_retries_and_prekey_reuse_serialize() {
    for same in [false, true] {
        let dir = dir();
        let path = dir.path().join("alice.db");
        let mut alice = open(&path);
        alice.identity().unwrap();
        let mut bob = open(&dir.path().join("bob.db"));
        let recipient = bob.identity().unwrap();
        let bundle = bob.create_prekey(SLOT, false).unwrap();
        let barrier = std::sync::Barrier::new(2);
        let run = |session| {
            let mut store = open(&path);
            barrier.wait();
            store.start_initial(session, MESSAGE, recipient, &bundle, b"racing initial")
        };
        let result = std::thread::scope(|scope| {
            let a = scope.spawn(|| run(SESSION));
            let b = scope.spawn(|| run(if same { SESSION } else { [8; 32] }));
            [a.join().unwrap(), b.join().unwrap()]
        });
        if same {
            assert_eq!(result[0].as_ref().unwrap(), result[1].as_ref().unwrap());
        } else {
            assert_eq!(result.iter().filter(|r| r.is_ok()).count(), 1);
        }
        let db = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM initiations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}

#[test]
fn schema_two_migration_keeps_encrypted_identity_and_live_prekeys() {
    let dir = dir();
    let path = dir.path().join("client.db");
    let mut store = open(&path);
    let identity = store.identity().unwrap();
    let bundle = store.create_prekey(SLOT, true).unwrap();
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    common::rewind(&db, 2);
    let mut store = open(&path);
    assert_eq!(store.identity().unwrap(), identity);
    assert_eq!(store.create_prekey(SLOT, true).unwrap(), bundle);
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        i64::from(sigil_client::DATABASE_VERSION)
    );
}

#[test]
#[ignore = "invoked by outgoing_commit_survives_abrupt_exit"]
fn outgoing_worker() {
    let path = std::env::var_os("SIGIL_SYNTHETIC_OUTGOING_DB").unwrap();
    let file = std::env::var_os("SIGIL_SYNTHETIC_OUTGOING_BUNDLE").unwrap();
    let input = std::fs::read(file).unwrap();
    open(Path::new(&path))
        .start_initial(
            SESSION,
            MESSAGE,
            input[..32].try_into().unwrap(),
            &input[32..],
            b"durable outgoing",
        )
        .unwrap();
    std::process::exit(0);
}

#[test]
fn outgoing_commit_survives_abrupt_exit() {
    let dir = dir();
    let path = dir.path().join("alice.db");
    drop(open(&path));
    let mut bob = open(&dir.path().join("bob.db"));
    let recipient = bob.identity().unwrap();
    let bundle = bob.create_prekey(SLOT, true).unwrap();
    let mut input = recipient.to_vec();
    input.extend_from_slice(&bundle);
    let file = dir.path().join("synthetic-bundle");
    std::fs::write(&file, input).unwrap();
    assert!(std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "outgoing_worker"])
        .env("SIGIL_SYNTHETIC_OUTGOING_DB", &path)
        .env("SIGIL_SYNTHETIC_OUTGOING_BUNDLE", &file)
        .status()
        .unwrap()
        .success());
    let mut alice = open(&path);
    let pending = alice.pending(SESSION).unwrap();
    let packet = alice
        .start_initial(SESSION, MESSAGE, recipient, &bundle, b"durable outgoing")
        .unwrap();
    assert_eq!(pending, vec![(MESSAGE, packet.clone())]);
    assert_eq!(
        bob.accept_initial(SLOT, SESSION, MESSAGE, alice.identity().unwrap(), &packet)
            .unwrap()
            .as_slice(),
        b"durable outgoing"
    );
    assert!(alice
        .start_initial([8; 32], MESSAGE, recipient, &bundle, b"durable outgoing")
        .is_err());
}
