use super::*;
fn image(byte: u8) -> Vec<u8> {
    [vec![255, 216], vec![byte; 1000], vec![255, 217]].concat()
}
#[test]
fn profile_transport_cache_and_removal_respect_explicit_shares() {
    let (dir, _fixture, mut alice, mut bob, _) = crate::claims::tests::pair();
    let (_, peer) = crate::incoming::tests::trust(&mut alice, &mut bob);
    let reference = transport::hex(&alice.account_reference().unwrap());
    let picture = image(42);
    alice.mobile_stage_photo(&picture).unwrap();
    assert_eq!(
        alice
            .mobile_profile_image(&reference)
            .unwrap()
            .unwrap()
            .as_slice(),
        picture
    );
    alice.mobile_photo_publish().unwrap();
    assert!(!alice.photo_upload_pending().unwrap());
    assert!(bob.mobile_profile_image(&reference).unwrap().is_none());
    alice.share_peer_profile(peer).unwrap();
    assert_eq!(
        bob.mobile_profile_image(&reference)
            .unwrap()
            .unwrap()
            .as_slice(),
        picture
    );
    let bytes: Vec<u8> = bob
        .db
        .query_row(
            "SELECT image FROM mobile_profiles WHERE id=?1",
            [id(&reference).unwrap().as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!bytes.windows(100).any(|part| part == &picture[2..102]));
    assert!(!std::fs::read(dir.path().join("bob.db"))
        .unwrap()
        .windows(100)
        .any(|part| part == &picture[2..102]));
    alice.mobile_stage_photo(&[]).unwrap();
    alice.mobile_photo_publish().unwrap();
    bob.db.execute("DELETE FROM mobile_profiles", []).unwrap();
    assert!(bob.mobile_profile_image(&reference).unwrap().is_none());
}
#[test]
fn stale_upload_work_cannot_replace_or_delete_a_newer_choice() {
    let (_dir, _fixture, mut alice, _bob, _) = crate::claims::tests::pair();
    alice.mobile_stage_photo(&image(17)).unwrap();
    let mut old = alice.photo_upload().unwrap().unwrap();
    old.expected = Some(0);
    alice.mobile_stage_photo(&image(18)).unwrap();
    let new = alice.photo_upload().unwrap().unwrap();
    assert!(!alice.update_photo_upload(old.id, Some(&old)).unwrap());
    assert!(!alice.update_photo_upload(old.id, None).unwrap());
    assert_eq!(alice.photo_upload().unwrap().unwrap().id, new.id);
    assert!(alice.mobile_stage_photo(&vec![0; MAX_PHOTO + 1]).is_err());
    assert_eq!(alice.photo_upload().unwrap().unwrap().image, new.image);
    alice.db.execute_batch("CREATE TRIGGER fail_photo BEFORE DELETE ON mobile_photo_upload BEGIN SELECT RAISE(ABORT,'synthetic'); END").unwrap();
    assert!(alice.mobile_photo_publish().is_err());
    assert!(alice.photo_upload_pending().unwrap());
    alice.db.execute_batch("DROP TRIGGER fail_photo").unwrap();
    alice.mobile_photo_publish().unwrap();
    assert_eq!(
        alice
            .connected_client()
            .unwrap()
            .profile_photo()
            .unwrap()
            .revision,
        1
    );
    assert!(!alice.photo_upload_pending().unwrap());
}
#[test]
fn cached_profile_rejects_rollback_and_ciphertext_substitution() {
    let (_dir, _fixture, mut alice, _bob, _) = crate::claims::tests::pair();
    let own = alice.account_reference().unwrap();
    let cached = Cached {
        value: ContactProfile {
            profile: sigil_protocol::profile::Profile {
                revision: 2,
                display_name: "Synthetic".into(),
            },
            photo: Default::default(),
        },
        at: conversations::now(),
    };
    alice.cache_profile(own, &cached, None).unwrap();
    let older = Cached {
        value: ContactProfile {
            profile: Default::default(),
            photo: Default::default(),
        },
        at: conversations::now(),
    };
    assert!(matches!(
        alice.cache_profile(own, &older, None),
        Err(Error::Conflict)
    ));
    let mut changed = cached;
    changed.value.profile.display_name = "Different".into();
    assert!(matches!(
        alice.cache_profile(own, &changed, None),
        Err(Error::Conflict)
    ));
    alice
        .db
        .execute("UPDATE mobile_profiles SET id=?1", [[7; 32].as_slice()])
        .unwrap();
    assert!(alice.cached_profile([7; 32]).is_err());
}

#[test]
fn maximum_photo_survives_restart_and_rejects_reordered_storage_chunks() {
    let (dir, _fixture, mut alice, _bob, _) = crate::claims::tests::pair();
    let mut image = vec![73; MAX_PHOTO];
    image[..2].copy_from_slice(&[255, 216]);
    image[MAX_PHOTO - 2..].copy_from_slice(&[255, 217]);
    let reference = transport::hex(&alice.account_reference().unwrap());
    alice.mobile_stage_photo(&image).unwrap();
    drop(alice);
    let mut alice = ClientStore::open(
        &dir.path().join("alice.db"),
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap();
    assert_eq!(
        alice
            .mobile_profile_image(&reference)
            .unwrap()
            .unwrap()
            .as_slice(),
        image
    );
    alice.mobile_photo_publish().unwrap();
    assert_eq!(
        alice
            .mobile_profile_image(&reference)
            .unwrap()
            .unwrap()
            .as_slice(),
        image
    );
    let mut sealed: Vec<u8> = alice
        .db
        .query_row(
            "SELECT image FROM mobile_profiles WHERE id=?1",
            [id(&reference).unwrap().as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    let (first, second) = sealed[40..].split_at_mut(65536 + 36);
    first.swap_with_slice(second);
    alice
        .db
        .execute("UPDATE mobile_profiles SET image=?1", [sealed])
        .unwrap();
    assert!(alice.mobile_profile_image(&reference).is_err());
}

#[test]
fn schema_74_photo_migration_preserves_pending_choices_and_rolls_back_on_failure() {
    let (dir, _fixture, mut alice, _bob, _) = crate::claims::tests::pair();
    let reference = transport::hex(&alice.account_reference().unwrap());
    let picture = image(93);
    alice.mobile_stage_photo(&picture).unwrap();
    alice.mobile_photo_publish().unwrap();
    alice.mobile_stage_photo(&image(94)).unwrap();
    let pending = alice.photo_upload().unwrap().unwrap();
    let old_image = alice
        .key
        .seal(
            &picture,
            &alice
                .profile_aad(id(&reference).unwrap(), b"image")
                .unwrap(),
        )
        .unwrap();
    let old_upload = alice
        .key
        .seal(
            &serde_json::to_vec(&pending).unwrap(),
            &alice.profile_aad([0; 32], b"upload").unwrap(),
        )
        .unwrap();
    alice
        .db
        .execute("UPDATE mobile_profiles SET image=?1", [&old_image])
        .unwrap();
    alice
        .db
        .execute("UPDATE mobile_photo_upload SET state=?1", [&old_upload])
        .unwrap();
    alice.db.execute_batch("PRAGMA user_version=74; CREATE TRIGGER fail_photo_migration BEFORE UPDATE ON mobile_photo_upload BEGIN SELECT RAISE(ABORT,'synthetic'); END").unwrap();
    drop(alice);
    let path = dir.path().join("alice.db");
    let open = || {
        ClientStore::open(
            &path,
            StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
        )
    };
    assert!(open().is_err());
    let db = Connection::open(&path).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        74
    );
    assert_eq!(
        db.query_row("SELECT image FROM mobile_profiles", [], |r| r
            .get::<_, Vec<u8>>(0))
            .unwrap(),
        old_image
    );
    db.execute_batch("DROP TRIGGER fail_photo_migration")
        .unwrap();
    drop(db);
    let mut alice = open().unwrap();
    assert_eq!(alice.photo_upload().unwrap().unwrap().id, pending.id);
    assert_eq!(
        alice
            .mobile_profile_image(&reference)
            .unwrap()
            .unwrap()
            .as_slice(),
        image(94)
    );
    alice.mobile_photo_publish().unwrap();
    assert_eq!(
        alice
            .mobile_profile_image(&reference)
            .unwrap()
            .unwrap()
            .as_slice(),
        image(94)
    );
}
