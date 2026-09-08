//! Structured actions carry causal predecessors, never a sender-chosen logical clock.
use crate::{
    structured::{self, Card, Construct, Id, ListMode, Selection},
    Error, Text, MAX_WIRE_BYTES,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    #[serde(with = "structured::id")]
    pub id: Id,
    #[serde(with = "structured::id")]
    pub creator: Id,
    #[serde(with = "structured::id")]
    pub digest: Id,
}
impl Reference {
    pub fn of(card: &Card) -> Result<Self, Error> {
        Ok(Self {
            id: card.id,
            creator: card.creator,
            digest: Sha256::digest(
                [b"Sigil/structured-card/v1".as_slice(), &card.to_bytes()?].concat(),
            )
            .into(),
        })
    }
    fn validate(self) -> Result<(), Error> {
        if self.id == [0; 32] || self.creator == [0; 32] || self.digest == [0; 32] {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub card: Reference,
    #[serde(with = "structured::id")]
    pub actor: Id,
    pub created_at: u64,
    #[serde(with = "optional_id")]
    pub previous: Option<Id>,
    #[serde(default, with = "optional_id", skip_serializing_if = "Option::is_none")]
    pub revision: Option<Id>,
    pub change: Change,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Change {
    StopLocation,
    ClosePoll(crate::poll_close::Closure),
    PollPage(crate::poll_close::Page),
    Edit {
        content: Construct,
        #[serde(with = "optional_id")]
        policy: Option<Id>,
    },
    Editors {
        content: Construct,
        #[serde(with = "ids")]
        editors: Vec<Id>,
    },
    Check {
        #[serde(with = "structured::id")]
        item: Id,
        checked: bool,
    },
    Vote {
        #[serde(with = "ids")]
        choices: Vec<Id>,
    },
    Complete {
        #[serde(with = "structured::id")]
        item: Id,
    },
    RecurringCheck {
        #[serde(with = "structured::id")]
        item: Id,
        period: u64,
    },
    Undo {
        #[serde(with = "structured::id")]
        item: Id,
        #[serde(with = "structured::id")]
        completion: Id,
    },
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Register {
    LocationStop,
    Item(Id),
    Ballot(Id),
    Completion(Id),
    Recurring(Id),
    Definition(Id),
    Policy,
    PollClose,
    PollPage(Id),
}
impl Register {
    pub fn recurring(item: Id, period: u64) -> Self {
        Self::Recurring(
            Sha256::digest(
                [
                    b"Sigil/recurring-item/v1".as_slice(),
                    &item,
                    &period.to_be_bytes(),
                ]
                .concat(),
            )
            .into(),
        )
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    body: String,
    format: String,
    formatted_body: String,
    sigil: Metadata,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    version: u8,
    unicode: String,
    action: Action,
}
impl Action {
    pub fn validate(&self) -> Result<(), Error> {
        self.card.validate()?;
        if self.actor == [0; 32]
            || self.created_at == 0
            || self.created_at > i64::MAX as u64
            || self.previous == Some([0; 32])
        {
            return Err(Error::Invalid);
        }
        match &self.change {
            Change::StopLocation if self.previous.is_some() || self.revision.is_some() => {
                Err(Error::Invalid)
            }
            Change::ClosePoll(_) | Change::PollPage(_)
                if self.previous.is_some() || self.revision.is_some() =>
            {
                Err(Error::Invalid)
            }
            Change::ClosePoll(value) => value.validate(),
            Change::PollPage(value) => value.validate(),
            Change::Edit { policy, .. } if *policy == Some([0; 32]) || self.revision.is_some() => {
                Err(Error::Invalid)
            }
            Change::Editors { editors, .. }
                if self.revision.is_some()
                    || editors.len() > 256
                    || editors.contains(&[0; 32])
                    || editors.windows(2).any(|pair| pair[0] >= pair[1]) =>
            {
                Err(Error::Invalid)
            }
            _ if self.revision == Some([0; 32]) => Err(Error::Invalid),
            Change::Edit { content, .. } | Change::Editors { content, .. } => {
                crate::editing::validate_shape(content, self.created_at)
            }
            Change::Check { item, .. }
            | Change::Complete { item }
            | Change::Undo { item, .. }
            | Change::RecurringCheck { item, .. }
                if *item == [0; 32] =>
            {
                Err(Error::Invalid)
            }
            Change::Complete { .. } if self.previous.is_some() => Err(Error::Invalid),
            Change::RecurringCheck { period, .. }
                if self.previous.is_some() || *period == 0 || *period > i64::MAX as u64 =>
            {
                Err(Error::Invalid)
            }
            Change::Undo { completion, .. }
                if *completion == [0; 32] || self.previous != Some(*completion) =>
            {
                Err(Error::Invalid)
            }
            Change::Vote { choices }
                if choices.len() > 64
                    || choices.contains(&[0; 32])
                    || choices.windows(2).any(|pair| pair[0] >= pair[1]) =>
            {
                Err(Error::Invalid)
            }
            _ => Ok(()),
        }
    }
    /// Conversation membership/actor authentication is the caller's responsibility.
    pub fn validate_for(&self, card: &Card) -> Result<(), Error> {
        self.validate_context(card, None, None, None)
    }
    /// Dependencies must already be authenticated and accepted for this card.
    pub fn validate_context(
        &self,
        card: &Card,
        revision: Option<&Action>,
        policy: Option<&Action>,
        parent: Option<&Action>,
    ) -> Result<(), Error> {
        self.validate()?;
        if self.card != Reference::of(card)? {
            return Err(Error::Invalid);
        }
        if self.change == Change::StopLocation {
            return if self.actor == card.creator
                && self.created_at >= card.created_at
                && matches!(
                    card.content,
                    Construct::Location(crate::location::Share {
                        mode: crate::location::Mode::Live { .. },
                        ..
                    })
                ) {
                Ok(())
            } else {
                Err(Error::Invalid)
            };
        }
        if matches!(self.change, Change::ClosePoll(_) | Change::PollPage(_)) {
            return if self.actor == card.creator && matches!(card.content, Construct::Poll(_)) {
                Ok(())
            } else {
                Err(Error::Invalid)
            };
        }
        if let Change::Edit {
            content,
            policy: policy_id,
        } = &self.change
        {
            crate::editing::validate_edit(self, card, content, *policy_id, policy, parent)?;
            return Ok(());
        }
        if let Change::Editors { content, .. } = &self.change {
            if self.actor != card.creator {
                return Err(Error::Invalid);
            }
            crate::editing::validate_content(card, content, self.created_at)?;
            if !matches!(&card.content, Construct::Checklist(list) if matches!(list.mode, ListMode::Recurring(_)))
            {
                return Err(Error::Invalid);
            }
            return Ok(());
        }
        let definition = crate::editing::at_revision(self, card, revision)?;
        match (&definition.content, &self.change) {
            (Construct::Checklist(list), Change::RecurringCheck { item, period }) => {
                let ListMode::Recurring(rule) = &list.mode else {
                    return Err(Error::Invalid);
                };
                let value = list
                    .items
                    .iter()
                    .find(|v| &v.id == item)
                    .ok_or(Error::Invalid)?;
                if rule.period_at(self.created_at)?.start != *period
                    || (!value.persistent && *period != rule.anchor_at)
                {
                    return Err(Error::Invalid);
                }
                Ok(())
            }
            (Construct::Checklist(list), Change::Complete { item } | Change::Undo { item, .. })
                if list.mode == ListMode::Task
                    && list
                        .items
                        .iter()
                        .any(|value| &value.id == item && !value.checked) =>
            {
                Ok(())
            }
            (Construct::Checklist(list), Change::Check { item, .. })
                if list.mode == ListMode::Standard
                    && list.items.iter().any(|value| &value.id == item) =>
            {
                Ok(())
            }
            (Construct::Poll(poll), Change::Vote { choices }) => {
                let max = match poll.selection {
                    Selection::Single => 1,
                    Selection::Unlimited => poll.options.len(),
                    Selection::Capped(max) => usize::from(max),
                };
                if choices.len() > max
                    || choices
                        .iter()
                        .any(|id| !poll.options.iter().any(|option| &option.id == id))
                {
                    return Err(Error::Invalid);
                }
                Ok(())
            }
            _ => Err(Error::Invalid),
        }
    }
    pub fn register(&self) -> Result<Register, Error> {
        Ok(match self.change {
            Change::StopLocation => Register::LocationStop,
            Change::Check { item, .. } => Register::Item(item),
            Change::Vote { .. } => Register::Ballot(self.actor),
            Change::Complete { .. } => Register::Completion(self.id()?),
            Change::Undo { completion, .. } => Register::Completion(completion),
            Change::RecurringCheck { item, period } => Register::recurring(item, period),
            Change::Edit { policy, .. } => Register::Definition(policy.unwrap_or([0; 32])),
            Change::Editors { .. } => Register::Policy,
            Change::ClosePoll(_) => Register::PollClose,
            Change::PollPage(ref page) => Register::PollPage(
                Sha256::digest(
                    [
                        b"Sigil/poll-page-register/v1".as_slice(),
                        &page.closure,
                        &page.index.to_be_bytes(),
                    ]
                    .concat(),
                )
                .into(),
            ),
        })
    }
    /// Only pass a previously validated, authenticated stored parent's depth.
    /// Out-of-order actions remain pending until that exact predecessor exists.
    pub fn depth_after(&self, parent: Option<(&Action, u64)>) -> Result<u64, Error> {
        self.validate()?;
        if let Change::Undo { item, completion } = self.change {
            let Some((parent, depth)) = parent else {
                return Err(Error::Invalid);
            };
            if parent.id()? != completion
                || parent.card != self.card
                || parent.actor != self.actor
                || parent.change != (Change::Complete { item })
                || depth != 1
                || self.created_at < parent.created_at
                || self.created_at - parent.created_at >= 30
            {
                return Err(Error::Invalid);
            }
            return Ok(2);
        }
        match (self.previous, parent) {
            (None, None) => Ok(1),
            (Some(id), Some((parent, depth)))
                if parent.id()? == id
                    && parent.card == self.card
                    && parent.register()? == self.register()?
                    && depth > 0 =>
            {
                depth
                    .checked_add(1)
                    .filter(|depth| *depth <= i64::MAX as u64)
                    .ok_or(Error::Limit)
            }
            _ => Err(Error::Invalid),
        }
    }
    pub fn wins_over(&self, depth: u64, prior: &Action, prior_depth: u64) -> Result<bool, Error> {
        if self.card != prior.card || self.register()? != prior.register()? {
            return Err(Error::Invalid);
        }
        Ok((depth, self.actor, self.id()?) > (prior_depth, prior.actor, prior.id()?))
    }
    pub fn body(&self) -> Result<&'static str, Error> {
        self.validate()?;
        Ok(match &self.change {
            Change::StopLocation => "Live location stopped",
            Change::Check { checked: true, .. } => "Checklist item checked",
            Change::Check { checked: false, .. } => "Checklist item unchecked",
            Change::Vote { choices } if choices.is_empty() => "Poll vote withdrawn",
            Change::Vote { .. } => "Poll vote updated",
            Change::Complete { .. } => "Task completed",
            Change::Undo { .. } => "Task completion undone",
            Change::RecurringCheck { .. } => "Recurring checklist item checked",
            Change::Edit { .. } => "Structured content edited",
            Change::Editors { .. } => "Recurrence editing permissions updated",
            Change::ClosePoll(_) => "Poll closed",
            Change::PollPage(_) => "Closed poll ballots",
        })
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        let body = self.body()?;
        let bytes = serde_json::to_vec(&Envelope {
            body: body.into(),
            format: "text/html".into(),
            formatted_body: Text::plain(body, Default::default())?.html(),
            sigil: Metadata {
                version: 1,
                unicode: "17.0.0".into(),
                action: self.clone(),
            },
        })
        .map_err(|_| Error::Invalid)?;
        if bytes.len() > MAX_WIRE_BYTES {
            return Err(Error::Limit);
        }
        Ok(bytes)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_WIRE_BYTES {
            return Err(Error::Limit);
        }
        let wire: Envelope = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
        if wire.sigil.version != 1 || wire.sigil.unicode != "17.0.0" {
            return Err(Error::Version);
        }
        let action = wire.sigil.action;
        if action.to_bytes()?.as_slice() != bytes {
            return Err(Error::Invalid);
        }
        Ok(action)
    }
    pub fn id(&self) -> Result<Id, Error> {
        Ok(
            Sha256::digest([b"Sigil/structured-action/v1".as_slice(), &self.to_bytes()?].concat())
                .into(),
        )
    }
    pub fn authorize_origin(&self, message: &Id, actor: &Id, timestamp: u64) -> Result<(), Error> {
        if &self.id()? != message || &self.actor != actor || self.created_at != timestamp {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub fn dependencies(&self) -> Vec<Id> {
        let mut ids: Vec<Id> = self.previous.into_iter().chain(self.revision).collect();
        if let Change::Edit {
            policy: Some(policy),
            ..
        } = self.change
        {
            ids.push(policy);
        }
        if let Change::PollPage(page) = &self.change {
            ids.push(page.closure);
            ids.extend(page.ballots.iter().map(|ballot| ballot.action));
        }
        ids.sort();
        ids.dedup();
        ids
    }
}
mod optional_id {
    use super::*;
    #[derive(Serialize, Deserialize)]
    struct Value(#[serde(with = "structured::id")] Id);
    pub fn serialize<S: serde::Serializer>(
        value: &Option<Id>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.map(Value).serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Id>, D::Error> {
        Ok(Option::<Value>::deserialize(deserializer)?.map(|value| value.0))
    }
}
pub(crate) mod ids {
    use super::*;
    #[derive(Serialize, Deserialize)]
    struct Value(#[serde(with = "structured::id")] Id);
    pub fn serialize<S: serde::Serializer>(value: &[Id], serializer: S) -> Result<S::Ok, S::Error> {
        value
            .iter()
            .copied()
            .map(Value)
            .collect::<Vec<_>>()
            .serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<Id>, D::Error> {
        Ok(Vec::<Value>::deserialize(deserializer)?
            .into_iter()
            .map(|value| value.0)
            .collect())
    }
}
