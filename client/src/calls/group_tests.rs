use super::*;
use crate::{
    claims::tests::pair,
    connection::tests::{credential, prepare},
    incoming::tests::trust,
};
fn retry<T>(mut work: impl FnMut() -> Result<T, Error>) -> T {
    for _ in 0..8 {
        match work() {
            Ok(value) => return value,
            Err(Error::Network(crate::network::Error::Status { code: 429, retry_after_seconds: Some(wait) })) if wait <= 5 => std::thread::sleep(std::time::Duration::from_secs(wait)),
            Err(error) => panic!("call fixture: {error:?}"),
        }
    }
    panic!("call fixture exhausted rate-limit retries")
}
fn round(clients: &mut [ClientStore], now: u64) {
    for client in clients.iter_mut() {
        retry(|| client.replenish_prekey_online());
    }
    for client in clients.iter_mut() {
        retry(|| client.resume_calls_online(now)?.into_iter().try_for_each(|a| a.result));
    }
    for client in clients.iter_mut() {
        retry(|| client.resume_outbound_online(now)?.into_iter().try_for_each(|a| a.result.map(|_| ())));
    }
    for (i, client) in clients.iter_mut().enumerate() {
        for attempt in retry(|| client.receive_mailbox_online(now)) {
            attempt
                .result
                .unwrap_or_else(|e| panic!("incoming {i}: {e:?}"));
        }
        retry(|| client.acknowledge_incoming_online());
    }
}
#[test]
fn eight_participants_exchange_authenticated_media_without_conversation_membership() {
    group_call(false, false);
}
#[test]
fn group_creator_can_leave_and_unverified_participants_continue_with_new_keys() {
    group_call(true, false);
}
#[test]
fn group_handoff_cancels_pending_invitations_and_answers() {
    group_call(true, true);
}
fn group_call(continuation: bool, pending: bool) {
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
    if continuation {
        clients[0].create_group_call(id, now, 3600).unwrap();
    } else {
        clients[0].create_call(id, now, 3600).unwrap();
    }
    for peer in invitees {
        clients[0].invite_to_call(id, peer, now).unwrap();
    }
    for _ in 0..4 {
        round(&mut clients, now);
    }
    let count = if pending { 6 } else { 8 };
    for guest in &mut clients[1..count] {
        assert!(guest.call(id, now).unwrap().phase == Phase::Ringing);
        guest.answer_call(id, true, now).unwrap();
    }
    for _ in 0..8 {
        round(&mut clients, now);
        if clients
            .iter()
            .take(count)
            .all(|c| load(&c.db, &c.key, &id).unwrap().state.participants.len() == count)
        {
            break;
        }
    }
    for client in &mut clients[..count] {
        assert_eq!(client.call(id, now).unwrap().participants.len(), count);
    }
    if pending {
        clients[7].answer_call(id, true, now).unwrap();
        clients[0].leave_call(id, now).unwrap();
        for _ in 0..16 {
            round(&mut clients, now);
        }
        for client in &mut clients[6..] {
            assert!(client.call(id, now).unwrap().phase == Phase::Declined);
            let record = load(&client.db, &client.key, &id).unwrap();
            assert!(record.secret.is_none() && record.shares.is_empty());
        }
        for client in &mut clients[1..6] {
            let call = client.call(id, now).unwrap();
            assert!(call.phase == Phase::Active);
            assert_eq!(call.participants.len(), 5);
        }
        return;
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
    if !continuation {
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
        }
        assert!(clients[..6]
            .iter()
            .all(|c| load(&c.db, &c.key, &id).unwrap().phase == Phase::Ended));
        return;
    }
    let speaker = load(&clients[1].db, &clients[1].key, &id)
        .unwrap()
        .own_id()
        .unwrap();
    let earlier_key = load(&clients[0].db, &clients[0].key, &id)
        .unwrap()
        .shares
        .iter()
        .find(|share| share.key.context.sender == speaker)
        .unwrap()
        .key
        .clone();
    let earlier = clients[1]
        .seal_call_frame(
            &mut media[1],
            sigil_calls::MediaKind::Audio,
            126,
            false,
            b"before creator leaves",
            now,
        )
        .unwrap();
    assert_eq!(
        &*sigil_calls::Receiver::new(&earlier_key)
            .unwrap()
            .open(&earlier)
            .unwrap()
            .data,
        b"before creator leaves"
    );
    clients[0].db.execute_batch("CREATE TRIGGER fail_handoff BEFORE INSERT ON call_jobs BEGIN SELECT RAISE(ABORT,'synthetic storage failure'); END;").unwrap();
    assert!(matches!(
        clients[0].leave_call(id, now),
        Err(Error::Storage(_))
    ));
    let unchanged = load(&clients[0].db, &clients[0].key, &id).unwrap();
    assert!(
        unchanged.phase == Phase::Active
            && unchanged.transfer.is_none()
            && unchanged.secret.is_some()
    );
    clients[0]
        .db
        .execute_batch("DROP TRIGGER fail_handoff;")
        .unwrap();
    clients[0].leave_call(id, now).unwrap();
    for _ in 0..24 {
        round(&mut clients[..7], now);
        if clients[1..7].iter().all(|c| {
            let record = load(&c.db, &c.key, &id).unwrap();
            record.phase == Phase::Active
                && record.state.participants.len() == 6
                && record.transfer.is_none()
        }) {
            break;
        }
    }
    let departed = load(&clients[0].db, &clients[0].key, &id).unwrap();
    assert!(
        departed.phase == Phase::Left && departed.secret.is_none() && departed.shares.is_empty()
    );
    for index in 1..7 {
        let record = load(&clients[index].db, &clients[index].key, &id).unwrap();
        assert!(record.phase == Phase::Active && record.transfer.is_none());
        assert_eq!(record.state.participants.len(), 6);
        assert!(!record
            .state
            .participants
            .iter()
            .any(|p| p.member.id == owner_id));
        media[index] = clients[index].start_call_media(id, tracks, now).unwrap();
    }
    for _ in 0..24 {
        round(&mut clients[1..7], now);
        let mut ready = true;
        for index in 1..7 {
            match clients[index].refresh_call_media(&mut media[index], now) {
                Ok(receivers) => ready &= receivers == 5,
                Err(Error::Unprepared) => ready = false,
                Err(error) => panic!("continued media: {error:?}"),
            }
        }
        if ready {
            break;
        }
    }
    let sender = load(&clients[1].db, &clients[1].key, &id)
        .unwrap()
        .own_id()
        .unwrap();
    let continued = clients[1]
        .seal_call_frame(
            &mut media[1],
            sigil_calls::MediaKind::Audio,
            126,
            false,
            b"after creator leaves",
            now,
        )
        .unwrap();
    assert!(sigil_calls::Receiver::new(&earlier_key)
        .unwrap()
        .open(&continued)
        .is_err());
    let current_key = load(&clients[2].db, &clients[2].key, &id)
        .unwrap()
        .shares
        .iter()
        .find(|share| share.key.context.sender == speaker)
        .unwrap()
        .key
        .clone();
    let mut guessed = serde_json::to_value(&current_key).unwrap();
    guessed["seed"] = serde_json::to_value(&earlier_key).unwrap()["seed"].clone();
    let guessed: sigil_calls::KeyShare = serde_json::from_value(guessed).unwrap();
    assert!(sigil_calls::Receiver::new(&guessed)
        .unwrap()
        .open(&continued)
        .is_err());
    for index in 2..7 {
        assert_eq!(
            &*clients[index]
                .open_call_frame(
                    &mut media[index],
                    sender,
                    sigil_calls::MediaKind::Audio,
                    &continued,
                    now
                )
                .unwrap()
                .data,
            b"after creator leaves"
        );
    }
    assert!(clients[0]
        .open_call_frame(
            &mut media[0],
            sender,
            sigil_calls::MediaKind::Audio,
            &continued,
            now
        )
        .is_err());
    assert!(clients[0]
        .seal_call_frame(
            &mut media[0],
            sigil_calls::MediaKind::Audio,
            126,
            false,
            b"left",
            now
        )
        .is_err());
    assert_eq!(
        clients[1]
            .call(id, now)
            .unwrap()
            .participants
            .iter()
            .filter(|p| p.verified)
            .count(),
        1
    );
    let controller = (1..7)
        .find(|&i| {
            load(&clients[i].db, &clients[i].key, &id)
                .unwrap()
                .owner_peer
                .is_none()
        })
        .unwrap();
    let recipient = if controller == 1 { 2 } else { 1 };
    let binding = crate::peers::parse(&clients[recipient].own_device_binding().unwrap())
        .unwrap()
        .binding;
    let request = [101; 32];
    let session = [102; 32];
    let message = [103; 32];
    clients[controller]
        .prepare_prekey_claim(request, binding.device, binding.identity)
        .unwrap();
    retry(|| clients[controller].claim_prekey_online(request, now));
    clients[controller]
        .start_claimed_initial(request, session, message, b"not a call control", now)
        .unwrap();
    assert_eq!(
        retry(|| clients[controller].send_pending_online(session, now)).accepted,
        1
    );
    let before: i64 = clients[recipient]
        .db
        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert!(retry(|| clients[recipient].receive_mailbox_online(now))
        .into_iter()
        .any(|attempt| matches!(attempt.result, Err(Error::Unprepared))));
    assert_eq!(
        before,
        clients[recipient]
            .db
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
            .unwrap()
    );
}
