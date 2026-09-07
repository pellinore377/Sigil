use super::*;
use crate::connection::tests::{prepare, setup};
use sigil_crypto::{storage::StorageKey, Secret32};
fn open(path: &std::path::Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
#[test]
fn offer_restart_consent_binding_and_atomic_cancellation_preserve_exact_state() {
    let (dir, fixture, invitation, now) = setup();
    let mut sponsor = open(&dir.path().join("sponsor.db"));
    prepare(&mut sponsor, &fixture, &invitation.secret);
    sponsor.enroll_online().unwrap();
    let sponsor_bytes = sponsor.own_device_binding().unwrap();
    let path = dir.path().join("joining.db");
    let mut joining = open(&path);
    let attempt = [1; 32];
    let offer = joining.prepare_device_link_offer(attempt, now).unwrap();
    let encoded = offer.to_bytes().unwrap();
    assert_eq!(Offer::from_bytes(&encoded).unwrap(), offer);
    for end in 0..encoded.len() {
        assert!(Offer::from_bytes(&encoded[..end]).is_err());
    }
    assert!(Offer::from_bytes(&[encoded.as_slice(), &[0]].concat()).is_err());
    drop(joining);
    let mut joining = open(&path);
    assert_eq!(
        joining.prepare_device_link_offer(attempt, now + 1).unwrap(),
        offer
    );
    assert!(matches!(
        joining.prepare_device_link_offer(attempt, offer.expires_at),
        Err(Error::Expired)
    ));
    assert!(joining.prepare_device_link_offer(attempt, now - 1).is_err());
    assert!(sponsor.prepare_device_link_offer(attempt, now).is_err());
    let raw = joining
        .sign_joining_device_binding(&sponsor_bytes, offer.device)
        .unwrap();
    let transcript = Transcript {
        sponsor: peers::device_fingerprint(&sponsor_bytes).unwrap(),
        joining: peers::device_fingerprint(&raw).unwrap(),
        sponsor_challenge: [2; 32],
        joining_challenge: offer.challenge,
        provisioning_key: offer.provisioning_key,
        credential_commitment: offer.credential_commitment,
        created_at: now,
        expires_at: offer.expires_at,
    };
    let digest = confirmation(&transcript).unwrap();
    let mut wrong = transcript.clone();
    wrong.credential_commitment = [9; 32];
    assert!(joining
        .approve_device_link_offer(
            attempt,
            &wrong,
            &sponsor_bytes,
            confirmation(&wrong).unwrap(),
            now
        )
        .is_err());
    let consent = joining
        .approve_device_link_offer(attempt, &transcript, &sponsor_bytes, digest, now)
        .unwrap();
    assert_eq!(consent.0, raw);
    assert_eq!(
        joining
            .approve_device_link_offer(attempt, &transcript, &sponsor_bytes, digest, now)
            .unwrap(),
        consent
    );
    let own = joining.identity().unwrap();
    let id = journal::reference(&own, 3, &attempt);
    let before = journal::read(&joining.db, &joining.key, &own, &id)
        .unwrap()
        .unwrap();
    let pending = decode(&joining.key, &own, &attempt, &before).unwrap();
    let credential = &pending.secrets.as_ref().unwrap().1;
    assert!(!encoded
        .windows(credential.len())
        .any(|window| window == credential.as_bytes()));
    assert_eq!(
        <Id>::from(Sha256::digest(credential.as_bytes())),
        offer.credential_commitment
    );
    // Fail on the offer write, after consent cancellation has been attempted.
    joining.db.execute_batch("CREATE TRIGGER fail_offer BEFORE UPDATE ON device_link_records WHEN length(NEW.state)=221 BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(joining.cancel_device_link_offer(attempt).is_err());
    joining
        .db
        .execute_batch("DROP TRIGGER fail_offer;")
        .unwrap();
    assert_eq!(
        joining
            .approve_device_link_offer(attempt, &transcript, &sponsor_bytes, digest, now)
            .unwrap(),
        consent
    );
    assert_eq!(
        journal::read(&joining.db, &joining.key, &own, &id)
            .unwrap()
            .unwrap()
            .as_slice(),
        before.as_slice()
    );
    assert!(joining.cancel_device_link_offer(attempt).unwrap());
    assert!(!joining.cancel_device_link_offer(attempt).unwrap());
    drop(joining);
    let mut joining = open(&path);
    assert!(matches!(
        joining.prepare_device_link_offer(attempt, now),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        joining.sign_device_link_consent(&transcript, &sponsor_bytes, &raw, digest, now),
        Err(Error::Cancelled)
    ));
    assert!(joining
        .approve_device_link_offer(attempt, &transcript, &sponsor_bytes, digest, now)
        .is_err());
    let tombstone = journal::read(&joining.db, &joining.key, &own, &id)
        .unwrap()
        .unwrap();
    assert_eq!(tombstone.len(), 185);
    assert!(decode(&joining.key, &own, &attempt, &tombstone)
        .unwrap()
        .secrets
        .is_none());
    let next = joining.prepare_device_link_offer([3; 32], now).unwrap();
    assert_ne!(next.device, offer.device);
    assert_ne!(next.challenge, offer.challenge);
    assert_ne!(next.provisioning_key, offer.provisioning_key);
    assert_ne!(next.credential_commitment, offer.credential_commitment);
}
#[test]
fn failed_offer_creation_and_corrupt_cached_offer_never_publish_replacement_secrets() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = open(&dir.path().join("joining.db"));
    store.db.execute_batch("CREATE TRIGGER fail_offer BEFORE INSERT ON device_link_records BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(store.prepare_device_link_offer([1; 32], 1000).is_err());
    assert_eq!(
        store
            .db
            .query_row("SELECT count(*) FROM device_link_records", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    store.db.execute_batch("DROP TRIGGER fail_offer;").unwrap();
    store.prepare_device_link_offer([1; 32], 1000).unwrap();
    store
        .db
        .execute(
            "UPDATE device_link_records SET state=zeroblob(length(state))",
            [],
        )
        .unwrap();
    assert!(store.prepare_device_link_offer([1; 32], 1000).is_err());
    assert!(store.cancel_device_link_offer([1; 32]).is_err());
    assert_eq!(
        store
            .db
            .query_row("SELECT count(*) FROM device_link_records", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
