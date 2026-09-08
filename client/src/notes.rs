use super::*;
#[cfg(test)]
#[path = "notes_tests.rs"]
mod tests;
use sigil_protocol::{
    conversation::{Body, Reference as MessageReference},
    text::{
        action::Reference,
        composition::Part,
        structured::{Card, Construct, ListMode},
        Document, Text,
    },
};

pub enum NoteKind {
    Note,
    Checklist,
    Task,
    Recurring,
    Reminder,
}
pub struct NoteCard {
    pub reference: Reference,
    pub kind: NoteKind,
    pub title: Text,
    pub active: bool,
    pub at: Option<u64>,
}
pub struct NoteEntry {
    pub message: crate::conversations::Message,
    pub cards: Vec<NoteCard>,
}
pub struct NotesPage {
    pub entries: Vec<NoteEntry>,
    pub next: Option<i64>,
}
impl ClientStore {
    pub fn notes_page(
        &mut self,
        conversation: Id,
        after: Option<i64>,
        now: u64,
    ) -> Result<NotesPage, Error> {
        let page = self.conversation_page(conversation, after, None, None, now)?;
        let mut entries = Vec::new();
        for message in page.messages {
            if message.view_once || message.body.is_none() {
                continue;
            }
            let cards: Vec<Card> = if let Some(Body::Rich(bytes)) = &message.body {
                match Document::from_bytes(bytes).map_err(|_| Error::InvalidStore)? {
                    Document::Card(card) => vec![card],
                    Document::Composition(value) => value
                        .parts
                        .into_iter()
                        .filter_map(|part| {
                            if let Part::Card(card) = part {
                                Some(*card)
                            } else {
                                None
                            }
                        })
                        .collect(),
                    _ => Vec::new(),
                }
            } else {
                Vec::new()
            };
            let mut notes = Vec::new();
            for card in cards {
                if !matches!(
                    card.content,
                    Construct::Note(_) | Construct::Checklist(_) | Construct::Reminder(_)
                ) {
                    continue;
                }
                let reference = Reference::of(&card).map_err(|_| Error::InvalidStore)?;
                let state = match self.card_state(conversation, reference) {
                    Ok(state) => state,
                    Err(Error::Obsolete) => continue,
                    Err(error) => return Err(error),
                };
                let (kind, title, active, at) = match state.definition.content {
                    Construct::Note(note) => (NoteKind::Note, note.text, true, None),
                    Construct::Reminder(reminder) => (
                        NoteKind::Reminder,
                        reminder.text,
                        now < reminder.at,
                        Some(reminder.at),
                    ),
                    Construct::Checklist(list) => match list.mode {
                        ListMode::Standard => (
                            NoteKind::Checklist,
                            list.title,
                            state.checks.iter().any(|item| !item.checked),
                            None,
                        ),
                        ListMode::Task => (
                            NoteKind::Task,
                            list.title,
                            state.tasks.iter().any(|item| !item.completed),
                            None,
                        ),
                        ListMode::Recurring(_) => {
                            let recurring = self.recurring_state(conversation, reference, now)?;
                            (
                                NoteKind::Recurring,
                                list.title,
                                recurring.checks.iter().any(|item| !item.checked),
                                Some(recurring.period.end),
                            )
                        }
                    },
                    _ => return Err(Error::InvalidStore),
                };
                notes.push(NoteCard {
                    reference,
                    kind,
                    title,
                    active,
                    at,
                });
            }
            if message.noted || !notes.is_empty() {
                entries.push(NoteEntry {
                    message,
                    cards: notes,
                });
            }
        }
        Ok(NotesPage {
            entries,
            next: page.next,
        })
    }
    pub fn note_promotion(
        &mut self,
        operation: Id,
        target: MessageReference,
        active: bool,
    ) -> Result<sigil_protocol::conversation::Operation, Error> {
        self.conversation_operation(
            operation,
            sigil_protocol::conversation::Action::Note { target, active },
        )
    }
}
