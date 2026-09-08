use super::*;
use crate::{
    auth::random_secret,
    call_config::{Configure, Settings},
    service_config::SecretUpdate,
};
use sigil_calls::{Connect, Layout, Member, Roster};
use sigil_crypto::IdentityKey;
fn setup(path: &std::path::Path) -> (Store, String, String) {
    let mut store = Store::open(path).unwrap();
    store
        .configure(sigil_protocol::Configure {
            expected_revision: 0,
            settings: serde_json::from_value(serde_json::json!({"server_name":"chat.example"}))
                .unwrap(),
        })
        .unwrap();
    let mut credentials = Vec::new();
    for username in ["alice", "bob"] {
        let invitation = store
            .invite(
                sigil_protocol::accounts::InviteRequest {
                    username: username.into(),
                    expires_in_seconds: 60,
                },
                1000,
            )
            .unwrap();
        let credential = random_secret().unwrap();
        store
            .enroll(
                sigil_protocol::accounts::Enrollment {
                    invitation: invitation.secret,
                    device_credential: credential.clone(),
                    device_label: "Synthetic".into(),
                },
                1000,
            )
            .unwrap();
        credentials.push(credential);
    }
    store.configure_calls(config(0)).unwrap();
    (store, credentials.remove(0), credentials.remove(0))
}
fn config(revision: u64) -> Configure {
    Configure {
        expected_revision: revision,
        settings: Some(Settings {
            bind: "127.0.0.1:39900".parse().unwrap(),
            advertised: "127.0.0.1:39900".parse().unwrap(),
            max_calls: 8,
            turn_urls: Vec::new(),
        }),
        turn_secret: SecretUpdate::Clear,
    }
}
fn roster(owner: &IdentityKey, call: u8) -> SignedRoster {
    Roster {
        version: 1,
        call: [call; 32],
        server: "chat.example".into(),
        owner: owner.public_key(),
        created: 1000,
        expires: 2000,
        revision: 0,
        previous: None,
        members: vec![Member::new(owner.public_key())],
        closed: false,
    }
    .sign(owner)
    .unwrap()
}
fn join(roster: &Roster, owner: &IdentityKey, sequence: u64) -> SignedConnect {
    Connect {
        call: roster.call,
        roster: roster.digest().unwrap(),
        participant: Member::new(owner.public_key()).id,
        sequence,
        sdp: "synthetic SDP; storage validates authorization before the media parser".into(),
        layout: Layout {
            uploads: ["a".into(), "v".into(), "s".into()],
            downloads: Vec::new(),
        },
    }
    .sign(owner)
    .unwrap()
}
#[test]
fn ownership_membership_reconnect_and_closure_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let (mut store, alice, bob) = setup(&path);
    let owner = IdentityKey::generate().unwrap();
    let value = roster(&owner, 1);
    store.publish_call(&alice, value.clone(), 1000).unwrap();
    assert!(matches!(
        store.publish_call(&bob, value.clone(), 1000),
        Err(StoreError::Forbidden)
    ));
    let proof = join(&value.roster, &owner, 1);
    assert!(store.admit_call(&proof, 1000).unwrap().fresh);
    assert!(!store.admit_call(&proof, 1000).unwrap().fresh);
    let mut forged = proof.clone();
    forged.signature[0] ^= 1;
    assert!(matches!(
        store.admit_call(&forged, 1000),
        Err(StoreError::Forbidden)
    ));
    let mut altered = proof.request.clone();
    altered.sdp.push('!');
    let altered = altered.sign(&owner).unwrap();
    assert!(matches!(
        store.admit_call(&altered, 1000),
        Err(StoreError::Conflict)
    ));
    let newer = join(&value.roster, &owner, 2);
    assert!(matches!(
        store.admit_call(&newer, 1000),
        Err(StoreError::Busy)
    ));
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert!(!store.admit_call(&proof, 1001).unwrap().fresh);
    assert!(store.admit_call(&newer, 1001).unwrap().fresh);
    assert!(matches!(
        store.admit_call(&proof, 1002),
        Err(StoreError::Conflict)
    ));
    let mut closed = value.roster.clone();
    closed.previous = Some(closed.digest().unwrap());
    closed.revision = 1;
    closed.closed = true;
    let closed = closed.sign(&owner).unwrap();
    store.publish_call(&alice, closed.clone(), 1002).unwrap();
    store.publish_call(&alice, closed, 1002).unwrap();
    assert!(matches!(
        store.admit_call(&newer, 1002),
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        store.publish_call(&alice, value.clone(), 1002),
        Err(StoreError::Conflict)
    ));
    assert!(store.call_snapshot(1002).unwrap().rosters.is_empty());
    store.call_snapshot(2100).unwrap();
    assert!(matches!(
        store.publish_call(&alice, value, 1000),
        Err(StoreError::NotFound)
    ));
}
#[test]
fn roster_updates_require_a_consecutive_head_and_removed_keys_cannot_join() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let (mut store, alice, _) = setup(&path);
    let owner = IdentityKey::generate().unwrap();
    let guest = IdentityKey::generate().unwrap();
    let value = roster(&owner, 2);
    store.publish_call(&alice, value.clone(), 1000).unwrap();
    let mut next = value.roster.clone();
    next.previous = Some(next.digest().unwrap());
    next.revision = 1;
    next.members.push(Member::new(guest.public_key()));
    next.members.sort_by_key(|m| m.id);
    let mut skipped = next.clone();
    skipped.revision = 2;
    assert!(matches!(
        store.publish_call(&alice, skipped.sign(&owner).unwrap(), 1001),
        Err(StoreError::Conflict)
    ));
    store
        .publish_call(&alice, next.clone().sign(&owner).unwrap(), 1001)
        .unwrap();
    assert!(matches!(
        store.admit_call(&join(&value.roster, &owner, 1), 1001),
        Err(StoreError::Conflict)
    ));
    let relay = sigil_calls::RelayRequest::new(&next, &guest, 1001).unwrap();
    store.call_relay(&relay, 1001).unwrap();
    next.previous = Some(next.digest().unwrap());
    next.revision = 2;
    next.members.retain(|m| m.key != guest.public_key());
    store
        .publish_call(&alice, next.sign(&owner).unwrap(), 1002)
        .unwrap();
    assert!(store.call_relay(&relay, 1002).is_err());
    assert!(store.call_relay(&relay, 1001).is_err());
}
#[test]
fn capacities_revocation_disable_and_failed_writes_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let (mut store, alice, bob) = setup(&path);
    let owner = IdentityKey::generate().unwrap();
    for n in 1..=2 {
        store.publish_call(&alice, roster(&owner, n), 1000).unwrap();
    }
    assert!(matches!(
        store.publish_call(&alice, roster(&owner, 3), 1000),
        Err(StoreError::Busy)
    ));
    store.publish_call(&bob, roster(&owner, 3), 1000).unwrap();
    let proof = join(&roster(&owner, 1).roster, &owner, 1);
    store.0.pragma_update(None, "query_only", true).unwrap();
    assert!(store.admit_call(&proof, 1000).is_err());
    store.0.pragma_update(None, "query_only", false).unwrap();
    assert!(store.admit_call(&proof, 1000).unwrap().fresh);
    let device = authorize(&store.0, &alice, 1000).unwrap();
    store
        .0
        .execute("UPDATE devices SET revoked=1 WHERE id=?1", [device])
        .unwrap();
    assert_eq!(store.call_snapshot(1001).unwrap().rosters.len(), 1);
    assert!(store.admit_call(&proof, 1001).is_err());
    store
        .configure_calls(Configure {
            expected_revision: 1,
            settings: None,
            turn_secret: SecretUpdate::Clear,
        })
        .unwrap();
    assert!(store.call_snapshot(1001).unwrap().rosters.is_empty());
    store.configure_calls(config(2)).unwrap();
    assert!(matches!(
        store.publish_call(&bob, roster(&owner, 3), 1001),
        Err(StoreError::Conflict)
    ));
}
#[test]
fn relay_configuration_redacts_secrets_and_rejects_ambiguous_endpoints() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sigil.db");
    let (mut store, _, _) = setup(&path);
    for url in [
        "https://relay.example",
        "turn:relay.example:3478",
        "turns:relay.example:5349?transport=udp",
        "turn:relay.example:3478?transport=tcp&secret=x",
        "turn:user@relay.example:3478?transport=udp",
        "turn:relay.example:0?transport=udp",
    ] {
        let mut request = config(1);
        request
            .settings
            .as_mut()
            .unwrap()
            .turn_urls
            .push(url.into());
        request.turn_secret =
            SecretUpdate::Set("synthetic-secret-32-characters-long".to_owned().into());
        assert!(store.configure_calls(request).is_err());
    }
    let mut request = config(1);
    request
        .settings
        .as_mut()
        .unwrap()
        .turn_urls
        .push("turns:relay.example:5349?transport=tcp".into());
    request.turn_secret =
        SecretUpdate::Set("synthetic-secret-32-characters-long".to_owned().into());
    let public = store.configure_calls(request).unwrap();
    let json = serde_json::to_string(&public).unwrap();
    assert!(!json.contains("synthetic-secret"));
    assert!(public.has_turn_secret);
    let mut retry = config(1);
    retry.settings = public.settings.clone();
    retry.turn_secret = SecretUpdate::Keep;
    assert_eq!(store.configure_calls(retry).unwrap().revision, 2);
    let mut changed = config(2);
    changed.settings = public.settings;
    changed.settings.as_mut().unwrap().turn_urls[0] =
        "turn:another.example:3478?transport=udp".into();
    changed.turn_secret = SecretUpdate::Keep;
    assert!(store.configure_calls(changed).is_err());
}
