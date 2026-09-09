use super::*;
use crate::connection::tests::{credential, prepare, setup};
use crate::network::tests::CA;
use sigil_crypto::{storage::StorageKey, Secret32};
fn open(path: &std::path::Path) -> ClientStore {
    ClientStore::open(
        path,
        StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
    )
    .unwrap()
}
#[test]
fn qr_linking_survives_each_restart_and_lost_local_commits_then_exchanges_encrypted_messages() {
    let (dir, fixture, invitation, now) = setup();
    let sp = dir.path().join("sponsor.db");
    let jp = dir.path().join("joining.db");
    let mut sponsor = open(&sp);
    prepare(&mut sponsor, &fixture, &invitation.secret);
    sponsor.enroll_online().unwrap();
    let mut joining = open(&jp);
    let offer = joining.prepare_device_link_offer([1; 32], now).unwrap();
    let (proposal, digest) = sponsor
        .prepare_sponsored_link([2; 32], &offer_qr(&offer).unwrap(), now)
        .unwrap();
    let mut damaged = proposal.clone().into_bytes();
    let last = damaged.last_mut().unwrap();
    *last = if *last == b'0' { b'1' } else { b'0' };
    assert!(joining
        .accept_link_proposal([1; 32], &String::from_utf8(damaged).unwrap(), now)
        .is_err());
    assert!(joining
        .accept_link_proposal([1; 32], &proposal, now + 600)
        .is_err());
    drop(sponsor);
    let mut sponsor = open(&sp);
    assert_eq!(
        sponsor
            .prepare_sponsored_link([2; 32], &offer_qr(&offer).unwrap(), now)
            .unwrap(),
        (proposal.clone(), digest)
    );
    assert_eq!(
        joining
            .accept_link_proposal([1; 32], &proposal, now)
            .unwrap(),
        digest
    );
    let (substitute, _) = sponsor
        .prepare_sponsored_link([3; 32], &offer_qr(&offer).unwrap(), now)
        .unwrap();
    assert!(joining
        .accept_link_proposal([1; 32], &substitute, now)
        .is_err());
    drop(joining);
    let mut joining = open(&jp);
    assert_eq!(
        joining
            .accept_link_proposal([1; 32], &proposal, now)
            .unwrap(),
        digest
    );
    assert_eq!(
        emoji_confirmation(digest),
        emoji_confirmation(
            joining
                .accept_link_proposal([1; 32], &proposal, now)
                .unwrap()
        )
    );
    assert!(joining
        .confirm_link_proposal([1; 32], [0; 32], now)
        .is_err());
    let response = joining.confirm_link_proposal([1; 32], digest, now).unwrap();
    assert!(sponsor
        .confirm_sponsored_link([3; 32], &response, digest, now)
        .is_err());
    drop(joining);
    let mut joining = open(&jp);
    assert_eq!(
        joining.confirm_link_proposal([1; 32], digest, now).unwrap(),
        response
    );
    assert!(sponsor
        .confirm_sponsored_link([2; 32], &response, [0; 32], now)
        .is_err());
    let proof = sponsor
        .confirm_sponsored_link([2; 32], &response, digest, now)
        .unwrap();
    drop(sponsor);
    let mut sponsor = open(&sp);
    assert_eq!(
        sponsor
            .confirm_sponsored_link([2; 32], &response, digest, now)
            .unwrap(),
        proof
    );
    sponsor.db.execute_batch("CREATE TRIGGER fail_receipt BEFORE UPDATE ON device_link_records BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(sponsor.authorize_sponsored_link_online([2; 32]).is_err());
    sponsor
        .db
        .execute_batch("DROP TRIGGER fail_receipt;")
        .unwrap();
    let mut server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
    assert_eq!(
        server
            .list_devices(&credential(&sponsor), None, now)
            .unwrap()
            .devices
            .len(),
        2
    );
    drop(sponsor);
    let mut sponsor = open(&sp);
    let session = sponsor.authorize_sponsored_link_online([2; 32]).unwrap();
    assert_eq!(
        sponsor.authorize_sponsored_link_online([2; 32]).unwrap(),
        session
    );
    joining.db.execute_batch("CREATE TRIGGER fail_finish BEFORE INSERT ON connection BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(joining
        .finish_device_link_online([1; 32], fixture.port(), &[CA.to_vec()])
        .is_err());
    joining
        .db
        .execute_batch("DROP TRIGGER fail_finish;")
        .unwrap();
    drop(joining);
    let mut joining = open(&jp);
    assert_eq!(
        joining
            .finish_device_link_online([1; 32], fixture.port(), &[CA.to_vec()])
            .unwrap(),
        session
    );
    assert_eq!(
        joining
            .finish_device_link_online([1; 32], fixture.port(), &[CA.to_vec()])
            .unwrap(),
        session
    );
    assert_eq!(
        peers::parse(&joining.own_device_binding().unwrap()).unwrap(),
        proof.joining
    );
    let a = peers::reference(&proof.sponsor.binding.server, &proof.sponsor.binding.device);
    let b = peers::reference(&proof.joining.binding.server, &proof.joining.binding.device);
    assert!(sponsor.peer(b).unwrap().trusted && joining.peer(a).unwrap().trusted);
    assert_ne!(sponsor.identity().unwrap(), joining.identity().unwrap());
    joining
        .prepare_prekey_publication([40; 32], true, 3600)
        .unwrap();
    joining.publish_prekey_online([40; 32]).unwrap();
    crate::incoming::tests::start(&mut sponsor, b, now);
    let received = joining
        .accept_delivery(&crate::incoming::tests::next(&joining))
        .unwrap();
    assert!(!received.duplicate);
    let (reply_session, _) = joining
        .send_peer_text(a, [41; 32], "linked reply", now, now)
        .unwrap();
    joining.send_pending_online(reply_session, now).unwrap();
    sponsor
        .accept_delivery(&crate::incoming::tests::next(&sponsor))
        .unwrap();
    assert!(sponsor.session_peer_confirmed([3; 32]).unwrap());
    let request = || sigil_protocol::link::Authorization {
        proof: crate::transport::hex(&proof.to_bytes().unwrap()),
    };
    sponsor.cancel_sponsored_link_online([2; 32]).unwrap();
    sponsor.cancel_sponsored_link_online([2; 32]).unwrap();
    assert!(sponsor.authorize_sponsored_link_online([2; 32]).is_err());
    assert!(!sponsor.peer(b).unwrap().trusted);
    assert!(server
        .authorize_device_link(&credential(&sponsor), request(), now)
        .is_err());
    assert!(joining.devices_online(None).is_err());
}
