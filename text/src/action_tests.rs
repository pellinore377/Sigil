use crate::{action::*, structured::*, *};
fn card(source: &str) -> Card {
    let Parsed::Card(card) = parse_card(
        source,
        Origin {
            message: [1; 32],
            creator: [2; 32],
            created_at: 1000,
            timezone: None,
        },
        Default::default(),
    )
    .unwrap()
    .content
    else {
        panic!()
    };
    *card
}
fn vote(card: &Card, actor: u8, previous: Option<Id>, choice: usize) -> Action {
    let Construct::Poll(poll) = &card.content else {
        panic!()
    };
    Action {
        card: Reference::of(card).unwrap(),
        actor: [actor; 32],
        created_at: 1001,
        previous,
        revision: None,
        change: Change::Vote {
            choices: vec![poll.options[choice].id],
        },
    }
}
#[test]
fn ballot_versions_converge_without_sender_chosen_clocks_or_cross_voter_edits() {
    let card = card("poll::Choose\n- One\n- Two;");
    let first = vote(&card, 3, None, 0);
    first.validate_for(&card).unwrap();
    assert_eq!(first.depth_after(None).unwrap(), 1);
    let next = vote(&card, 3, Some(first.id().unwrap()), 1);
    assert_eq!(next.depth_after(Some((&first, 1))).unwrap(), 2);
    assert!(next.depth_after(None).is_err());
    assert!(next.wins_over(2, &first, 1).unwrap());
    let concurrent = vote(&card, 3, Some(first.id().unwrap()), 0);
    assert_eq!(concurrent.depth_after(Some((&first, 1))).unwrap(), 2);
    assert_ne!(
        next.wins_over(2, &concurrent, 2).unwrap(),
        concurrent.wins_over(2, &next, 2).unwrap()
    );
    let attacker = vote(&card, 4, Some(first.id().unwrap()), 1);
    assert!(attacker.depth_after(Some((&first, 1))).is_err());
    assert!(attacker.wins_over(2, &next, 2).is_err());
    let missing = vote(&card, 3, Some([7; 32]), 1);
    assert!(missing.depth_after(Some((&first, 1))).is_err());
    assert!(next.depth_after(Some((&first, i64::MAX as u64))).is_err());
    let mut equivocation = card.clone();
    equivocation.created_at += 1;
    assert!(first.validate_for(&equivocation).is_err());
}
#[test]
fn checklist_registers_are_independent_and_unsupported_task_mutations_fail() {
    let card = card("checklist::Things\n- One\n- Two;");
    let Construct::Checklist(list) = &card.content else {
        panic!()
    };
    let mut action = Action {
        card: Reference::of(&card).unwrap(),
        actor: [3; 32],
        created_at: 1001,
        previous: None,
        revision: None,
        change: Change::Check {
            item: list.items[0].id,
            checked: true,
        },
    };
    action.validate_for(&card).unwrap();
    let first = action.clone();
    action.actor = [4; 32];
    action.previous = Some(first.id().unwrap());
    action.change = Change::Check {
        item: list.items[0].id,
        checked: false,
    };
    assert_eq!(action.depth_after(Some((&first, 1))).unwrap(), 2);
    action.change = Change::Check {
        item: list.items[1].id,
        checked: true,
    };
    assert!(action.depth_after(Some((&first, 1))).is_err());
    let mut tasks = card.clone();
    let Construct::Checklist(list) = &mut tasks.content else {
        panic!()
    };
    list.mode = ListMode::Task;
    action.card = Reference::of(&tasks).unwrap();
    assert!(action.validate_for(&tasks).is_err());
}
#[test]
fn actions_have_canonical_content_ids_origin_binding_and_bounded_choices() {
    let card = card("poll::Choose\n- One\n- Two;");
    let mut action = vote(&card, 3, None, 0);
    let bytes = action.to_bytes().unwrap();
    assert!(Action::from_bytes(&bytes).unwrap() == action);
    assert!(matches!(
        Document::from_bytes(&bytes).unwrap(),
        Document::Action(_)
    ));
    assert!(Text::from_bytes(&bytes).is_err());
    assert!(Card::from_bytes(&bytes).is_err());
    action
        .authorize_origin(&action.id().unwrap(), &[3; 32], 1001)
        .unwrap();
    assert!(action
        .authorize_origin(&action.id().unwrap(), &[4; 32], 1001)
        .is_err());
    assert!(action.authorize_origin(&[9; 32], &[3; 32], 1001).is_err());
    let text = String::from_utf8(bytes).unwrap();
    for forged in [
        text.replacen("Poll vote updated", "forged", 1),
        text.replacen("\"previous\":null", "\"previous\":null,\"clock\":999999", 1),
        format!(" {text}"),
    ] {
        assert!(Action::from_bytes(forged.as_bytes()).is_err());
    }
    let Construct::Poll(poll) = &card.content else {
        panic!()
    };
    let mut choices: Vec<_> = poll.options.iter().map(|option| option.id).collect();
    choices.sort();
    action.change = Change::Vote {
        choices: choices.clone(),
    };
    assert!(action.validate_for(&card).is_err());
    action.change = Change::Vote {
        choices: vec![choices[0]; 2],
    };
    assert!(action.to_bytes().is_err());
    action.change = Change::Vote {
        choices: vec![[9; 32]],
    };
    assert!(action.validate_for(&card).is_err());
    action.change = Change::Vote { choices: vec![] };
    action.validate_for(&card).unwrap();
    assert_eq!(action.body().unwrap(), "Poll vote withdrawn");
}

#[test]
fn task_undo_is_bound_to_the_exact_completion_author_item_and_window() {
    let card = card("checklist::task::Tasks\n- One\n- Two;");
    let Construct::Checklist(list) = &card.content else {
        panic!()
    };
    let first = Action {
        card: Reference::of(&card).unwrap(),
        actor: [3; 32],
        created_at: 1001,
        previous: None,
        revision: None,
        change: Change::Complete {
            item: list.items[0].id,
        },
    };
    first.validate_for(&card).unwrap();
    assert_eq!(first.depth_after(None).unwrap(), 1);
    let completion = first.id().unwrap();
    let undo = Action {
        card: first.card,
        actor: first.actor,
        created_at: 1030,
        previous: Some(completion),
        revision: None,
        change: Change::Undo {
            item: list.items[0].id,
            completion,
        },
    };
    undo.validate_for(&card).unwrap();
    assert_eq!(undo.depth_after(Some((&first, 1))).unwrap(), 2);
    assert!(undo.register().unwrap() == first.register().unwrap());
    assert!(undo.wins_over(2, &first, 1).unwrap());
    assert!(Action::from_bytes(&undo.to_bytes().unwrap()).unwrap() == undo);
    for n in 0..6 {
        let mut invalid = undo.clone();
        match n {
            0 => invalid.actor = [4; 32],
            1 => invalid.created_at = 1031,
            2 => invalid.created_at = 1000,
            3 => {
                invalid.change = Change::Undo {
                    item: list.items[1].id,
                    completion,
                }
            }
            4 => invalid.previous = None,
            _ => invalid.card.digest = [8; 32],
        }
        assert!(invalid.depth_after(Some((&first, 1))).is_err());
    }
    assert!(undo.depth_after(None).is_err());
    assert!(undo.depth_after(Some((&first, 2))).is_err());
    let mut second = first.clone();
    second.actor = [4; 32];
    assert!(second.register().unwrap() != first.register().unwrap());
    assert!(undo.wins_over(2, &second, 1).is_err());
    let mut permanent = card.clone();
    let Construct::Checklist(ref mut list) = permanent.content else {
        panic!()
    };
    list.items[0].checked = true;
    let mut invalid = first;
    invalid.card = Reference::of(&permanent).unwrap();
    assert!(invalid.validate_for(&permanent).is_err());
}

#[test]
fn recurring_checks_are_period_bound_and_one_offs_cannot_return_after_reset() {
    let mut card = card("checklist::Items\n- One\n- Two;");
    let rule =
        recurrence::Recurrence::new(recurrence::Interval::Monthly, "UTC", card.created_at).unwrap();
    let reset = rule.next_reset(card.created_at).unwrap();
    let Construct::Checklist(ref mut list) = card.content else {
        panic!()
    };
    list.mode = ListMode::Recurring(rule);
    list.items[0].persistent = true;
    let items = [list.items[0].id, list.items[1].id];
    let initial = Action {
        card: Reference::of(&card).unwrap(),
        actor: [3; 32],
        created_at: reset - 1,
        previous: None,
        revision: None,
        change: Change::RecurringCheck {
            item: items[0],
            period: card.created_at,
        },
    };
    initial.validate_for(&card).unwrap();
    assert_eq!(initial.depth_after(None).unwrap(), 1);
    let current = Action {
        created_at: reset,
        change: Change::RecurringCheck {
            item: items[0],
            period: reset,
        },
        ..initial.clone()
    };
    current.validate_for(&card).unwrap();
    assert!(current.register().unwrap() != initial.register().unwrap());
    assert!(current.wins_over(1, &initial, 1).is_err());
    let mut old = current.clone();
    old.change = initial.change.clone();
    assert!(old.validate_for(&card).is_err());
    let one_off = Action {
        change: Change::RecurringCheck {
            item: items[1],
            period: card.created_at,
        },
        ..initial.clone()
    };
    one_off.validate_for(&card).unwrap();
    let removed = Action {
        change: Change::RecurringCheck {
            item: items[1],
            period: reset,
        },
        ..current.clone()
    };
    assert!(removed.validate_for(&card).is_err());
    let toggle = Action {
        change: Change::Check {
            item: items[0],
            checked: false,
        },
        ..current.clone()
    };
    assert!(toggle.validate_for(&card).is_err());
    let chained = Action {
        previous: Some(initial.id().unwrap()),
        revision: None,
        ..current
    };
    assert!(chained.validate().is_err());
    assert!(Action::from_bytes(&initial.to_bytes().unwrap()).unwrap() == initial);
}
