use super::*;
use sigil_protocol::text::{
    action::{Action as CardAction, Change, Reference as CardReference},
    composition::Part,
    structured::{Card, Construct, ListMode},
    Document,
};

fn parts(body: Option<&Body>) -> Result<Vec<Part>, Error> {
    let Some(Body::Rich(bytes)) = body else {
        return Ok(Vec::new());
    };
    Ok(
        match Document::from_bytes(bytes).map_err(|_| Error::InvalidStore)? {
            Document::Text(text) => vec![Part::Text(text)],
            Document::Action(_) => Vec::new(),
            Document::Card(card) => vec![Part::Card(Box::new(card))],
            Document::Composition(value) => value.parts,
        },
    )
}
impl ClientStore {
    fn mobile_task_undo(
        &self,
        conversation: Id,
        card: CardReference,
        item: Id,
    ) -> Result<Option<Id>, Error> {
        let own = self.account_reference()?;
        let now = conversations::now();
        let mut after = None;
        loop {
            let page = self.task_completions(conversation, card, item, after)?;
            if let Some(completion) = page
                .completions
                .iter()
                .find(|c| c.actor == own && c.created_at <= now && now < c.undo_until)
            {
                return Ok(Some(completion.operation));
            }
            after = page.next;
            if after.is_none() {
                return Ok(None);
            }
        }
    }
    pub(super) fn mobile_parts(
        &self,
        conversation: Id,
        body: Option<&Body>,
    ) -> Result<Vec<Value>, Error> {
        parts(body)?
            .into_iter()
            .map(|part| match part {
                Part::Text(text) => {
                    Ok(json!({"kind":"text", "text":text.body(), "rich":text.presentation()}))
                }
                Part::Card(card) => self.mobile_card(conversation, &card),
            })
            .collect()
    }
    fn mobile_card(&self, conversation: Id, card: &Card) -> Result<Value, Error> {
        let reference = CardReference::of(card).map_err(|_| Error::InvalidStore)?;
        let state = self.card_state(conversation, reference)?;
        let mut value = json!({"id":transport::hex(&card.id), "kind":"card", "text":card.body().map_err(|_| Error::InvalidStore)?, "items":[]});
        match &state.definition.content {
            Construct::Note(note) => {
                value["kind"] = json!("note");
                value["text"] = json!(note.text.body());
                value["rich"] = json!(note.text.presentation());
            }
            Construct::Checklist(list) => {
                value["kind"] = json!(if list.mode == ListMode::Task {
                    "task"
                } else {
                    "checklist"
                });
                value["text"] = json!(list.title.body());
                value["rich"] = json!(list.title.presentation());
                value["items"] = json!(list.items.iter().map(|item| -> Result<Value, Error> {
                    let check = state.checks.iter().find(|s| s.item == item.id);
                    let task = state.tasks.iter().find(|s| s.item == item.id);
                    let undo = if task.is_some_and(|t| t.completed && !t.initially_completed) { self.mobile_task_undo(conversation, reference, item.id)?.is_some() } else { false };
                    Ok(json!({"id":transport::hex(&item.id), "text":item.text.body(), "rich":item.text.presentation(), "checked":check.map(|s| s.checked).or_else(||task.map(|s|s.completed)).unwrap_or(item.checked), "enabled":check.is_some() || task.is_some_and(|s| !s.completed) || undo}))
                }).collect::<Result<Vec<_>,_>>()?);
            }
            Construct::Poll(poll) => {
                let current = state.poll.as_ref().ok_or(Error::InvalidStore)?;
                value["kind"] = json!("poll");
                value["text"] = json!(poll.question.body());
                value["rich"] = json!(poll.question.presentation());
                value["multiple"] =
                    json!(poll.selection != sigil_protocol::text::structured::Selection::Single);
                value["closed"] = json!(current.closed);
                value["voters"] = json!(current.voters);
                value["items"] = json!(poll.options.iter().map(|item| json!({"id":transport::hex(&item.id), "text":item.text.body(), "rich":item.text.presentation(), "checked":current.choices.contains(&item.id), "enabled":!current.closed, "count":current.counts.as_ref().and_then(|v|v.iter().find(|(id,_)|id==&item.id).map(|(_,n)|n))})).collect::<Vec<_>>());
            }
            Construct::Reminder(d) | Construct::Countdown(d) | Construct::Ago(d) => {
                value["kind"] = json!(match state.definition.content {
                    Construct::Reminder(_) => "reminder",
                    Construct::Countdown(_) => "countdown",
                    _ => "ago",
                });
                value["text"] = json!(d.text.body());
                value["rich"] = json!(d.text.presentation());
                value["at"] = json!(d.at);
            }
            Construct::Timer(timer) => {
                value["kind"] = json!("timer");
                value["text"] = json!("Timer");
                value["at"] = json!(timer.ends_at);
            }
            Construct::Location(_) => {
                let location = state.location.as_ref().ok_or(Error::InvalidStore)?;
                value["kind"] = json!("location");
                value["text"] = json!(location.share.label.body());
                value["rich"] = json!(location.share.label.presentation());
                value["latitude_e6"] = json!(location.share.point.coordinates.latitude_e6);
                value["longitude_e6"] = json!(location.share.point.coordinates.longitude_e6);
            }
            _ => {}
        }
        Ok(value)
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn mobile_card_action(
        &mut self,
        peer: &str,
        target: Reference,
        card: Id,
        item: Option<Id>,
        checked: Option<bool>,
        choices: Option<Vec<Id>>,
        timestamp: u64,
    ) -> Result<Value, Error> {
        let conversation = self.mobile_conversation(peer)?;
        let message = self.conversation_message(conversation, target, conversations::now())?;
        if message.deleted || message.view_once {
            return Err(Error::Obsolete);
        }
        let card = parts(message.body.as_ref())?
            .into_iter()
            .find_map(|part| match part {
                Part::Card(value) if value.id == card => Some(value),
                _ => None,
            })
            .ok_or(Error::NotFound)?;
        let reference = CardReference::of(&card).map_err(|_| Error::InvalidStore)?;
        let state = self.card_state(conversation, reference)?;
        let (previous, change) = match (item, checked, choices) {
            (Some(item), Some(checked), None) => {
                if let Some(current) = state.checks.iter().find(|s| s.item == item) {
                    if current.checked == checked {
                        return Ok(json!({}));
                    }
                    (current.operation, Change::Check { item, checked })
                } else if let Some(current) = state.tasks.iter().find(|s| s.item == item) {
                    if current.completed == checked {
                        return Ok(json!({}));
                    }
                    if checked {
                        (None, Change::Complete { item })
                    } else {
                        if current.initially_completed {
                            return Err(Error::Obsolete);
                        }
                        let completion = self
                            .mobile_task_undo(conversation, reference, item)?
                            .ok_or(Error::Obsolete)?;
                        (Some(completion), Change::Undo { item, completion })
                    }
                } else {
                    return Err(Error::InvalidEvent);
                }
            }
            (None, None, Some(mut choices)) => {
                choices.sort();
                choices.dedup();
                let current = state.poll.as_ref().ok_or(Error::InvalidEvent)?;
                if current.closed {
                    return Err(Error::Obsolete);
                }
                if current.choices == choices {
                    return Ok(json!({}));
                }
                (current.operation, Change::Vote { choices })
            }
            _ => return Err(Error::InvalidEvent),
        };
        let action = CardAction {
            card: reference,
            actor: self.account_reference()?,
            created_at: timestamp,
            previous,
            revision: state.definition.revision,
            change,
        };
        self.require_action(conversation, &action)?;
        let request = transport::hex(&action.id().map_err(|_| Error::InvalidEvent)?);
        self.mobile_action(
            peer,
            &request,
            timestamp,
            Action::Post {
                body: Body::Rich(action.to_bytes().map_err(|_| Error::InvalidEvent)?),
                reply: None,
                thread: None,
                expires_at: None,
                view_once: false,
            },
        )
    }
}
