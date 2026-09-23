use super::*;
use crate::auth::random_secret;
use sigil_crypto::IdentityKey;
use sigil_protocol::{
    accounts::{Enrollment, Session},
    device::Binding,
};

fn unhex32(value: &str) -> [u8; 32] {
    unhex(value, 32).unwrap().try_into().unwrap()
}
fn statement(session: &Session, identity: &IdentityKey) -> SignedBinding {
    let binding = Binding {
        server: "chat.example".into(),
        username: session.address[1..session.address.find(':').unwrap()].into(),
        account: unhex32(&session.account_id),
        device: unhex32(&session.device_id),
        identity: identity.public_key(),
    };
    let signature = identity.sign(&binding.signing_bytes().unwrap()).unwrap();
    SignedBinding { binding, signature }
}
fn endorse(key: &IdentityKey, signed: &SignedBinding) -> String {
    let fingerprint = sigil_crypto::link::fingerprint(&signed.binding).unwrap();
    hex(&sigil_crypto::account::endorse(key, &fingerprint).unwrap())
}
fn publish(store: &mut Store, token: &str, now: u64) -> (IdentityKey, SignedBinding) {
    let session = store.session(token, now).unwrap();
    let identity = IdentityKey::generate().unwrap();
    let signed = statement(&session, &identity);
    store
        .publish_device_binding(
            token,
            Statement {
                statement: hex(&signed.to_bytes().unwrap()),
            },
            now,
        )
        .unwrap();
    let key = IdentityKey::generate().unwrap();
    store
        .publish_account_key(
            token,
            PublishAccountKey {
                public: hex(&key.public_key()),
                bundle: Some("ab".repeat(76)),
                endorsement: endorse(&key, &signed),
            },
            now,
        )
        .unwrap();
    (key, signed)
}
fn sign_in(store: &mut Store, account: &str, token: &str, now: u64) -> Session {
    let invitation = store.invite_reauthorization(account, 60, now).unwrap();
    store
        .reauthorize(
            Enrollment {
                invitation: invitation.secret,
                device_credential: token.into(),
                device_label: "Synthetic laptop".into(),
            },
            now,
        )
        .unwrap()
}

#[test]
fn account_key_is_pinned_once_and_listed_with_endorsements() {
    let (_dir, mut store, alice, bob, now) = crate::admin::tests::setup();
    let (key, signed) = publish(&mut store, &alice, now);
    let other = IdentityKey::generate().unwrap();
    assert!(matches!(
        store.publish_account_key(
            &alice,
            PublishAccountKey {
                public: hex(&other.public_key()),
                bundle: None,
                endorsement: endorse(&other, &signed),
            },
            now
        ),
        Err(StoreError::Conflict)
    ));
    assert!(store
        .publish_account_key(
            &alice,
            PublishAccountKey {
                public: hex(&key.public_key()),
                bundle: None,
                endorsement: endorse(&other, &signed),
            },
            now
        )
        .is_err());
    let directory = store.contact_directory(&bob, "alice", now).unwrap();
    assert_eq!(directory.account_key, Some(hex(&key.public_key())));
    assert_eq!(directory.endorsements.len(), 1);
    let fingerprint = sigil_crypto::link::fingerprint(&signed.binding).unwrap();
    sigil_crypto::account::verify_endorsement(&key.public_key(), &fingerprint, &unhex(&directory.endorsements[0], 64).unwrap()).unwrap();
    assert_eq!(store.account_key(&alice, now).unwrap().bundle, Some("ab".repeat(76)));
}

#[test]
fn signing_in_waits_for_an_endorsement_and_never_revokes_other_devices() {
    let (_dir, mut store, alice, _, now) = crate::admin::tests::setup();
    let (key, _) = publish(&mut store, &alice, now);
    let account = store.session(&alice, now).unwrap().account_id;
    let token = random_secret().unwrap();
    let pending = sign_in(&mut store, &account, &token, now);
    assert!(pending.pending);
    assert!(store.session(&token, now).is_err());
    assert!(store.prekey_inventory(&token, now).is_err());
    let state = store.pending_state(&token, now).unwrap();
    assert_eq!(state.account_key.unwrap().public, hex(&key.public_key()));
    let identity = IdentityKey::generate().unwrap();
    let signed = statement(&pending, &identity);
    let forged = IdentityKey::generate().unwrap();
    let request = |endorsement: String| Activate {
        statement: hex(&signed.to_bytes().unwrap()),
        endorsement,
    };
    assert!(matches!(
        store.activate_pending(&token, request(endorse(&forged, &signed)), now),
        Err(StoreError::Unauthorized)
    ));
    let active = store
        .activate_pending(&token, request(endorse(&key, &signed)), now)
        .unwrap();
    assert!(!active.pending);
    assert_eq!(active.device_id, pending.device_id);
    assert!(store.session(&token, now).is_ok());
    assert!(store.session(&alice, now).is_ok());
    assert!(store.pending_state(&token, now).is_err());
}

#[test]
fn reset_replaces_the_key_and_signs_out_every_other_device() {
    let (_dir, mut store, alice, bob, now) = crate::admin::tests::setup();
    let (old, _) = publish(&mut store, &alice, now);
    let account = store.session(&alice, now).unwrap().account_id;
    store
        .put_recovery_wrap(
            &alice,
            RecoveryWrap {
                id: "01".repeat(16),
                salt: "02".repeat(32),
                wrapped: "03".repeat(68),
                label: "Synthetic phone".into(),
                created: 0,
            },
            now,
        )
        .unwrap();
    let token = random_secret().unwrap();
    let pending = sign_in(&mut store, &account, &token, now);
    assert_eq!(store.pending_state(&token, now).unwrap().wraps.len(), 1);
    let identity = IdentityKey::generate().unwrap();
    let signed = statement(&pending, &identity);
    let key = IdentityKey::generate().unwrap();
    let session = store
        .reset_identity(
            &token,
            ResetIdentity {
                statement: hex(&signed.to_bytes().unwrap()),
                key: PublishAccountKey {
                    public: hex(&key.public_key()),
                    bundle: None,
                    endorsement: endorse(&key, &signed),
                },
            },
            now,
        )
        .unwrap();
    assert!(!session.pending);
    assert!(store.session(&alice, now).is_err());
    assert!(store.session(&bob, now).is_ok());
    assert!(store.recovery_wraps(&token, now).unwrap().wraps.is_empty());
    let directory = store.contact_directory(&bob, "alice", now).unwrap();
    assert_eq!(directory.account_key, Some(hex(&key.public_key())));
    assert_ne!(directory.account_key, Some(hex(&old.public_key())));
    assert_eq!(directory.bindings.len(), 1);
}

#[test]
fn recovery_wraps_are_bounded_account_scoped_and_removable() {
    let (_dir, mut store, alice, bob, now) = crate::admin::tests::setup();
    let wrap = |n: u8| RecoveryWrap {
        id: format!("{n:02x}").repeat(16),
        salt: "02".repeat(32),
        wrapped: "03".repeat(68),
        label: "Synthetic".into(),
        created: 0,
    };
    for n in 0..MAX_RECOVERY_WRAPS as u8 {
        store.put_recovery_wrap(&alice, wrap(n), now).unwrap();
    }
    assert!(matches!(
        store.put_recovery_wrap(&alice, wrap(200), now),
        Err(StoreError::Busy)
    ));
    store.put_recovery_wrap(&alice, wrap(0), now).unwrap();
    assert!(store.recovery_wraps(&bob, now).unwrap().wraps.is_empty());
    assert!(store.delete_recovery_wrap(&bob, &wrap(0).id, now).is_err());
    store.delete_recovery_wrap(&alice, &wrap(0).id, now).unwrap();
    assert_eq!(
        store.recovery_wraps(&alice, now).unwrap().wraps.len(),
        MAX_RECOVERY_WRAPS - 1
    );
    let mut bad = wrap(1);
    bad.salt = "02".repeat(16);
    assert!(store.put_recovery_wrap(&alice, bad, now).is_err());
}
