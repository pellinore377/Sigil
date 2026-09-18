use super::*;
use crate::auth::random_secret;
use sigil_protocol::{
    accounts::{Enrollment, InviteRequest},
    Configure as ConfigureServer, Settings,
};
const NOW: u64 = 1000;

fn setup(path: &std::path::Path) -> (Store, String, String) {
    let mut store = Store::open(path).unwrap();
    store
        .configure(ConfigureServer {
            expected_revision: 0,
            settings: Settings {
                server_name: "chat.example".into(),
                default_quota_bytes: 16 * 1024 * 1024,
                max_attachment_bytes: 1024 * 1024,
            },
        })
        .unwrap();
    let mut tokens = Vec::new();
    for username in ["alice", "bob"] {
        let invitation = store
            .invite(
                InviteRequest {
                    username: username.into(),
                    expires_in_seconds: 60,
                },
                NOW,
            )
            .unwrap();
        let credential = random_secret().unwrap();
        store
            .enroll(
                Enrollment {
                    invitation: invitation.secret,
                    device_credential: credential.clone(),
                    device_label: "Synthetic".into(),
                },
                NOW,
            )
            .unwrap();
        tokens.push(credential);
    }
    (store, tokens.remove(0), tokens.remove(0))
}
fn message(target: &str, n: u64) -> sigil_protocol::mailbox::Submit {
    sigil_protocol::mailbox::Submit {
        recipient_device: target.into(),
        message_id: format!("{n:064x}"),
        payload: "ab".repeat(32),
        expires_at: NOW + 100_000,
    }
}

#[test]
fn signing_out_a_device_frees_the_queue_it_can_never_collect_and_refuses_more() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let (mut store, alice, bob) = setup(&path);
    let sender = store.session(&alice, NOW).unwrap().device_id;
    let target = store.session(&bob, NOW).unwrap().device_id;
    store.allow_sender(&bob, &sender, NOW).unwrap();

    // Fill the per-pair allowance. Messages that outlive each other keep their
    // places, so the sender is refused rather than losing one unseen.
    let allowance = u64::from(crate::mailbox::PEER_ALLOWANCE);
    for n in 0..allowance {
        store.submit_message(&alice, message(&target, n), NOW).unwrap();
    }
    assert!(matches!(
        store.submit_message(&alice, message(&target, allowance), NOW),
        Err(StoreError::MailboxFull)
    ));

    let pending = |path: &std::path::Path| -> i64 {
        rusqlite::Connection::open(path)
            .unwrap()
            .query_row(
                "SELECT count(*) FROM mailbox WHERE payload IS NOT NULL AND expires_at>?1",
                [NOW as i64],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(pending(&path), allowance as i64);

    store.revoke_device(&bob, &target, NOW).unwrap();
    assert_eq!(pending(&path), 0, "a signed-out device must not hold its senders' slots");

    // Nothing new may be queued for it either, so the backlog cannot rebuild.
    assert!(matches!(
        store.submit_message(&alice, message(&target, allowance + 1), NOW),
        Err(StoreError::NotFound)
    ));
}

#[test]
fn short_lived_notices_give_way_to_a_message_that_outlives_them() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    let (mut store, alice, bob) = setup(&path);
    let sender = store.session(&alice, NOW).unwrap().device_id;
    let target = store.session(&bob, NOW).unwrap().device_id;
    store.allow_sender(&bob, &sender, NOW).unwrap();

    // Typing and presence describe a moment and are sent short-lived.
    let allowance = u64::from(crate::mailbox::PEER_ALLOWANCE);
    for n in 0..allowance {
        let mut notice = message(&target, n);
        notice.expires_at = NOW + 180;
        store.submit_message(&alice, notice, NOW).unwrap();
    }
    // A conversation cannot be silenced by notices nobody will ever read.
    store
        .submit_message(&alice, message(&target, allowance), NOW)
        .unwrap();
    let retired: i64 = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM mailbox WHERE payload IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(retired, 1, "exactly one notice gave way");
}
