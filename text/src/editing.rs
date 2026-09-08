use crate::{
    action::{Action, Change, Reference},
    structured::{Card, Construct, Id, ListMode},
    Error,
};
pub(crate) fn validate_shape(content: &Construct, at: u64) -> Result<(), Error> {
    if !matches!(
        content,
        Construct::Note(_)
            | Construct::Reminder(_)
            | Construct::Checklist(_)
            | Construct::Location(_)
    ) {
        return Err(Error::Invalid);
    }
    let created_at = if let Construct::Checklist(list) = content {
        if let ListMode::Recurring(rule) = &list.mode {
            if rule.anchor_at > at {
                return Err(Error::Invalid);
            }
            rule.anchor_at
        } else {
            at
        }
    } else {
        at
    };
    if let Construct::Location(value) = content {
        value.point.validate()?;
        return if value.label.body().len() <= 256
            && matches!(value.mode,crate::location::Mode::Live{device,..} if device!=[0;32])
        {
            Ok(())
        } else {
            Err(Error::Invalid)
        };
    }
    Card {
        id: [1; 32],
        creator: [1; 32],
        created_at,
        content: content.clone(),
    }
    .validate(Default::default())
}

pub(crate) fn validate_content(card: &Card, content: &Construct, at: u64) -> Result<(), Error> {
    if at < card.created_at {
        return Err(Error::Invalid);
    }
    match (&card.content, content) {
        (Construct::Note(_), Construct::Note(_))
        | (Construct::Location(_), Construct::Location(_))
        | (Construct::Reminder(_), Construct::Reminder(_)) => (),
        (Construct::Checklist(old), Construct::Checklist(new))
            if std::mem::discriminant(&old.mode) == std::mem::discriminant(&new.mode) => {}
        _ => return Err(Error::Invalid),
    }
    let mut edited = card.clone();
    edited.content = content.clone();
    if let Construct::Checklist(list) = content {
        if let ListMode::Recurring(rule) = &list.mode {
            if rule.anchor_at < card.created_at || rule.anchor_at > at {
                return Err(Error::Invalid);
            }
            edited.created_at = rule.anchor_at;
        }
    }
    edited.validate(Default::default())
}
pub(crate) fn validate_edit(
    action: &Action,
    card: &Card,
    content: &Construct,
    policy_id: Option<Id>,
    policy: Option<&Action>,
    parent: Option<&Action>,
) -> Result<(), Error> {
    validate_content(card, content, action.created_at)?;
    if let (Construct::Location(original), Construct::Location(value)) = (&card.content, content) {
        if action.actor != card.creator || policy_id.is_some() {
            return Err(Error::Invalid);
        }
        let previous = match parent {
            Some(parent) => match &parent.change {
                Change::Edit {
                    content: Construct::Location(value),
                    ..
                } => value,
                _ => return Err(Error::Invalid),
            },
            None => original,
        };
        return value.validate_update(original, previous, card.created_at, action.created_at);
    }
    let baseline = match (policy_id, policy) {
        (None, None) => &card.content,
        (Some(id), Some(policy))
            if policy.id()? == id && policy.card == action.card && policy.actor == card.creator =>
        {
            let Change::Editors { content, editors } = &policy.change else {
                return Err(Error::Invalid);
            };
            if action.actor != card.creator && editors.binary_search(&action.actor).is_err() {
                return Err(Error::Invalid);
            }
            content
        }
        _ => return Err(Error::Invalid),
    };
    if action.actor == card.creator {
        return Ok(());
    }
    if policy_id.is_none() {
        return Err(Error::Invalid);
    }
    let baseline = match (action.previous, parent) {
        (None, None) => baseline,
        (Some(id), Some(parent))
            if parent.id()? == id
                && parent.card == action.card
                && parent.register()? == action.register()? =>
        {
            let Change::Edit { content, .. } = &parent.change else {
                return Err(Error::Invalid);
            };
            content
        }
        _ => return Err(Error::Invalid),
    };
    match (baseline, content) {
        (Construct::Checklist(old), Construct::Checklist(new))
            if old.title == new.title
                && old.items == new.items
                && matches!(old.mode, ListMode::Recurring(_))
                && matches!(new.mode, ListMode::Recurring(_)) =>
        {
            Ok(())
        }
        _ => Err(Error::Invalid),
    }
}
pub fn at_revision(action: &Action, card: &Card, revision: Option<&Action>) -> Result<Card, Error> {
    let mut value = card.clone();
    match (action.revision, revision) {
        (None, None) => (),
        (Some(id), Some(revision))
            if revision.id()? == id && revision.card == Reference::of(card)? =>
        {
            let (Change::Edit { content, .. } | Change::Editors { content, .. }) = &revision.change
            else {
                return Err(Error::Invalid);
            };
            validate_content(card, content, revision.created_at)?;
            value.content = content.clone();
        }
        _ => return Err(Error::Invalid),
    }
    Ok(value)
}
