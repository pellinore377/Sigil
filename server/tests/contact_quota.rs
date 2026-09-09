use sha2::{Digest, Sha256};
use sigil_protocol::{
    accounts::{Enrollment, InviteRequest},
    contacts::CreateContactInvite,
    recovery::{PutObject, MAX_OBJECT_BYTES},
    Configure, Settings,
};
use sigil_server::store::{Store, StoreError};

const NOW: u64 = 1000;
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn fixture(path: &std::path::Path) -> (Store, Vec<(String, String)>) {
    let mut store = Store::open(path).unwrap();
    store
        .configure(Configure {
            expected_revision: 0,
            settings: Settings {
                server_name: "quota.example".into(),
                default_quota_bytes: 1048576,
                max_attachment_bytes: 1048576,
            },
        })
        .unwrap();
    let mut devices = Vec::new();
    for index in 1..=3u8 {
        let invitation = store
            .invite(
                InviteRequest {
                    username: format!("synthetic{index}"),
                    expires_in_seconds: 60,
                },
                NOW,
            )
            .unwrap();
        let token = hex(&[index; 32]);
        let session = store
            .enroll(
                Enrollment {
                    invitation: invitation.secret,
                    device_credential: token.clone(),
                    device_label: "Synthetic accounting fixture".into(),
                },
                NOW,
            )
            .unwrap();
        devices.push((token, session.device_id));
    }
    (store, devices)
}
fn usage(store: &mut Store, token: &str) -> u64 {
    store.account_storage(token, NOW).unwrap().used_bytes
}
fn full(store: &mut Store, token: &str) {
    let mut index = 0u32;
    loop {
        let current = store.account_storage(token, NOW).unwrap();
        let remaining = (current.quota_bytes - current.used_bytes) as usize;
        if remaining == 0 {
            return;
        }
        assert!(remaining >= 36);
        let mut bytes = vec![9; remaining.min(MAX_OBJECT_BYTES)];
        bytes[..4].copy_from_slice(&index.to_be_bytes());
        store
            .put_recovery_object(
                token,
                &hex(&Sha256::digest(&bytes)),
                PutObject {
                    ciphertext: hex(&bytes),
                },
                NOW,
            )
            .unwrap();
        index += 1;
    }
}
#[test]
fn full_account_rejects_new_sender_grant_but_can_remove_existing_grant() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, devices) = fixture(&directory.path().join("server.db"));
    let token = &devices[0].0;
    store.allow_sender(token, &devices[1].1, NOW).unwrap();
    assert_eq!(
        store.allowed_senders(token, NOW).unwrap(),
        vec![devices[1].1.clone()]
    );
    full(&mut store, token);
    store.allow_sender(token, &devices[1].1, NOW).unwrap();
    store.remove_sender(token, &devices[1].1, NOW).unwrap();
    assert!(store.allowed_senders(token, NOW).unwrap().is_empty());
    full(&mut store, token);
    let control = vec![8; 36];
    assert!(matches!(
        store.put_recovery_object(
            token,
            &hex(&Sha256::digest(&control)),
            PutObject {
                ciphertext: hex(&control)
            },
            NOW
        ),
        Err(StoreError::Busy)
    ));
    assert!(matches!(
        store.allow_sender(token, &devices[2].1, NOW),
        Err(StoreError::Busy)
    ));
}

#[test]
fn full_account_rejects_new_contact_storage_but_can_revoke_existing_invitation() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, devices) = fixture(&directory.path().join("server.db"));
    let token = &devices[0].0;
    let request = || CreateContactInvite {
        secret: hex(&[4; 32]),
        expires_at: NOW + 60,
    };
    let prior = store.create_contact_invite(token, request(), NOW).unwrap();
    full(&mut store, token);
    assert_eq!(
        store
            .create_contact_invite(token, request(), NOW)
            .unwrap()
            .id,
        prior.id
    );
    let control = vec![8; 36];
    assert!(matches!(
        store.put_recovery_object(
            token,
            &hex(&Sha256::digest(&control)),
            PutObject {
                ciphertext: hex(&control)
            },
            NOW
        ),
        Err(StoreError::Busy)
    ));
    store.revoke_contact_invite(token, &prior.id, NOW).unwrap();
    assert!(matches!(
        store.create_contact_invite(
            token,
            CreateContactInvite {
                secret: hex(&[5; 32]),
                expires_at: NOW + 60,
            },
            NOW
        ),
        Err(StoreError::Busy)
    ));
}

#[test]
fn reciprocal_contact_quota_failure_rolls_back_both_grants_and_claim() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, devices) = fixture(&directory.path().join("server.db"));
    let owner = &devices[0];
    let claimant = &devices[1];
    let other = &devices[2];
    let secret = hex(&[20; 32]);
    let invite = store
        .create_contact_invite(
            &owner.0,
            CreateContactInvite {
                secret: secret.clone(),
                expires_at: NOW + 60,
            },
            NOW,
        )
        .unwrap();
    full(&mut store, &claimant.0);
    let before_owner = usage(&mut store, &owner.0);
    let before_claimant = usage(&mut store, &claimant.0);
    // Owner's first grant has room; claimant's reciprocal grant must fail.
    assert!(matches!(
        store.redeem_contact_invite(&claimant.0, &secret, NOW),
        Err(StoreError::Busy)
    ));
    assert_eq!(usage(&mut store, &owner.0), before_owner);
    assert_eq!(usage(&mut store, &claimant.0), before_claimant);
    assert!(store.allowed_senders(&owner.0, NOW).unwrap().is_empty());
    assert!(store.allowed_senders(&claimant.0, NOW).unwrap().is_empty());
    // A different claimant can still redeem: the failed claim was not retained.
    let before_other = usage(&mut store, &other.0);
    assert_eq!(
        store
            .redeem_contact_invite(&other.0, &secret, NOW)
            .unwrap()
            .device_id,
        owner.1
    );
    assert_eq!(usage(&mut store, &owner.0), before_owner + 256);
    assert_eq!(usage(&mut store, &other.0), before_other + 256);
    store.redeem_contact_invite(&other.0, &secret, NOW).unwrap();
    assert_eq!(usage(&mut store, &owner.0), before_owner + 256);
    assert_eq!(usage(&mut store, &other.0), before_other + 256);
    store.remove_sender(&owner.0, &other.1, NOW).unwrap();
    store.remove_sender(&owner.0, &other.1, NOW).unwrap();
    assert_eq!(usage(&mut store, &owner.0), before_owner);
    assert_eq!(usage(&mut store, &other.0), before_other + 256);
    assert!(matches!(
        store.redeem_contact_invite(&other.0, &secret, NOW),
        Err(StoreError::Forbidden)
    ));
    store
        .revoke_contact_invite(&owner.0, &invite.id, NOW)
        .unwrap();
    assert_eq!(usage(&mut store, &owner.0), before_owner);
}

#[test]
fn schema26_rebuild_counts_retained_contacts_and_recipient_grants_once_including_restore() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("server.db");
    let (mut store, devices) = fixture(&path);
    let owner = &devices[0];
    for index in [21, 22] {
        let invite = store
            .create_contact_invite(
                &owner.0,
                CreateContactInvite {
                    secret: hex(&[index; 32]),
                    expires_at: NOW + 60,
                },
                NOW,
            )
            .unwrap();
        if index == 21 {
            store
                .revoke_contact_invite(&owner.0, &invite.id, NOW)
                .unwrap();
        }
    }
    store.allow_sender(&owner.0, &devices[1].1, NOW).unwrap();
    let account = store.session(&owner.0, NOW).unwrap().account_id;
    drop(store);
    // Synthetic historical fixture: schema26 had these exact tables/rows but
    // omitted their charges. This models migration, not untrusted DB reachability.
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute(
        "UPDATE retained_storage SET bytes=2048 WHERE account_id=?1",
        [&account],
    )
    .unwrap();
    db.execute_batch("DROP TABLE account_passwords; DROP TABLE password_policy; DROP TABLE web_oidc; DROP TABLE web_sessions; DROP TABLE oidc_fallback_ack; DROP TABLE oidc_transition; DROP TABLE account_profiles; DROP TABLE web_owner; DROP TABLE deleted_accounts; ALTER TABLE private_groups DROP COLUMN blocked;").unwrap();
    db.pragma_update(None, "user_version", 26).unwrap();
    drop(db);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(usage(&mut store, &owner.0), 2048 + 2 * 512 + 256);
    assert_eq!(usage(&mut store, &devices[1].0), 2048);
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(usage(&mut store, &owner.0), 2048 + 2 * 512 + 256);
    let restored = directory.path().join("restored.db");
    drop(store);
    Store::restore(&path, &restored).unwrap();
    let db = rusqlite::Connection::open(&restored).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        30
    );
    assert_eq!(
        db.query_row(
            "SELECT bytes FROM retained_storage WHERE account_id=?1",
            [&account],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2048 + 2 * 512 + 256
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM contact_invitations WHERE revoked=1",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}
