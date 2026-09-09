use super::*;
fn upload(revision: u64, byte: u8) -> SetPhoto {
    SetPhoto {
        revision,
        photo: format!("ffd8{}ffd9", format!("{byte:02x}").repeat(1000)),
    }
}
fn retained(store: &Store, account: &str) -> u64 {
    store
        .0
        .query_row(
            "SELECT bytes FROM retained_storage WHERE account_id=?1",
            [account],
            |r| unsigned(r, 0),
        )
        .unwrap()
}
#[test]
fn photos_need_explicit_sharing_and_blocks_revoke_it() {
    let (_dir, mut store, alice, bob, now) = crate::admin::tests::setup();
    let a = store.session(&alice, now).unwrap();
    let b = store.session(&bob, now).unwrap();
    let photo = store.set_profile_photo(&alice, upload(0, 7), now).unwrap();
    store.allow_sender(&alice, &b.device_id, now).unwrap();
    assert!(matches!(
        store.contact_profile(&bob, &a.account_id, now),
        Err(StoreError::Forbidden)
    ));
    assert!(matches!(
        store.contact_photo(&bob, &a.account_id, photo.hash.as_ref().unwrap(), now),
        Err(StoreError::Forbidden)
    ));
    let share = ShareProfile {
        server: "chat.example".into(),
        account: b.account_id.clone(),
        allowed: true,
    };
    store.share_profile(&alice, share.clone(), now).unwrap();
    let quota = retained(&store, &a.account_id);
    store.share_profile(&alice, share.clone(), now).unwrap();
    assert_eq!(retained(&store, &a.account_id), quota);
    assert_eq!(
        store
            .contact_profile(&bob, &a.account_id, now)
            .unwrap()
            .photo,
        photo
    );
    assert_eq!(
        store
            .contact_photo(&bob, &a.account_id, photo.hash.as_ref().unwrap(), now)
            .unwrap()
            .len(),
        1004
    );
    assert!(contact(&store.0, &a.account_id, "other.example", &b.account_id).is_err());
    assert!(contact(&store.0, &a.account_id, "chat.example", &"00".repeat(32)).is_err());
    assert!(store
        .contact_photo(&bob, &a.account_id, &"00".repeat(32), now)
        .is_err());
    store
        .block_contact_requests(
            &alice,
            sigil_protocol::contacts::BlockContact {
                server: "chat.example".into(),
                account: b.account_id.clone(),
                blocked: true,
            },
            now,
        )
        .unwrap();
    assert!(store.contact_profile(&bob, &a.account_id, now).is_err());
    assert!(store.share_profile(&alice, share.clone(), now).is_err());
    store
        .block_contact_requests(
            &alice,
            sigil_protocol::contacts::BlockContact {
                server: "chat.example".into(),
                account: b.account_id,
                blocked: false,
            },
            now,
        )
        .unwrap();
    assert!(store.contact_profile(&bob, &a.account_id, now).is_err());
    crate::storage_budget::rebuild(&store.0).unwrap();
    assert_eq!(retained(&store, &a.account_id), quota - RESERVATION);
}
#[test]
fn photo_revision_retry_removal_and_storage_failure_are_atomic() {
    let (dir, mut store, alice, _bob, now) = crate::admin::tests::setup();
    let account = store.session(&alice, now).unwrap().account_id;
    let before = retained(&store, &account);
    let first = store.set_profile_photo(&alice, upload(0, 7), now).unwrap();
    assert_eq!(
        store.set_profile_photo(&alice, upload(0, 7), now).unwrap(),
        first
    );
    assert!(matches!(
        store.set_profile_photo(&alice, upload(0, 8), now),
        Err(StoreError::Conflict)
    ));
    assert_eq!(retained(&store, &account), before + RESERVATION + 1004);
    store.0.execute_batch("CREATE TRIGGER fail_photo BEFORE UPDATE ON profile_photos BEGIN SELECT RAISE(ABORT,'synthetic'); END").unwrap();
    assert!(store
        .set_profile_photo(
            &alice,
            SetPhoto {
                revision: 1,
                photo: String::new()
            },
            now
        )
        .is_err());
    assert_eq!(store.profile_photo(&alice, now).unwrap(), first);
    assert_eq!(retained(&store, &account), before + RESERVATION + 1004);
    store.0.execute_batch("DROP TRIGGER fail_photo").unwrap();
    let cleared = store
        .set_profile_photo(
            &alice,
            SetPhoto {
                revision: 1,
                photo: String::new(),
            },
            now,
        )
        .unwrap();
    assert_eq!(cleared.revision, 2);
    assert!(cleared.hash.is_none());
    assert_eq!(retained(&store, &account), before + RESERVATION);
    assert!(store
        .contact_photo(&alice, &account, first.hash.as_ref().unwrap(), now)
        .is_err());
    drop(store);
    let store = Store::open(&dir.path().join("sigil.db")).unwrap();
    assert_eq!(store.profile_photo(&alice, now).unwrap(), cleared);
}

#[test]
fn restore_keeps_photos_but_clears_shares_and_deletion_refunds_storage() {
    let (dir, mut store, alice, bob, now) = crate::admin::tests::setup();
    let a = store.session(&alice, now).unwrap().account_id;
    let b = store.session(&bob, now).unwrap().account_id;
    let before = retained(&store, &a);
    store.set_profile_photo(&alice, upload(0, 19), now).unwrap();
    store
        .share_profile(
            &alice,
            ShareProfile {
                server: "chat.example".into(),
                account: b,
                allowed: true,
            },
            now,
        )
        .unwrap();
    let backup = dir.path().join("backup.db");
    let restored = dir.path().join("restored.db");
    store.backup(&backup).unwrap();
    Store::restore(&backup, &restored).unwrap();
    let restored = Store::open(&restored).unwrap();
    assert_eq!(
        restored
            .0
            .query_row("SELECT count(*) FROM profile_shares", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(raw(&restored.0, &a).unwrap().0, 1);
    assert_eq!(retained(&restored, &a), before + RESERVATION + 1004);
    let tx = store
        .0
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    delete_account(&tx, &a).unwrap();
    tx.commit().unwrap();
    assert_eq!(retained(&store, &a), before);
    assert_eq!(store.profile_photo(&alice, now).unwrap(), Photo::default());
}
