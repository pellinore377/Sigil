use super::*;
use crate::{
    claims::tests::pair,
    incoming::tests::{next, trust},
};
const BUDGET: u64 = 8 * 1024 * 1024;
fn metadata() -> Metadata {
    Metadata {
        name: "synthetic.bin".into(),
        media_type: "application/octet-stream".into(),
    }
}
fn configure(store: &mut ClientStore, secret: u8) {
    let account =
        crate::connection::decode_id(&store.connection_session().unwrap().unwrap().account_id)
            .unwrap();
    store
        .configure_recovery("chat.example", account, Secret32::from_bytes([secret; 32]))
        .unwrap();
}
fn publish(store: &ClientStore, cache: &mut Cache, file: Id) {
    for expected in [
        UploadStep::Begun,
        UploadStep::Chunk(0),
        UploadStep::Published,
    ] {
        assert_eq!(store.upload_attachment_step(cache, file).unwrap(), expected);
    }
}

#[test]
fn recovery_media_is_reencrypted_republished_and_survives_original_removal() {
    let (dir, fixture, mut alice, mut bob, now) = pair();
    let (_, b) = trust(&mut alice, &mut bob);
    configure(&mut bob, 8);
    let mut sender = alice
        .open_attachment_cache(&dir.path().join("source.db"), BUDGET)
        .unwrap();
    let file = sender.prepare_upload(5, metadata(), None).unwrap();
    sender.stage_chunk(file, 0, b"hello").unwrap();
    sender.finish_staging(file).unwrap();
    publish(&alice, &mut sender, file);
    alice
        .queue_peer_file(b, [212; 32], (&mut sender, file), now, now)
        .unwrap();
    let session = resume(&mut alice, now);
    alice.send_pending_online(session, now).unwrap();
    let packet = next(&bob);
    let incoming = bob.accept_delivery(&packet).unwrap();
    let event = incoming.event().unwrap();
    let history = crate::event_history_id(&event, &alice.identity().unwrap());
    let mut cache = bob
        .open_attachment_cache(&dir.path().join("backup.db"), BUDGET)
        .unwrap();
    let status = bob.recovery_media_checkpoints(None).unwrap();
    assert!(status
        .records
        .iter()
        .any(|v| v.record == history && !v.protected && !v.excluded));
    assert_eq!(
        bob.republish_recovery_media_step(&mut cache, history, now)
            .unwrap(),
        MediaRecovery::Download(file)
    );
    bob.download_attachment_step(&mut cache, file, now).unwrap();
    bob.download_attachment_step(&mut cache, file, now).unwrap();
    let MediaRecovery::Staged(copy, 0) = bob
        .republish_recovery_media_step(&mut cache, history, now)
        .unwrap()
    else {
        panic!("copy not staged")
    };
    assert_ne!(copy, file);
    assert_eq!(
        bob.republish_recovery_media_step(&mut cache, history, now)
            .unwrap(),
        MediaRecovery::Upload(copy)
    );
    publish(&bob, &mut cache, copy);
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON archive_records BEGIN SELECT RAISE(ABORT,'synthetic'); END").unwrap();
    assert!(matches!(
        bob.republish_recovery_media_step(&mut cache, history, now),
        Err(Error::Storage(_))
    ));
    bob.db.execute_batch("DROP TRIGGER fail").unwrap();
    assert_eq!(
        bob.republish_recovery_media_step(&mut cache, history, now)
            .unwrap(),
        MediaRecovery::Protected(copy)
    );
    bob.maintain_recovery(now).unwrap();
    let head = bob.prepare_recovery_upload(now).unwrap();
    while bob.upload_recovery_step().unwrap().is_none() {}
    assert!(bob
        .recovery_media_checkpoints(None)
        .unwrap()
        .records
        .iter()
        .any(|v| v.record == history && v.protected));
    let session = bob.connection_session().unwrap().unwrap();
    let invite = sigil_server::store::Store::open(&dir.path().join("server.db"))
        .unwrap()
        .invite_reauthorization(&session.account_id, 60, now)
        .unwrap();
    let mut restored = ClientStore::open(
        &dir.path().join("lost-phone.db"),
        StorageKey::new(Secret32::from_bytes([10; 32])).unwrap(),
    )
    .unwrap();
    restored
        .prepare_enrollment(
            "chat.example",
            fixture.port(),
            &[crate::network::tests::CA.to_vec()],
            &invite.secret,
            "Synthetic restored phone",
            true,
        )
        .unwrap();
    restored.enroll_online().unwrap();
    configure(&mut restored, 8);
    for _ in 0..16 {
        if restored.download_recovery_step(true).unwrap() == Some(head) {
            break;
        }
    }
    assert_eq!(restored.recovery_status().unwrap().anchor, Some(head));
    alice
        .connected_client()
        .unwrap()
        .remove_attachment(file)
        .unwrap();
    let mut fresh = restored
        .open_attachment_cache(&dir.path().join("restored-media.db"), BUDGET)
        .unwrap();
    assert_eq!(
        restored
            .prepare_recovered_file(&mut fresh, history, now)
            .unwrap(),
        copy
    );
    restored
        .download_attachment_step(&mut fresh, copy, now)
        .unwrap();
    restored
        .download_attachment_step(&mut fresh, copy, now)
        .unwrap();
    assert_eq!(
        &*restored
            .recovered_file_chunk(&fresh, history, 0, now)
            .unwrap(),
        b"hello"
    );
    let snapshot = restored
        .recovery_records(None)
        .unwrap()
        .into_iter()
        .find(|v| {
            matches!(
                restored.recovery_record(v.id).unwrap().content,
                sigil_crypto::recovery::Content::Conversation(_)
            )
        })
        .unwrap();
    // Reauthorization revoked the lost phone's credential; continue on its replacement.
    let mut bob = restored;
    let mut cache = fresh;
    bob.set_recovery_policy(crate::recovery::RecoveryPolicy {
        history_days: Some(1),
    })
    .unwrap();
    bob.prepare_recovery_upload(now + 86400).unwrap();
    for _ in 0..16 {
        if bob.upload_recovery_step().unwrap().is_some() {
            break;
        }
    }
    assert!(bob.recovery_status().unwrap().pending.is_none());
    let mut cleaned = 0;
    for _ in 0..16 {
        let count = bob.cleanup_recovery_online().unwrap();
        cleaned += count;
        if count == 0 {
            break;
        }
    }
    assert!(cleaned >= 1);
    bob.maintain_attachment_cache(&mut cache, now + 86400)
        .unwrap();
    assert_eq!(cache.phase(copy).unwrap(), Phase::Complete);
    assert_eq!(
        &*bob
            .recovered_file_chunk(&cache, history, 0, now + 86400)
            .unwrap(),
        b"hello"
    );
    assert!(matches!(
        bob.recovery_record(history).unwrap().content,
        sigil_crypto::recovery::Content::Omitted
    ));
    assert_eq!(
        bob.republish_recovery_media_step(&mut cache, history, now + 86400)
            .unwrap(),
        MediaRecovery::Idle
    );
    assert!(bob
        .recovery_media_checkpoints(None)
        .unwrap()
        .records
        .iter()
        .any(|v| v.record == history && v.excluded && !v.protected));
    let revision = bob.recovery_record(snapshot.id).unwrap().revision;
    bob.delete_recovery_record(snapshot.id, revision).unwrap();
    bob.prepare_recovery_upload(now + 86400).unwrap();
    assert!(matches!(
        bob.recovery_record(history).unwrap().content,
        sigil_crypto::recovery::Content::Deleted
    ));
    bob.delete_recovery_record(history, 2).unwrap();
    assert!(matches!(
        bob.recovered_file_chunk(&cache, history, 0, now),
        Err(Error::Obsolete)
    ));
}
fn resume(store: &mut ClientStore, now: u64) -> Id {
    // A single retained intent can require a cursor-wrap pass after failure.
    for _ in 0..2 {
        if let Some(attempt) = store.resume_send_intents_online(now).unwrap().pop() {
            return attempt.result.unwrap();
        }
    }
    panic!("pending file intent was not revisited")
}
#[test]
fn published_file_uses_normal_claim_delivery_and_typed_atomic_recovery() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    configure(&mut alice, 7);
    configure(&mut bob, 8);
    let mut sender = alice
        .open_attachment_cache(&dir.path().join("sender-cache.db"), BUDGET)
        .unwrap();
    let file = sender.prepare_upload(5, metadata(), None).unwrap();
    sender.stage_chunk(file, 0, b"hello").unwrap();
    sender.finish_staging(file).unwrap();
    assert!(matches!(
        alice.queue_peer_file(b, [61; 32], (&mut sender, file), now, now),
        Err(Error::Unprepared)
    ));
    publish(&alice, &mut sender, file);
    alice
        .queue_peer_file(b, [61; 32], (&mut sender, file), now, now)
        .unwrap();
    alice
        .queue_peer_file(b, [61; 32], (&mut sender, file), now, now)
        .unwrap();
    assert!(matches!(
        alice.queue_peer_file(b, [61; 32], (&mut sender, file), now + 1, now),
        Err(Error::Conflict)
    ));
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON archive_records BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    let attempt = alice.resume_send_intents_online(now).unwrap().remove(0);
    assert!(matches!(attempt.result, Err(Error::Storage(_))));
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    alice.db.execute_batch("DROP TRIGGER fail").unwrap();
    let session = resume(&mut alice, now);
    alice.send_pending_online(session, now).unwrap();
    let delivery = next(&bob);
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON archive_records BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        bob.accept_delivery(&delivery),
        Err(Error::Storage(_))
    ));
    assert_eq!(
        bob.db
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    bob.db.execute_batch("DROP TRIGGER fail").unwrap();
    let attempt = bob.receive_mailbox_online(now).unwrap().remove(0);
    let sequence = attempt.sequence;
    let crate::MailboxEvent::File(incoming) = attempt.result.unwrap() else {
        panic!("file lost its event type")
    };
    assert!(incoming.text().is_err());
    let event = incoming.event().unwrap();
    assert_eq!(event.message, [61; 32]);
    assert_eq!(event.content.file().unwrap().name, "synthetic.bin");
    let history = crate::event_history_id(&event, &alice.identity().unwrap());
    assert!(matches!(
        alice.recovery_record(history).unwrap().content,
        sigil_crypto::recovery::Content::File(_)
    ));
    assert!(matches!(
        bob.recovery_record(history).unwrap().content,
        sigil_crypto::recovery::Content::File(_)
    ));
    let mut receiver = bob
        .open_attachment_cache(&dir.path().join("receiver-cache.db"), BUDGET)
        .unwrap();
    assert!(matches!(
        bob.prepare_received_file(&mut sender, sequence, now),
        Err(Error::Conflict)
    ));
    assert!(matches!(
        bob.prepare_received_file(&mut receiver, sequence + 1, now),
        Err(Error::NotFound)
    ));
    assert_eq!(
        bob.prepare_received_file(&mut receiver, sequence, now)
            .unwrap(),
        file
    );
    assert_eq!(
        bob.download_attachment_step(&mut receiver, file, now)
            .unwrap(),
        DownloadStep::Chunk(0)
    );
    assert_eq!(
        bob.download_attachment_step(&mut receiver, file, now)
            .unwrap(),
        DownloadStep::Complete
    );
    assert_eq!(
        receiver.completed_chunk(file, 0, now).unwrap().as_slice(),
        b"hello"
    );
    bob.acknowledge_incoming_online().unwrap();
    receiver.evict(file).unwrap();
    receiver.cleanup(file).unwrap();
    assert_eq!(
        bob.prepare_recovered_file(&mut receiver, history, now)
            .unwrap(),
        file
    );
    receiver.cancel(file).unwrap();
    receiver.evict(file).unwrap();
    receiver.cleanup(file).unwrap();
    let mut deleted = bob.recovery_record(history).unwrap();
    deleted.revision = 2;
    deleted.content = sigil_crypto::recovery::Content::Deleted;
    bob.retain_recovery_record(&deleted).unwrap();
    assert!(matches!(
        bob.prepare_recovered_file(&mut receiver, history, now),
        Err(Error::Obsolete)
    ));
    assert!(matches!(
        bob.prepare_received_file(&mut receiver, sequence, now),
        Err(Error::Obsolete)
    ));
    assert_eq!(
        bob.db
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        i64::from(crate::DATABASE_VERSION)
    );
}

#[test]
fn handoff_failure_cancellation_expiry_and_foreign_source_do_not_create_send_intents() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    let mut cache = alice
        .open_attachment_cache(&dir.path().join("cache.db"), BUDGET)
        .unwrap();
    let file = cache.prepare_upload(0, metadata(), Some(now + 60)).unwrap();
    cache.stage_chunk(file, 0, b"").unwrap();
    cache.finish_staging(file).unwrap();
    publish(&alice, &mut cache, file);
    alice.db.execute_batch("CREATE TRIGGER fail BEFORE INSERT ON send_intents BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        alice.queue_peer_file(b, [71; 32], (&mut cache, file), now, now),
        Err(Error::Storage(_))
    ));
    assert_eq!(cache.phase(file).unwrap(), Phase::Published);
    alice.db.execute_batch("DROP TRIGGER fail").unwrap();
    assert!(matches!(
        alice.queue_peer_file(b, [71; 32], (&mut cache, file), now, now + 60),
        Err(Error::Expired)
    ));
    let state = load(&cache.db, &cache.key, file).unwrap();
    let bad = Zeroizing::new(
        sigil_protocol::file::File {
            caption: "",
            source: "foreign.example",
            name: "synthetic.bin",
            media_type: "application/octet-stream",
            expires_at: None,
            access: &[1; 32],
            descriptor: state.descriptor.as_ref().unwrap(),
        }
        .to_bytes()
        .unwrap(),
    );
    assert!(matches!(
        alice.queue_peer_content(b, [71; 32], Content::File(&bad), now, now),
        Err(Error::InvalidEvent)
    ));
    cache.cancel(file).unwrap();
    assert!(matches!(
        alice.queue_peer_file(b, [71; 32], (&mut cache, file), now, now),
        Err(Error::Cancelled)
    ));
    assert_eq!(
        alice
            .db
            .query_row("SELECT count(*) FROM send_intents", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn fresh_device_recovers_file_key_and_downloads_media_without_restoring_sessions() {
    let (dir, fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    configure(&mut bob, 8);
    let mut cache = alice
        .open_attachment_cache(&dir.path().join("sender-cache.db"), BUDGET)
        .unwrap();
    let file = cache.prepare_upload(5, metadata(), None).unwrap();
    cache.stage_chunk(file, 0, b"media").unwrap();
    cache.finish_staging(file).unwrap();
    publish(&alice, &mut cache, file);
    alice
        .queue_peer_file(b, [81; 32], (&mut cache, file), now, now)
        .unwrap();
    let session = resume(&mut alice, now);
    alice.send_pending_online(session, now).unwrap();
    let received = bob.accept_delivery(&next(&bob)).unwrap();
    let history = crate::event_history_id(&received.event().unwrap(), &alice.identity().unwrap());
    let head = bob.prepare_recovery_upload(now).unwrap();
    assert_eq!(bob.upload_recovery_step().unwrap(), Some(head));
    let original = bob.connection_session().unwrap().unwrap();
    let mut server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
    let invite = server
        .invite_reauthorization(&original.account_id, 60, now)
        .unwrap();
    drop(bob);
    let path = dir.path().join("fresh.db");
    let open = || {
        ClientStore::open(
            &path,
            StorageKey::new(Secret32::from_bytes([10; 32])).unwrap(),
        )
        .unwrap()
    };
    let mut fresh = open();
    fresh
        .prepare_enrollment(
            "chat.example",
            fixture.port(),
            &[crate::network::tests::CA.to_vec()],
            &invite.secret,
            "Synthetic recovered device",
            true,
        )
        .unwrap();
    let enrolled = fresh.enroll_online().unwrap();
    assert_eq!(enrolled.account_id, original.account_id);
    assert_ne!(enrolled.device_id, original.device_id);
    configure(&mut fresh, 8);
    let mut complete = false;
    for _ in 0..8 {
        if fresh.download_recovery_step(true).unwrap() == Some(head) {
            complete = true;
            break;
        }
        assert!(fresh.recovery_records(None).unwrap().is_empty());
        drop(fresh);
        fresh = open();
    }
    assert!(complete);
    assert_eq!(fresh.recovery_records(None).unwrap().len(), 1);
    let mut recovered = fresh
        .open_attachment_cache(&dir.path().join("recovered-cache.db"), BUDGET)
        .unwrap();
    assert_eq!(
        fresh
            .prepare_recovered_file(&mut recovered, history, now)
            .unwrap(),
        file
    );
    assert_eq!(
        fresh
            .download_attachment_step(&mut recovered, file, now)
            .unwrap(),
        DownloadStep::Chunk(0)
    );
    assert_eq!(
        fresh
            .download_attachment_step(&mut recovered, file, now)
            .unwrap(),
        DownloadStep::Complete
    );
    assert_eq!(
        recovered.completed_chunk(file, 0, now).unwrap().as_slice(),
        b"media"
    );
    for table in ["sessions", "identity", "prekeys", "peers"] {
        assert_eq!(
            fresh
                .db
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    let mut record = fresh.recovery_record(history).unwrap();
    record.revision = 2;
    let sigil_crypto::recovery::Content::File(bytes) = record.content else {
        panic!("missing media recovery type")
    };
    record.content = sigil_crypto::recovery::Content::Retained(bytes);
    assert!(matches!(
        fresh.retain_recovery_record(&record),
        Err(Error::Conflict)
    ));
}

#[test]
fn lost_session_resends_exact_file_content_and_deduplicates_the_delayed_original() {
    let (dir, _fixture, mut alice, mut bob, now) = pair();
    let (_a, b) = trust(&mut alice, &mut bob);
    configure(&mut bob, 8);
    let mut cache = alice
        .open_attachment_cache(&dir.path().join("cache.db"), BUDGET)
        .unwrap();
    let file = cache.prepare_upload(0, metadata(), None).unwrap();
    cache.stage_chunk(file, 0, b"").unwrap();
    cache.finish_staging(file).unwrap();
    publish(&alice, &mut cache, file);
    alice
        .queue_peer_file(b, [91; 32], (&mut cache, file), now, now)
        .unwrap();
    let original_session = resume(&mut alice, now);
    alice.send_pending_online(original_session, now).unwrap();
    let original = next(&bob);
    let retry = bob.prepare_retry_request(&original, now).unwrap();
    bob.send_retry_request_online(retry, now).unwrap();
    let request = alice.accept_retry_request(&next(&alice), now).unwrap();
    bob.prepare_prekey_publication([80; 32], true, 3600)
        .unwrap();
    bob.publish_prekey_online([80; 32]).unwrap();
    alice.claim_prekey_online(request.claim, now).unwrap();
    let (session, packet) = alice.resend_event(retry, now).unwrap();
    assert_ne!(session, original_session);
    assert_eq!(
        alice.resend_event(retry, now + 1).unwrap(),
        (session, packet)
    );
    alice.send_pending_online(session, now).unwrap();
    let response = bob
        .connected_client()
        .unwrap()
        .mailbox()
        .unwrap()
        .into_iter()
        .find(|v| v.message_id == crate::transport::hex(&retry))
        .unwrap();
    let received = bob.accept_delivery(&response).unwrap();
    assert!(!received.duplicate);
    assert_eq!(received.event().unwrap().message, [91; 32]);
    assert!(received.event().unwrap().content.file().is_ok());
    let delayed = bob.accept_delivery(&original).unwrap();
    assert!(delayed.duplicate);
    assert_eq!(delayed.plaintext, received.plaintext);
    assert_eq!(bob.recovery_records(None).unwrap().len(), 1);
}

fn archived_file(cache: &Cache, file: Id, id: Id, now: u64) -> sigil_crypto::recovery::Record {
    let mut author = [0; 32];
    author[0] = 9;
    sigil_crypto::recovery::Record {
        id,
        revision: 1,
        conversation: [1; 32],
        author,
        created_at: now,
        direction: sigil_crypto::recovery::Direction::Incoming,
        content: sigil_crypto::recovery::Content::File(
            content(&load(&cache.db, &cache.key, file).unwrap(), "chat.example").unwrap(),
        ),
    }
}
#[cfg(target_os = "linux")]
#[test]
fn preview_requires_complete_authenticated_content_and_rechecks_expiry_and_deletion() {
    use sigil_media::{
        sandbox::{Job, Runtime},
        Content, Preview, Request,
    };
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::AtomicBool;
    let (dir, _fixture, alice, mut bob, now) = pair();
    configure(&mut bob, 8);
    let mut wire = Vec::new();
    Preview {
        content: Content::Text {
            text: "synthetic".into(),
            next: None,
        },
        bytes: Vec::new(),
    }
    .write(&mut wire)
    .unwrap();
    let mut sender = alice
        .open_attachment_cache(&dir.path().join("source.db"), BUDGET)
        .unwrap();
    let file = sender
        .prepare_upload(
            wire.len() as u64,
            Metadata {
                name: "synthetic.txt".into(),
                media_type: "text/plain".into(),
            },
            Some(now + 20),
        )
        .unwrap();
    sender.stage_chunk(file, 0, &wire).unwrap();
    sender.finish_staging(file).unwrap();
    publish(&alice, &mut sender, file);
    let record = [115; 32];
    bob.retain_recovery_record(&archived_file(&sender, file, record, now))
        .unwrap();
    let mut cache = bob
        .open_attachment_cache(&dir.path().join("cache.db"), BUDGET)
        .unwrap();
    bob.prepare_recovered_file(&mut cache, record, now).unwrap();
    let worker = dir.path().join("worker");
    std::fs::write(&worker, "#!/bin/sh\nexec /usr/bin/cat /input\n").unwrap();
    std::fs::set_permissions(&worker, std::fs::Permissions::from_mode(0o700)).unwrap();
    let runtime = Runtime {
        worker,
        pdfium: None,
        office_libraries: None,
        office_language_data: None,
    };
    let job = Job {
        runtime: &runtime,
        request: &Request::Text { offset: 0 },
        cancel: &AtomicBool::new(false),
    };
    assert!(matches!(
        bob.preview_recovered_file(&cache, record, &job, || now),
        Err(Error::Unprepared)
    ));
    bob.download_attachment_step(&mut cache, file, now).unwrap();
    bob.download_attachment_step(&mut cache, file, now).unwrap();
    let result = bob
        .preview_recovered_file(&cache, record, &job, || now)
        .unwrap();
    assert!(matches!(result.content,Content::Text {text,..} if text=="synthetic"));
    let mut tick = 0;
    assert!(matches!(
        bob.preview_recovered_file(&cache, record, &job, || {
            tick += 1;
            if tick == 1 {
                now
            } else {
                now + 21
            }
        }),
        Err(Error::Expired)
    ));
    let replacement = sender
        .prepare_upload(
            wire.len() as u64,
            Metadata {
                name: "replacement.txt".into(),
                media_type: "text/plain".into(),
            },
            None,
        )
        .unwrap();
    sender.stage_chunk(replacement, 0, &wire).unwrap();
    sender.finish_staging(replacement).unwrap();
    publish(&alice, &mut sender, replacement);
    bob.retain_recovery_record(&archived_file(&sender, replacement, [116; 32], now))
        .unwrap();
    bob.prepare_recovered_file(&mut cache, [116; 32], now)
        .unwrap();
    bob.download_attachment_step(&mut cache, replacement, now)
        .unwrap();
    bob.download_attachment_step(&mut cache, replacement, now)
        .unwrap();
    let mut other = ClientStore::open(
        &dir.path().join("bob.db"),
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    let mut changed = archived_file(&sender, replacement, record, now);
    changed.revision = 2;
    let mut tick = 0;
    assert!(matches!(
        bob.preview_recovered_file(&cache, record, &job, || {
            tick += 1;
            if tick == 2 {
                other.retain_recovery_record(&changed).unwrap();
            }
            now
        }),
        Err(Error::Obsolete)
    ));
    bob.delete_recovery_record(record, 2).unwrap();
    assert!(matches!(
        bob.preview_recovered_file(&cache, record, &job, || now),
        Err(Error::Obsolete)
    ));
}
#[test]
fn archived_cache_keeps_other_references_and_last_deletion_evicts_locally_after_failures() {
    let (dir, _fixture, alice, mut bob, now) = pair();
    configure(&mut bob, 8);
    let mut sender = alice
        .open_attachment_cache(&dir.path().join("sender.db"), BUDGET)
        .unwrap();
    let file = sender.prepare_upload(5, metadata(), None).unwrap();
    sender.stage_chunk(file, 0, b"hello").unwrap();
    sender.finish_staging(file).unwrap();
    publish(&alice, &mut sender, file);
    let first = [110; 32];
    let second = [111; 32];
    bob.retain_recovery_record(&archived_file(&sender, file, first, now))
        .unwrap();
    // This second archive reference is never separately prepared in the cache.
    bob.retain_recovery_record(&archived_file(&sender, file, second, now))
        .unwrap();
    let mut cache = bob
        .open_attachment_cache(&dir.path().join("receiver.db"), BUDGET)
        .unwrap();
    bob.prepare_recovered_file(&mut cache, first, now).unwrap();
    bob.download_attachment_step(&mut cache, file, now).unwrap();
    bob.download_attachment_step(&mut cache, file, now).unwrap();
    assert!(load(&cache.db, &cache.key, file).unwrap().managed);
    assert_eq!(
        bob.recovered_file_chunk(&cache, first, 0, now)
            .unwrap()
            .as_slice(),
        b"hello"
    );
    bob.db.execute_batch("CREATE TRIGGER fail BEFORE DELETE ON archive_media BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        bob.delete_recovery_record(first, 1),
        Err(Error::Storage(_))
    ));
    assert_eq!(bob.recovery_record(first).unwrap().revision, 1);
    bob.db.execute_batch("DROP TRIGGER fail").unwrap();
    bob.delete_recovery_record(first, 1).unwrap();
    assert!(matches!(
        bob.recovered_file_chunk(&cache, first, 0, now),
        Err(Error::Obsolete)
    ));
    assert_eq!(bob.maintain_attachment_cache(&mut cache, now).unwrap(), 0);
    assert_eq!(cache.phase(file).unwrap(), Phase::Complete);
    assert_eq!(
        bob.recovered_file_chunk(&cache, second, 0, now)
            .unwrap()
            .as_slice(),
        b"hello"
    );
    bob.delete_recovery_record(second, 1).unwrap();
    cache.db.execute_batch("CREATE TRIGGER fail BEFORE UPDATE ON files BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    assert!(matches!(
        bob.maintain_attachment_cache(&mut cache, now),
        Err(Error::Storage(_))
    ));
    assert_eq!(cache.phase(file).unwrap(), Phase::Complete);
    assert!(matches!(
        bob.recovered_file_chunk(&cache, second, 0, now),
        Err(Error::Obsolete)
    ));
    cache.db.execute_batch("DROP TRIGGER fail").unwrap();
    assert_eq!(bob.maintain_attachment_cache(&mut cache, now).unwrap(), 1);
    assert!(matches!(cache.phase(file), Err(Error::NotFound)));
    let state = load(&sender.db, &sender.key, file).unwrap();
    assert_eq!(
        alice
            .connected_client()
            .unwrap()
            .attachment_status(state.shape())
            .unwrap()
            .state,
        sigil_protocol::attachments::State::Published
    );
}

#[test]
fn media_index_migration_authenticates_history_and_legacy_cache_requires_a_new_handoff() {
    let (dir, _fixture, alice, mut bob, now) = pair();
    configure(&mut bob, 8);
    let mut sender = alice
        .open_attachment_cache(&dir.path().join("sender.db"), BUDGET)
        .unwrap();
    let file = sender.prepare_upload(0, metadata(), None).unwrap();
    sender.stage_chunk(file, 0, b"").unwrap();
    sender.finish_staging(file).unwrap();
    publish(&alice, &mut sender, file);
    let record = [112; 32];
    bob.retain_recovery_record(&archived_file(&sender, file, record, now))
        .unwrap();
    let path = dir.path().join("receiver.db");
    let mut cache = bob.open_attachment_cache(&path, BUDGET).unwrap();
    bob.prepare_recovered_file(&mut cache, record, now).unwrap();
    let mut old = serde_json::to_value(load(&cache.db, &cache.key, file).unwrap()).unwrap();
    old.as_object_mut().unwrap().remove("managed");
    let bytes = Zeroizing::new(serde_json::to_vec(&old).unwrap());
    cache
        .db
        .execute(
            "UPDATE files SET state=?1 WHERE id=?2",
            (
                cache.key.seal(&bytes, &aad(&file, None)).unwrap(),
                file.as_slice(),
            ),
        )
        .unwrap();
    cache
        .db
        .execute_batch("DROP TABLE recovery_transfers; DROP TABLE recovery_cursor;")
        .unwrap();
    cache.db.pragma_update(None, "user_version", 2).unwrap();
    drop(cache);
    let original: Vec<u8> = bob
        .db
        .query_row(
            "SELECT data FROM archive_records WHERE id=?1",
            [record.as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    crate::test_schema::rewind(&bob.db, 47);
    let mut corrupt = original.clone();
    corrupt[40] ^= 1;
    bob.db
        .execute(
            "UPDATE archive_records SET data=?1 WHERE id=?2",
            (corrupt, record.as_slice()),
        )
        .unwrap();
    drop(bob);
    let open = || {
        ClientStore::open(
            &dir.path().join("bob.db"),
            StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
        )
    };
    assert!(matches!(open(), Err(Error::Crypto(_))));
    let db = Connection::open(dir.path().join("bob.db")).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        47
    );
    assert!(!db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='archive_media')",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    db.execute(
        "UPDATE archive_records SET data=?1 WHERE id=?2",
        (original, record.as_slice()),
    )
    .unwrap();
    drop(db);
    let mut bob = open().unwrap();
    let mut cache = bob.open_attachment_cache(&path, BUDGET).unwrap();
    assert_eq!(
        cache
            .db
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        5
    );
    assert!(!load(&cache.db, &cache.key, file).unwrap().managed);
    assert_eq!(
        bob.db
            .query_row("SELECT count(*) FROM archive_media", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    bob.prepare_recovered_file(&mut cache, record, now).unwrap();
    assert!(load(&cache.db, &cache.key, file).unwrap().managed);
    bob.delete_recovery_record(record, 1).unwrap();
    assert_eq!(bob.maintain_attachment_cache(&mut cache, now).unwrap(), 0);
    assert!(matches!(cache.phase(file), Err(Error::NotFound)));
}
