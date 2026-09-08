use sigil_text::{
    action::{Action, Change, Reference},
    location::{Duration, Mode, Point, Share},
    service::Coordinates,
    structured::{Card, Construct},
    Text,
};
fn card() -> Card {
    Card {
        id: [1; 32],
        creator: [2; 32],
        created_at: 1000,
        content: Construct::Location(Share {
            mode: Mode::Live {
                duration: Duration::FifteenMinutes,
                device: [3; 32],
            },
            point: Point {
                coordinates: Coordinates {
                    latitude_e6: 10_000_000,
                    longitude_e6: -20_000_000,
                },
                accuracy_cm: Some(250),
                sampled_at: 1000,
            },
            label: Text::plain("Synthetic", Default::default()).unwrap(),
        }),
    }
}
#[test]
fn location_wire_authority_duration_and_immutable_share_scope() {
    let card = card();
    let bytes = card.to_bytes().unwrap();
    assert!(Card::from_bytes(&bytes).unwrap() == card);
    let stop = Action {
        card: Reference::of(&card).unwrap(),
        actor: card.creator,
        created_at: 1001,
        previous: None,
        revision: None,
        change: Change::StopLocation,
    };
    stop.validate_for(&card).unwrap();
    assert!(stop.dependencies().is_empty());
    let mut forged = stop.clone();
    forged.actor = [9; 32];
    assert!(forged.validate_for(&card).is_err());
    let Construct::Location(mut value) = card.content.clone() else {
        panic!()
    };
    assert_eq!(value.until(1000).unwrap(), Some(1900));
    value.point.sampled_at = 1010;
    let mut update = Action {
        change: Change::Edit {
            content: Construct::Location(value.clone()),
            policy: None,
        },
        created_at: 1010,
        ..stop.clone()
    };
    update.validate_for(&card).unwrap();
    value.mode = Mode::Live {
        duration: Duration::EightHours,
        device: [3; 32],
    };
    update.change = Change::Edit {
        content: Construct::Location(value),
        policy: None,
    };
    assert!(update.validate_for(&card).is_err());
    update = Action {
        created_at: 1900,
        ..update
    };
    assert!(update.validate_for(&card).is_err());
    forged = stop;
    forged.previous = Some([8; 32]);
    assert!(forged.validate().is_err());
}
