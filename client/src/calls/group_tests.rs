use super::*;
use crate::{
    claims::tests::pair,
    connection::tests::{credential, prepare},
    incoming::tests::trust,
};
fn round(clients: &mut [ClientStore], now: u64) {
    for (i, client) in clients.iter_mut().enumerate() {
        for attempt in client.resume_calls_online(now).unwrap() {
            attempt
                .result
                .unwrap_or_else(|e| panic!("call worker {i}: {e:?}"));
        }
    }
    for (i, client) in clients.iter_mut().enumerate() {
        for attempt in client.resume_outbound_online(now).unwrap() {
            attempt
                .result
                .unwrap_or_else(|e| panic!("outbound {i}: {e:?}"));
        }
    }
    for (i, client) in clients.iter_mut().enumerate() {
        for attempt in client.receive_mailbox_online(now).unwrap() {
            attempt
                .result
                .unwrap_or_else(|e| panic!("incoming {i}: {e:?}"));
        }
        client.acknowledge_incoming_online().unwrap();
    }
}
#[test]
fn eight_participants_exchange_authenticated_media_without_conversation_membership() {
    let (dir, fixture, alice, bob, _) = pair();
    super::tests::configure(dir.path());
    let mut clients = vec![alice, bob];
    for n in 2..8 {
        let now = crate::conversations::now();
        let mut server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
        let invitation = server
            .invite(
                sigil_protocol::accounts::InviteRequest {
                    username: format!("guest{n}"),
                    expires_in_seconds: 60,
                },
                now,
            )
            .unwrap();
        let mut client = ClientStore::open(
            &dir.path().join(format!("guest{n}.db")),
            StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
        )
        .unwrap();
        prepare(&mut client, &fixture, &invitation.secret);
        loop {
            match client.enroll_online() {
                Ok(_) => break,
                Err(Error::Network(crate::network::Error::Status {
                    code: 429,
                    retry_after_seconds,
                })) => std::thread::sleep(std::time::Duration::from_secs(
                    retry_after_seconds.unwrap_or(1).clamp(1, 10),
                )),
                Err(e) => panic!("enrollment: {e:?}"),
            }
        }
        let owner = clients[0].connection_session().unwrap().unwrap();
        let guest = client.connection_session().unwrap().unwrap();
        server
            .allow_sender(&credential(&clients[0]), &guest.device_id, now)
            .unwrap();
        server
            .allow_sender(&credential(&client), &owner.device_id, now)
            .unwrap();
        let slot = [n as u8; 32];
        client.prepare_prekey_publication(slot, true, 3600).unwrap();
        client.publish_prekey_online(slot).unwrap();
        clients.push(client);
    }
    let mut invitees = Vec::new();
    let (owner, guests) = clients.split_first_mut().unwrap();
    for guest in guests {
        invitees.push(trust(owner, guest).1);
    }
    let now = crate::conversations::now();
    let id = [92; 32];
    clients[0].create_call(id, now, 3600).unwrap();
    for peer in invitees {
        clients[0].invite_to_call(id, peer, now).unwrap();
    }
    for _ in 0..4 {
        round(&mut clients, now);
    }
    for guest in &mut clients[1..] {
        assert!(guest.call(id, now).unwrap().phase == Phase::Ringing);
        guest.answer_call(id, true, now).unwrap();
    }
    for _ in 0..8 {
        round(&mut clients, now);
        if clients
            .iter()
            .all(|c| load(&c.db, &c.key, &id).unwrap().state.participants.len() == 8)
        {
            break;
        }
    }
    for client in &mut clients {
        assert_eq!(client.call(id, now).unwrap().participants.len(), 8);
    }
    let tracks = Tracks {
        audio: true,
        camera: true,
        screen: true,
    };
    let mut media: Vec<_> = clients
        .iter_mut()
        .map(|c| c.start_call_media(id, tracks, now).unwrap())
        .collect();
    for _ in 0..8 {
        round(&mut clients, now);
        if clients
            .iter()
            .all(|c| load(&c.db, &c.key, &id).unwrap().state.ready.len() == 8)
        {
            break;
        }
    }
    for (c, m) in clients.iter_mut().zip(&mut media) {
        c.refresh_call_media(m, now).unwrap();
    }
    for _ in 0..16 {
        round(&mut clients, now);
        if clients
            .iter()
            .all(|c| load(&c.db, &c.key, &id).unwrap().shares.len() == 8)
        {
            break;
        }
    }
    for i in 0..8 {
        let sender = load(&clients[i].db, &clients[i].key, &id)
            .unwrap()
            .own_id()
            .unwrap();
        for kind in [
            sigil_calls::MediaKind::Audio,
            sigil_calls::MediaKind::Camera,
            sigil_calls::MediaKind::Screen,
        ] {
            let payload = vec![i as u8; 4096];
            let ciphertext = clients[i]
                .seal_call_frame(
                    &mut media[i],
                    kind,
                    123,
                    kind != sigil_calls::MediaKind::Audio,
                    &payload,
                    now,
                )
                .unwrap();
            for j in 0..8 {
                if i != j {
                    assert_eq!(
                        &*clients[j]
                            .open_call_frame(&mut media[j], sender, kind, &ciphertext, now)
                            .unwrap()
                            .data,
                        &payload
                    );
                }
            }
        }
    }
    assert_eq!(
        clients[0]
            .call(id, now)
            .unwrap()
            .participants
            .iter()
            .filter(|p| p.verified)
            .count(),
        8
    );
    assert_eq!(
        clients[1]
            .call(id, now)
            .unwrap()
            .participants
            .iter()
            .filter(|p| p.verified)
            .count(),
        2
    );
    for client in &clients {
        assert_eq!(
            client
                .db
                .query_row("SELECT count(*) FROM groups", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    let owner = load(&clients[0].db, &clients[0].key, &id).unwrap();
    let owner_id = owner.own_id().unwrap();
    let old_key = owner
        .shares
        .iter()
        .find(|s| s.key.context.sender == owner_id)
        .unwrap()
        .key
        .clone();
    let old_frame = clients[0]
        .seal_call_frame(
            &mut media[0],
            sigil_calls::MediaKind::Audio,
            124,
            false,
            b"before removal",
            now,
        )
        .unwrap();
    let removed = load(&clients[7].db, &clients[7].key, &id)
        .unwrap()
        .own
        .unwrap();
    clients[0]
        .block_peer(peer(&removed).unwrap(), true)
        .unwrap();
    clients[0]
        .remove_call_participant(id, removed.member.id, now)
        .unwrap();
    for _ in 0..8 {
        round(&mut clients[..7], now);
        if clients[..7]
            .iter()
            .all(|c| load(&c.db, &c.key, &id).unwrap().state.participants.len() == 7)
        {
            break;
        }
    }
    for (c, m) in clients[..7].iter_mut().zip(&mut media[..7]) {
        c.refresh_call_media(m, now).unwrap();
    }
    for _ in 0..16 {
        round(&mut clients[..7], now);
        if clients[..7]
            .iter()
            .all(|c| load(&c.db, &c.key, &id).unwrap().shares.len() == 7)
        {
            break;
        }
    }
    let fresh = clients[0]
        .seal_call_frame(
            &mut media[0],
            sigil_calls::MediaKind::Audio,
            125,
            false,
            b"after removal",
            now,
        )
        .unwrap();
    assert!(sigil_calls::Receiver::new(&old_key)
        .unwrap()
        .open(&fresh)
        .is_err());
    assert!(clients[7]
        .open_call_frame(
            &mut media[7],
            owner_id,
            sigil_calls::MediaKind::Audio,
            &fresh,
            now
        )
        .is_err());
    for i in 1..7 {
        assert!(clients[i]
            .open_call_frame(
                &mut media[i],
                owner_id,
                sigil_calls::MediaKind::Audio,
                &old_frame,
                now
            )
            .is_err());
        assert_eq!(
            &*clients[i]
                .open_call_frame(
                    &mut media[i],
                    owner_id,
                    sigil_calls::MediaKind::Audio,
                    &fresh,
                    now
                )
                .unwrap()
                .data,
            b"after removal"
        );
    }
    let blocked = load(&clients[6].db, &clients[6].key, &id)
        .unwrap()
        .own
        .unwrap();
    clients[0]
        .block_peer(peer(&blocked).unwrap(), true)
        .unwrap();
    clients[0].leave_call(id, now).unwrap();
    for _ in 0..8 {
        round(&mut clients[..6], now);
        if clients[..6]
            .iter()
            .all(|c| load(&c.db, &c.key, &id).unwrap().phase == Phase::Ended)
        {
            break;
        }
    }
    assert!(clients[..6]
        .iter()
        .all(|c| load(&c.db, &c.key, &id).unwrap().phase == Phase::Ended));
}
