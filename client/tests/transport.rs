use sigil_client::{ClientStore, Error};
mod common;
use sigil_crypto::{storage::StorageKey, triple::Session, DhKey, Secret32};
use sigil_protocol::mailbox::Submit;
use std::{os::unix::fs::PermissionsExt, path::Path};
const SESSION: [u8; 32] = [1; 32];
const MESSAGE: [u8; 32] = [2; 32];
const RECIPIENT: [u8; 32] = [3; 32];
const NOW: u64 = 1000;
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
fn seed(store: &mut ClientStore, id: [u8; 32]) {
    let dh = DhKey::generate().unwrap();
    store
        .insert_session(
            id,
            Session::initiator(Secret32::from_bytes([7; 32]), dh.public_key(), [8; 32]).unwrap(),
        )
        .unwrap();
    store.send(id, MESSAGE, b"synthetic transport").unwrap();
}
fn same(a: &Submit, b: &Submit) {
    assert_eq!(a.recipient_device, b.recipient_device);
    assert_eq!(a.message_id, b.message_id);
    assert_eq!(a.payload, b.payload);
    assert_eq!(a.expires_at, b.expires_at);
}
fn decode_id(text: &str) -> [u8; 32] {
    let mut id = [0; 32];
    for (n, part) in text.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        id[n] = u8::from_str_radix(std::str::from_utf8(part).unwrap(), 16).unwrap();
    }
    id
}

#[test]
fn frozen_requests_retry_through_server_storage_after_restart() {
    use sigil_protocol::{
        accounts::{Enrollment, InviteRequest},
        Configure, Settings,
    };
    let dir = dir();
    let path = dir.path().join("client.db");
    let mut client = open(&path);
    seed(&mut client, SESSION);
    let mut server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
    server
        .configure(Configure {
            expected_revision: 0,
            settings: Settings {
                server_name: "chat.example".into(),
                default_quota_bytes: sigil_protocol::DEFAULT_QUOTA,
                max_attachment_bytes: sigil_protocol::DEFAULT_ATTACHMENT_LIMIT,
            },
        })
        .unwrap();
    let mut enroll = |username: &str, credential: &str| {
        let invite = server
            .invite(
                InviteRequest {
                    username: username.into(),
                    expires_in_seconds: 60,
                },
                NOW,
            )
            .unwrap();
        server
            .enroll(
                Enrollment {
                    invitation: invite.secret,
                    device_credential: credential.into(),
                    device_label: "Synthetic device".into(),
                },
                NOW,
            )
            .unwrap()
    };
    let alice_token = "a".repeat(64);
    let bob_token = "b".repeat(64);
    let alice = enroll("alice", &alice_token);
    let bob = enroll("bob", &bob_token);
    server
        .allow_sender(&bob_token, &alice.device_id, NOW)
        .unwrap();
    let recipient = decode_id(&bob.device_id);
    let request = client
        .prepare_delivery(SESSION, MESSAGE, recipient, NOW + 60, NOW)
        .unwrap();
    let receipt = server.submit_message(&alice_token, request, NOW).unwrap();
    drop(client);
    let client = open(&path);
    let retried = client
        .pending_deliveries(SESSION, NOW + 1)
        .unwrap()
        .remove(0);
    assert_eq!(
        server
            .submit_message(&alice_token, retried, NOW + 1)
            .unwrap(),
        receipt
    );
    assert_eq!(server.mailbox(&bob_token, NOW + 1).unwrap().len(), 1);
    let mut client = client;
    client.acknowledge_sent(SESSION, MESSAGE, &receipt).unwrap();
    drop(client);
    let mut client = open(&path);
    assert!(client
        .pending_deliveries(SESSION, NOW + 2)
        .unwrap()
        .is_empty());
    assert!(matches!(
        client.prepare_delivery(SESSION, MESSAGE, recipient, NOW + 60, NOW + 2),
        Err(Error::AlreadyDelivered)
    ));
    assert!(matches!(
        client.prepare_delivery(SESSION, MESSAGE, RECIPIENT, NOW + 60, NOW + 2),
        Err(Error::Conflict)
    ));
}

#[test]
fn concurrent_preparation_cannot_change_frozen_fields() {
    let dir = dir();
    let path = dir.path().join("client.db");
    let mut client = open(&path);
    seed(&mut client, SESSION);
    let barrier = std::sync::Barrier::new(2);
    let run = |recipient| {
        let mut client = open(&path);
        barrier.wait();
        client.prepare_delivery(SESSION, MESSAGE, recipient, NOW + 60, NOW)
    };
    let result = std::thread::scope(|scope| {
        let a = scope.spawn(|| run(RECIPIENT));
        let b = scope.spawn(|| run([4; 32]));
        [a.join().unwrap(), b.join().unwrap()]
    });
    assert_eq!(result.iter().filter(|r| r.is_ok()).count(), 1);
    let winner = result.into_iter().find_map(Result::ok).unwrap();
    same(
        &client.pending_deliveries(SESSION, NOW + 1).unwrap()[0],
        &winner,
    );
    same(
        &client
            .prepare_delivery(
                SESSION,
                MESSAGE,
                decode_id(&winner.recipient_device),
                NOW + 60,
                NOW + 1,
            )
            .unwrap(),
        &winner,
    );
}

#[test]
fn expiry_and_failed_preparation_leave_committed_packets_intact() {
    let dir = dir();
    let path = dir.path().join("client.db");
    let mut client = open(&path);
    seed(&mut client, SESSION);
    let packet = client.pending(SESSION).unwrap();
    assert!(matches!(
        client.pending_deliveries(SESSION, NOW),
        Err(Error::Unprepared)
    ));
    for expires in [NOW, NOW + 604801, u64::MAX] {
        assert!(matches!(
            client.prepare_delivery(SESSION, MESSAGE, RECIPIENT, expires, NOW),
            Err(Error::Expired)
        ));
    }
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON deliveries BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
    assert!(client
        .prepare_delivery(SESSION, MESSAGE, RECIPIENT, NOW + 60, NOW)
        .is_err());
    assert_eq!(client.pending(SESSION).unwrap(), packet);
    assert!(matches!(
        client.pending_deliveries(SESSION, NOW),
        Err(Error::Unprepared)
    ));
    db.execute_batch("DROP TRIGGER fail;").unwrap();
    client
        .prepare_delivery(SESSION, MESSAGE, RECIPIENT, NOW + 60, NOW)
        .unwrap();
    assert!(matches!(
        client.pending_deliveries(SESSION, NOW + 60),
        Err(Error::Expired)
    ));
    assert!(matches!(
        client.prepare_delivery(SESSION, MESSAGE, RECIPIENT, NOW + 61, NOW + 60),
        Err(Error::Conflict)
    ));
    assert!(matches!(
        client.prepare_delivery(SESSION, MESSAGE, RECIPIENT, NOW + 60, NOW + 60),
        Err(Error::Expired)
    ));
    assert_eq!(client.pending(SESSION).unwrap(), packet);
}

#[test]
fn metadata_tampering_and_cross_session_message_ids_fail_closed() {
    let dir = dir();
    let path = dir.path().join("client.db");
    let mut client = open(&path);
    seed(&mut client, SESSION);
    seed(&mut client, [4; 32]);
    client
        .prepare_delivery(SESSION, MESSAGE, RECIPIENT, NOW + 60, NOW)
        .unwrap();
    assert!(matches!(
        client.prepare_delivery([4; 32], MESSAGE, RECIPIENT, NOW + 60, NOW),
        Err(Error::Conflict)
    ));
    let db = rusqlite::Connection::open(&path).unwrap();
    let mut metadata: Vec<u8> = db
        .query_row("SELECT metadata FROM deliveries", [], |r| r.get(0))
        .unwrap();
    metadata[25] ^= 1;
    db.execute("UPDATE deliveries SET metadata=?1", [metadata])
        .unwrap();
    assert!(client.pending_deliveries(SESSION, NOW).is_err());
    assert!(client
        .prepare_delivery(SESSION, MESSAGE, RECIPIENT, NOW + 60, NOW)
        .is_err());
}

#[test]
fn migration_preserves_legacy_packets_and_new_sessions_require_transport_fields() {
    let dir = dir();
    let path = dir.path().join("client.db");
    let mut client = open(&path);
    seed(&mut client, SESSION);
    drop(client);
    let db = rusqlite::Connection::open(&path).unwrap();
    let packet: Vec<u8> = db
        .query_row("SELECT packet FROM outbox", [], |r| r.get(0))
        .unwrap();
    common::rewind(&db, 3);
    let mut client = open(&path);
    assert!(matches!(
        client.pending(SESSION),
        Err(Error::UnsupportedSession)
    ));
    assert_eq!(
        db.query_row("SELECT packet FROM outbox", [], |r| r.get::<_, Vec<u8>>(0))
            .unwrap(),
        packet
    );
    assert!(matches!(
        client.pending_deliveries(SESSION, NOW),
        Err(Error::UnsupportedSession)
    ));
    assert!(matches!(
        client.prepare_delivery(SESSION, MESSAGE, RECIPIENT, NOW + 60, NOW),
        Err(Error::UnsupportedSession)
    ));
    let session = [77; 32];
    seed(&mut client, session);
    assert!(matches!(
        client.pending_deliveries(session, NOW),
        Err(Error::Unprepared)
    ));
    client
        .prepare_delivery(session, MESSAGE, RECIPIENT, NOW + 60, NOW)
        .unwrap();
    client.send(session, [5; 32], b"second").unwrap();
    assert!(matches!(
        client.pending_deliveries(session, NOW),
        Err(Error::Unprepared)
    ));
    client
        .prepare_delivery(session, [5; 32], RECIPIENT, NOW + 60, NOW)
        .unwrap();
    assert_eq!(client.pending_deliveries(session, NOW).unwrap().len(), 2);
}

#[test]
fn maximum_initial_packet_preserves_all_transport_fields() {
    let dir = dir();
    let path = dir.path().join("alice.db");
    let mut alice = open(&path);
    let mut bob = open(&dir.path().join("bob.db"));
    let peer = bob.identity().unwrap();
    let bundle = bob.create_prekey([6; 32], true).unwrap();
    alice
        .start_initial(
            SESSION,
            MESSAGE,
            peer,
            &bundle,
            &vec![42; sigil_protocol::initial::MAX_PLAINTEXT],
        )
        .unwrap();
    let first = alice
        .prepare_delivery(SESSION, MESSAGE, RECIPIENT, NOW + 60, NOW)
        .unwrap();
    assert_eq!(
        first.payload.len(),
        sigil_protocol::mailbox::MAX_PAYLOAD_HEX
    );
    drop(alice);
    same(
        &first,
        &open(&path).pending_deliveries(SESSION, NOW + 1).unwrap()[0],
    );
}

#[test]
fn invalid_receipts_and_unprepared_packets_cannot_clear_queue() {
    use sigil_protocol::mailbox::Receipt;
    let dir = dir();
    let path = dir.path().join("client.db");
    let mut client = open(&path);
    seed(&mut client, SESSION);
    let valid = Receipt {
        sequence: 7,
        expires_at: NOW + 60,
    };
    assert!(client.acknowledge_sent(SESSION, MESSAGE, &valid).is_err());
    client
        .prepare_delivery(SESSION, MESSAGE, RECIPIENT, NOW + 60, NOW)
        .unwrap();
    for receipt in [
        Receipt {
            sequence: 0,
            expires_at: NOW + 60,
        },
        Receipt {
            sequence: -1,
            expires_at: NOW + 60,
        },
        Receipt {
            sequence: 7,
            expires_at: NOW + 61,
        },
        Receipt {
            sequence: 7,
            expires_at: u64::MAX,
        },
    ] {
        assert!(client.acknowledge_sent(SESSION, MESSAGE, &receipt).is_err());
        assert!(client.delivery_receipt(SESSION, MESSAGE).unwrap().is_none());
        assert_eq!(client.pending_deliveries(SESSION, NOW).unwrap().len(), 1);
    }
    assert!(client.acknowledge_sent([9; 32], MESSAGE, &valid).is_err());
    // A delayed genuine receipt may arrive after the request expires locally.
    assert!(matches!(
        client.pending_deliveries(SESSION, NOW + 60),
        Err(Error::Expired)
    ));
    client.acknowledge_sent(SESSION, MESSAGE, &valid).unwrap();
    drop(client);
    let mut client = open(&path);
    assert_eq!(
        client.delivery_receipt(SESSION, MESSAGE).unwrap(),
        Some(valid)
    );
    client
        .acknowledge_sent(
            SESSION,
            MESSAGE,
            &Receipt {
                sequence: 7,
                expires_at: NOW + 60,
            },
        )
        .unwrap();
    assert!(matches!(
        client.acknowledge_sent(
            SESSION,
            MESSAGE,
            &Receipt {
                sequence: 8,
                expires_at: NOW + 60
            }
        ),
        Err(Error::Conflict)
    ));
    assert!(client
        .pending_deliveries(SESSION, NOW + 60)
        .unwrap()
        .is_empty());
}

#[test]
fn receipt_and_packet_removal_roll_back_together() {
    use sigil_protocol::mailbox::Receipt;
    let dir = dir();
    let path = dir.path().join("client.db");
    let mut client = open(&path);
    seed(&mut client, SESSION);
    client
        .prepare_delivery(SESSION, MESSAGE, RECIPIENT, NOW + 60, NOW)
        .unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    for trigger in [
        "CREATE TRIGGER fail BEFORE UPDATE ON deliveries BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;",
        "CREATE TRIGGER fail BEFORE UPDATE ON outbox BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;",
    ] {
        db.execute_batch(trigger).unwrap();
        assert!(client.acknowledge_sent(SESSION,MESSAGE,&Receipt {sequence:1,expires_at:NOW+60}).is_err());
        assert!(client.delivery_receipt(SESSION,MESSAGE).unwrap().is_none());
        assert_eq!(client.pending_deliveries(SESSION,NOW).unwrap().len(),1);
        db.execute_batch("DROP TRIGGER fail;").unwrap();
    }
    client
        .acknowledge_sent(
            SESSION,
            MESSAGE,
            &Receipt {
                sequence: 1,
                expires_at: NOW + 60,
            },
        )
        .unwrap();
    assert!(client.pending_deliveries(SESSION, NOW).unwrap().is_empty());
}

#[test]
fn competing_server_receipts_cannot_overwrite_acceptance() {
    use sigil_protocol::mailbox::Receipt;
    let dir = dir();
    let path = dir.path().join("client.db");
    let mut client = open(&path);
    seed(&mut client, SESSION);
    client
        .prepare_delivery(SESSION, MESSAGE, RECIPIENT, NOW + 60, NOW)
        .unwrap();
    let barrier = std::sync::Barrier::new(2);
    let run = |sequence| {
        let mut client = open(&path);
        barrier.wait();
        client.acknowledge_sent(
            SESSION,
            MESSAGE,
            &Receipt {
                sequence,
                expires_at: NOW + 60,
            },
        )
    };
    let results = std::thread::scope(|scope| {
        let a = scope.spawn(|| run(1));
        let b = scope.spawn(|| run(2));
        [a.join().unwrap(), b.join().unwrap()]
    });
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    let receipt = client.delivery_receipt(SESSION, MESSAGE).unwrap().unwrap();
    assert_eq!(receipt.sequence, if results[0].is_ok() { 1 } else { 2 });
    assert!(client.pending_deliveries(SESSION, NOW).unwrap().is_empty());
}

#[test]
fn encrypted_receipt_tampering_is_rejected() {
    use sigil_protocol::mailbox::Receipt;
    let dir = dir();
    let path = dir.path().join("client.db");
    let mut client = open(&path);
    seed(&mut client, SESSION);
    client
        .prepare_delivery(SESSION, MESSAGE, RECIPIENT, NOW + 60, NOW)
        .unwrap();
    let receipt = Receipt {
        sequence: 1,
        expires_at: NOW + 60,
    };
    client.acknowledge_sent(SESSION, MESSAGE, &receipt).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    let mut sealed: Vec<u8> = db
        .query_row("SELECT receipt FROM deliveries", [], |r| r.get(0))
        .unwrap();
    sealed[25] ^= 1;
    db.execute("UPDATE deliveries SET receipt=?1", [sealed])
        .unwrap();
    assert!(client.delivery_receipt(SESSION, MESSAGE).is_err());
    assert!(client.acknowledge_sent(SESSION, MESSAGE, &receipt).is_err());
}

#[test]
fn schema_four_migration_does_not_invent_missing_receipts() {
    use sigil_protocol::mailbox::Receipt;
    let dir = dir();
    let path = dir.path().join("client.db");
    let mut client = open(&path);
    seed(&mut client, SESSION);
    client
        .prepare_delivery(SESSION, MESSAGE, RECIPIENT, NOW + 60, NOW)
        .unwrap();
    drop(client);
    let db = rusqlite::Connection::open(&path).unwrap();
    common::rewind(&db, 4);
    db.execute("UPDATE outbox SET packet=NULL", []).unwrap();
    let mut client = open(&path);
    assert!(client.delivery_receipt(SESSION, MESSAGE).unwrap().is_none());
    assert!(matches!(
        client.acknowledge_sent(
            SESSION,
            MESSAGE,
            &Receipt {
                sequence: 1,
                expires_at: NOW + 60
            }
        ),
        Err(Error::AlreadyDelivered)
    ));
    assert!(matches!(
        client.pending_deliveries(SESSION, NOW),
        Err(Error::UnsupportedSession)
    ));
}

#[test]
fn explicit_expiry_survives_restart_unblocks_queue_and_allows_retirement() {
    let dir = dir();
    let path = dir.path().join("client.db");
    let mut client = open(&path);
    seed(&mut client, SESSION);
    assert!(client.expire_delivery(SESSION, MESSAGE, NOW + 60).is_err());
    client
        .prepare_delivery(SESSION, MESSAGE, RECIPIENT, NOW + 60, NOW)
        .unwrap();
    let next = [5; 32];
    client.send(SESSION, next, b"synthetic successor").unwrap();
    client
        .prepare_delivery(SESSION, next, RECIPIENT, NOW + 120, NOW)
        .unwrap();
    let packets = client.pending(SESSION).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    let checkpoint = || {
        db.query_row("SELECT revision,state FROM sessions", [], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?))
        })
        .unwrap()
    };
    let before = checkpoint();
    assert!(client.expire_delivery(SESSION, MESSAGE, NOW + 59).is_err());
    assert_eq!(client.pending(SESSION).unwrap(), packets);
    assert!(!client.delivery_expired(SESSION, MESSAGE).unwrap());
    assert!(matches!(
        client.pending_deliveries(SESSION, NOW + 60),
        Err(Error::Expired)
    ));
    assert!(client.expire_delivery(SESSION, MESSAGE, NOW + 60).unwrap());
    assert_eq!(checkpoint(), before);
    assert!(client.delivery_receipt(SESSION, MESSAGE).unwrap().is_none());
    assert_eq!(
        client
            .outgoing_message(SESSION, MESSAGE)
            .unwrap()
            .as_slice(),
        b"synthetic transport"
    );
    assert_eq!(
        client.pending_deliveries(SESSION, NOW + 60).unwrap().len(),
        1
    );
    assert!(matches!(
        client.send(SESSION, MESSAGE, b"synthetic transport"),
        Err(Error::Expired)
    ));
    assert!(matches!(
        client.send(SESSION, MESSAGE, b"changed"),
        Err(Error::Conflict)
    ));
    assert!(matches!(
        client.prepare_delivery(SESSION, MESSAGE, RECIPIENT, NOW + 60, NOW + 60),
        Err(Error::Expired)
    ));
    drop(client);
    let mut client = open(&path);
    assert!(client.delivery_expired(SESSION, MESSAGE).unwrap());
    assert!(!client.expire_delivery(SESSION, MESSAGE, NOW + 60).unwrap());
    assert!(client.retire_session(SESSION).is_err());
    assert!(client.expire_delivery(SESSION, next, NOW + 120).unwrap());
    client.retire_session(SESSION).unwrap();
    let receipt = sigil_protocol::mailbox::Receipt {
        sequence: 1,
        expires_at: NOW + 60,
    };
    client.acknowledge_sent(SESSION, MESSAGE, &receipt).unwrap();
    assert!(!client.delivery_expired(SESSION, MESSAGE).unwrap());
    assert_eq!(
        client
            .delivery_receipt(SESSION, MESSAGE)
            .unwrap()
            .unwrap()
            .sequence,
        1
    );
    assert!(!client.expire_delivery(SESSION, MESSAGE, NOW + 120).unwrap());
    client.retire_session(SESSION).unwrap();
}

#[test]
fn expiry_marker_and_packet_removal_are_atomic_and_authenticated() {
    let dir = dir();
    let path = dir.path().join("client.db");
    let mut client = open(&path);
    seed(&mut client, SESSION);
    client
        .prepare_delivery(SESSION, MESSAGE, RECIPIENT, NOW + 60, NOW)
        .unwrap();
    let packets = client.pending(SESSION).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    for trigger in [
        "CREATE TRIGGER fail BEFORE UPDATE ON deliveries BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;",
        "CREATE TRIGGER fail BEFORE UPDATE ON outbox BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;",
    ] {
        db.execute_batch(trigger).unwrap();
        assert!(client.expire_delivery(SESSION, MESSAGE, NOW + 60).is_err());
        assert!(!client.delivery_expired(SESSION, MESSAGE).unwrap());
        assert_eq!(client.pending(SESSION).unwrap(), packets);
        db.execute_batch("DROP TRIGGER fail;").unwrap();
    }
    client.expire_delivery(SESSION, MESSAGE, NOW + 60).unwrap();
    let marker: Vec<u8> = db
        .query_row("SELECT expired FROM deliveries", [], |r| r.get(0))
        .unwrap();
    let mut damaged = marker.clone();
    damaged[20] ^= 1;
    db.execute("UPDATE deliveries SET expired=?1", [damaged])
        .unwrap();
    assert!(client.delivery_expired(SESSION, MESSAGE).is_err());
    assert!(client.expire_delivery(SESSION, MESSAGE, NOW + 60).is_err());
    assert!(client
        .acknowledge_sent(
            SESSION,
            MESSAGE,
            &sigil_protocol::mailbox::Receipt {
                sequence: 1,
                expires_at: NOW + 60
            }
        )
        .is_err());
    db.execute("UPDATE deliveries SET expired=?1", [marker.clone()])
        .unwrap();
    let other = [6; 32];
    client
        .send(SESSION, other, b"another synthetic packet")
        .unwrap();
    client
        .prepare_delivery(SESSION, other, RECIPIENT, NOW + 60, NOW)
        .unwrap();
    db.execute(
        "UPDATE deliveries SET expired=?1 WHERE id=?2",
        (marker, other.as_slice()),
    )
    .unwrap();
    assert!(client.expire_delivery(SESSION, other, NOW + 60).is_err());
    assert_eq!(client.pending(SESSION).unwrap().len(), 1);
}

#[test]
fn schema_twenty_delivery_survives_migration_and_expiry_receipt_race() {
    let dir = dir();
    let path = dir.path().join("client.db");
    let mut client = open(&path);
    seed(&mut client, SESSION);
    let request = client
        .prepare_delivery(SESSION, MESSAGE, RECIPIENT, NOW + 60, NOW)
        .unwrap();
    drop(client);
    common::rewind(&rusqlite::Connection::open(&path).unwrap(), 20);
    let mut client = open(&path);
    same(
        &client.pending_deliveries(SESSION, NOW).unwrap()[0],
        &request,
    );
    assert!(!client.delivery_expired(SESSION, MESSAGE).unwrap());
    let receipt = sigil_protocol::mailbox::Receipt {
        sequence: 1,
        expires_at: NOW + 60,
    };
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let expiry = scope.spawn(|| {
            let mut client = open(&path);
            barrier.wait();
            client.expire_delivery(SESSION, MESSAGE, NOW + 60).unwrap();
        });
        let acceptance = scope.spawn(|| {
            let mut client = open(&path);
            barrier.wait();
            client.acknowledge_sent(SESSION, MESSAGE, &receipt).unwrap();
        });
        expiry.join().unwrap();
        acceptance.join().unwrap();
    });
    assert!(!client.delivery_expired(SESSION, MESSAGE).unwrap());
    assert_eq!(
        client.delivery_receipt(SESSION, MESSAGE).unwrap().unwrap(),
        receipt
    );
    assert!(client.pending(SESSION).unwrap().is_empty());
    client.retire_session(SESSION).unwrap();
}
