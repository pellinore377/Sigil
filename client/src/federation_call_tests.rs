use super::*;
use crate::calls::{Media, Phase};

fn step(source: &mut ClientStore, target: &mut ClientStore) {
    retry(|| {
        for attempt in source.resume_calls_online(now())? {
            attempt.result?;
        }
        Ok(())
    });
    pump(source, target);
}
fn until(
    alice: &mut ClientStore,
    bob: &mut ClientStore,
    mut done: impl FnMut(&mut ClientStore, &mut ClientStore) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        step(alice, bob);
        step(bob, alice);
        if done(alice, bob) {
            return;
        }
        assert!(Instant::now() < deadline, "federated call timed out");
        std::thread::sleep(Duration::from_millis(100));
    }
}
pub(super) fn run(alice: &mut ClientStore, bob: &mut ClientStore, server: &mut Store, pb: Id) {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let address = socket.local_addr().unwrap();
    drop(socket);
    server
        .configure_calls(sigil_server::call_config::Configure {
            expected_revision: 0,
            settings: Some(sigil_server::call_config::Settings {
                bind: address,
                advertised: address,
                max_calls: 1,
                turn_urls: Vec::new(),
            }),
            turn_secret: sigil_server::service_config::SecretUpdate::Clear,
        })
        .unwrap();
    let id = [211; 32];
    alice.create_call(id, now(), 3600).unwrap();
    alice.invite_to_call(id, pb, now()).unwrap();
    until(alice, bob, |_, bob| {
        bob.call(id, now()).is_ok_and(|c| c.phase == Phase::Ringing)
    });
    bob.answer_call(id, true, now()).unwrap();
    until(alice, bob, |_, bob| {
        bob.call(id, now()).is_ok_and(|c| c.phase == Phase::Active)
    });
    let tracks = sigil_calls::Tracks {
        audio: true,
        camera: true,
        screen: true,
    };
    let mut handles: [Media; 2] = [
        alice.start_call_media(id, tracks, now()).unwrap(),
        bob.start_call_media(id, tracks, now()).unwrap(),
    ];
    until(alice, bob, |alice, bob| {
        let a = alice.refresh_call_media(&mut handles[0], now());
        let b = bob.refresh_call_media(&mut handles[1], now());
        for value in [&a, &b] {
            assert!(matches!(value, Ok(_) | Err(Error::Unprepared)), "{value:?}");
        }
        matches!(a, Ok(1)) && matches!(b, Ok(1))
    });
    crate::calls::network_tests::exchange(alice, bob, id, &mut handles, now());
    eprintln!("Federated call media verified; closing call");
    alice.leave_call(id, now()).unwrap();
    until(alice, bob, |_, bob| {
        bob.call(id, now()).is_ok_and(|c| c.phase == Phase::Ended)
    });
    assert!(bob.call_relay_online(id, now()).is_err());
    let id = [212; 32];
    alice.create_group_call(id, now(), 3600).unwrap();
    alice.invite_to_call(id, pb, now()).unwrap();
    until(alice, bob, |_, bob| {
        bob.call(id, now())
            .is_ok_and(|call| call.phase == Phase::Ringing)
    });
    bob.answer_call(id, true, now()).unwrap();
    until(alice, bob, |_, bob| {
        bob.call(id, now())
            .is_ok_and(|call| call.phase == Phase::Active)
    });
    alice.leave_call(id, now()).unwrap();
    until(alice, bob, |_, bob| {
        bob.call(id, now())
            .is_ok_and(|call| call.phase == Phase::Active && call.participants.len() == 1)
            && bob.call_relay_online(id, now()).is_ok()
    });
    assert!(alice.call(id, now()).unwrap().phase == Phase::Left);
    bob.leave_call(id, now()).unwrap();
    step(bob, alice);
    assert!(bob.call(id, now()).unwrap().phase == Phase::Ended);
}
