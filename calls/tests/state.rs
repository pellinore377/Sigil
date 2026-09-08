use sigil_calls::*;
use sigil_crypto::IdentityKey;
use sigil_protocol::device::{Binding, SignedBinding};

fn state() -> (State, Vec<IdentityKey>) {
    let keys: Vec<_> = (0..8).map(|_| IdentityKey::generate().unwrap()).collect();
    let server = [
        "z".repeat(63),
        "z".repeat(63),
        "z".repeat(63),
        "z".repeat(61),
    ]
    .join(".");
    let mut roster = Roster {
        version: 1,
        call: [1; 32],
        server: server.clone(),
        owner: keys[0].public_key(),
        created: 1000,
        expires: 2000,
        revision: 0,
        previous: None,
        members: keys.iter().map(|k| Member::new(k.public_key())).collect(),
        closed: false,
    };
    roster.members.sort_by_key(|m| m.id);
    let mut participants = Vec::new();
    let mut ready = Vec::new();
    for (n, key) in keys.iter().enumerate() {
        let identity = IdentityKey::generate().unwrap();
        let binding = Binding {
            server: server.clone(),
            username: "z".repeat(32),
            account: [255; 32],
            device: [255 - n as u8; 32],
            identity: identity.public_key(),
        };
        let device = SignedBinding {
            signature: identity.sign(&binding.signing_bytes().unwrap()).unwrap(),
            binding,
        }
        .to_bytes()
        .unwrap();
        participants.push(
            Attestation::sign(&roster, Member::new(key.public_key()), device, &identity).unwrap(),
        );
        ready.push(
            Ready::sign(
                &roster,
                1,
                [n as u8 + 1; 32],
                Tracks {
                    audio: true,
                    camera: true,
                    screen: true,
                },
                key,
            )
            .unwrap(),
        );
    }
    participants.sort_by_key(|p| p.member.id);
    ready.sort_by_key(|r| r.member);
    let state = State {
        roster: roster.sign(&keys[0]).unwrap(),
        epoch: 0,
        participants,
        ready,
        signature: Vec::new(),
    }
    .sign(&keys[0])
    .unwrap();
    (state, keys)
}
#[test]
fn eight_maximum_addresses_fit_and_device_ready_share_authority_is_separate() {
    let (state, keys) = state();
    assert!(serde_json::to_vec(&state).unwrap().len() < 49152);
    let mut forged = state.clone();
    forged.participants[0].device[100] ^= 1;
    assert!(forged.sign(&keys[0]).is_err());
    let target = Member::new(keys[1].public_key()).id;
    let mut forged = state.clone();
    forged
        .ready
        .iter_mut()
        .find(|r| r.member == target)
        .unwrap()
        .tracks
        .camera = false;
    assert!(forged.sign(&keys[0]).is_err());
    let (_, media) = Sender::generate(Context {
        call: state.roster.roster.call,
        roster: state.roster.roster.digest().unwrap(),
        sender: target,
        incarnation: [3; 32],
    })
    .unwrap();
    assert!(Share::sign(&state, 1, media.clone(), &keys[0]).is_err());
    let share = Share::sign(&state, 1, media, &keys[1]).unwrap();
    share.verify(&state).unwrap();
    let mut next = state.clone();
    next.epoch += 1;
    let ready = next.ready.iter_mut().find(|r| r.member == target).unwrap();
    *ready = Ready::sign(&state.roster.roster, 2, [9; 32], ready.tracks, &keys[1]).unwrap();
    let next = next.sign(&keys[0]).unwrap();
    state.successor(&next).unwrap();
    assert!(share.verify(&next).is_err());
    let mut rollback = state.clone();
    rollback.epoch = next.epoch + 1;
    assert!(next.successor(&rollback.sign(&keys[0]).unwrap()).is_err());
}
