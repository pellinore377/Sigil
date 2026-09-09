use super::*;
use sigil_protocol::text::{composition::Part, structured::Construct, Document};

pub(super) fn body_kind(body: Option<&Body>) -> &'static str {
    match body {
        Some(Body::File(bytes)) => match sigil_protocol::file::File::from_bytes(bytes) {
            Ok(file) if file.media_type.starts_with("image/") => "Images",
            Ok(file) if file.media_type.starts_with("video/") => "Videos",
            _ => "Files",
        },
        Some(Body::Text(text)) if text.contains("https://") || text.contains("http://") => "Links",
        Some(Body::Rich(bytes)) => {
            let location = |card: &sigil_protocol::text::structured::Card| {
                matches!(card.content, Construct::Location(_))
            };
            if match Document::from_bytes(bytes) {
                Ok(Document::Card(card)) => location(&card),
                Ok(Document::Composition(value)) => value
                    .parts
                    .iter()
                    .any(|p| matches!(p, Part::Card(card) if location(card))),
                _ => false,
            } {
                "Places"
            } else {
                "Structured"
            }
        }
        _ => "Text",
    }
}
impl ClientStore {
    pub(super) fn mobile_presence(&mut self, status: &str) -> Result<Value, Error> {
        use sigil_protocol::conversation::Presence;
        let activity = match status {
            "active" | "inactive" => None,
            "away" => Some(Presence::Away),
            "busy" => Some(Presence::Busy),
            _ => return Err(Error::InvalidEvent),
        };
        let now = conversations::now();
        let own = peers::parse(&self.own_device_binding()?)?.binding;
        let mut seen = std::collections::BTreeSet::new();
        for peer in self.mobile_peers()? {
            if peer.binding.server == own.server && peer.binding.account == own.account {
                continue;
            }
            if !peer.verified || peer.blocked || peer.changed_fingerprint.is_some() {
                continue;
            }
            let conversation = self.direct_conversation(peer.id)?;
            if !seen.insert(conversation)
                || !self
                    .conversation_preferences(conversation)?
                    .presence_sharing
            {
                continue;
            }
            let recipients = match self.mobile_recipients(peer.id) {
                Ok(value) => value,
                Err(Error::Unprepared) => continue,
                Err(error) => return Err(error),
            };
            let mut request = [0; 32];
            getrandom::fill(&mut request)
                .map_err(|_| Error::Crypto(sigil_crypto::Error::Entropy))?;
            let operation = self.conversation_operation(
                request,
                Action::Presence {
                    online: status != "inactive",
                    until: now.saturating_add(90),
                    activity,
                },
            )?;
            self.queue_direct_operation(&recipients, &operation, now, now)?;
        }
        Ok(json!({}))
    }
    pub(super) fn mobile_names(
        &mut self,
        peer: &str,
    ) -> Result<std::collections::BTreeMap<String, String>, Error> {
        let mut names = std::collections::BTreeMap::new();
        for peer in self.mobile_peers()? {
            let account =
                crate::event::account_reference(&peer.binding.server, &peer.binding.account);
            let name = if self.mobile_recipients(peer.id).is_ok() {
                self.mobile_profile_name(account)?
            } else {
                None
            };
            names.insert(
                transport::hex(&crate::event::account_reference(
                    &peer.binding.server,
                    &peer.binding.account,
                )),
                name.unwrap_or(peer.binding.username),
            );
        }
        if let Some(group) = peer.strip_prefix("group:") {
            for member in self.group_status(id(group)?)?.state.members() {
                let binding = member.account_binding();
                names.insert(
                    transport::hex(&crate::event::account_reference(
                        &binding.server,
                        &binding.account,
                    )),
                    binding.username.clone(),
                );
            }
        }
        names.insert(transport::hex(&self.account_reference()?), "You".into());
        Ok(names)
    }
    pub(super) fn mobile_conversation(&mut self, peer: &str) -> Result<Id, Error> {
        if peer == "self" {
            let (_, own) = structured::account_context(&self.db, &self.key)?;
            Ok(Sha256::digest([b"Sigil/note-to-self/v0".as_slice(), &own].concat()).into())
        } else if let Some(history) = peer.strip_prefix("history:") {
            self.connected_account_scope()?;
            id(history)
        } else if peer.starts_with("dm:") {
            self.mobile_contact_conversation(peer)
        } else if let Some(group) = peer.strip_prefix("group:") {
            let group = id(group)?;
            self.group_status(group)?;
            Ok(group)
        } else {
            self.direct_conversation(id(peer)?)
        }
    }
    pub(super) fn mobile_is_note(
        &mut self,
        _conversation: Id,
        message: &conversations::Message,
    ) -> Result<bool, Error> {
        if message.view_once || message.deleted {
            return Ok(false);
        }
        if message.noted {
            return Ok(true);
        }
        let Some(Body::Rich(bytes)) = &message.body else {
            return Ok(false);
        };
        let note = |c: &sigil_protocol::text::structured::Card| {
            matches!(
                c.content,
                Construct::Note(_) | Construct::Checklist(_) | Construct::Reminder(_)
            )
        };
        Ok(
            match Document::from_bytes(bytes).map_err(|_| Error::InvalidStore)? {
                Document::Card(card) => note(&card),
                Document::Composition(value) => value
                    .parts
                    .iter()
                    .any(|p| matches!(p, Part::Card(c) if note(c))),
                _ => false,
            },
        )
    }
    pub(super) fn mobile_summary(&mut self, peer: &str, chat: &mut Value) -> Result<(), Error> {
        let conversation = self.mobile_conversation(peer)?;
        let prefs = self.conversation_preferences(conversation)?;
        let now = conversations::now();
        let (_, own) = structured::account_context(&self.db, &self.key)?;
        chat["conversation"] = json!(transport::hex(&conversation));
        chat["pinned"] = json!(prefs.pinned);
        chat["hidden"] = json!(prefs.hidden);
        let mut unread = 0;
        let mut latest = false;
        let mut before = None;
        loop {
            let page = self.recent_conversation_page(conversation, before, now)?;
            if !latest {
                if let Some(message) = page.messages.first() {
                    chat["latest_message"] = json!(transport::hex(&message.reference.message));
                    chat["timestamp"] = json!(message.timestamp);
                    chat["preview"] = json!(message
                        .body
                        .as_ref()
                        .map(body_text)
                        .transpose()?
                        .unwrap_or_default());
                    latest = true;
                }
            }
            unread += page
                .messages
                .iter()
                .filter(|m| m.reference.author != own && !m.seen && !m.read.contains(&own))
                .count();
            before = page.next;
            if before.is_none() || unread > 99 {
                break;
            }
        }
        chat["hidden"] = json!(prefs.hidden || (!prefs.cleared.is_empty() && !latest));
        chat["unread"] = json!(unread.min(100).max(usize::from(prefs.unread)));
        chat["snoozed"] = json!(prefs.snoozed_until.is_some_and(|v| v > now));
        chat["collections"] = json!(prefs
            .collection_members
            .iter()
            .map(|v| transport::hex(v))
            .collect::<Vec<_>>());
        chat["draft"] = json!(prefs.drafts.first().map(|v| &v.text));
        chat["ui"] = json!(prefs.ui);
        chat["read_receipts"] = json!(prefs.read_receipts);
        chat["typing_indicators"] = json!(prefs.typing_indicators);
        chat["presence_sharing"] = json!(prefs.presence_sharing);
        let activity = self.conversation_activity(conversation, now)?;
        chat["presence"] = json!(activity
            .iter()
            .find(|a| a.author != own && a.online)
            .map(|a| match a.status {
                Some(sigil_protocol::conversation::Presence::Busy) => "busy",
                Some(sigil_protocol::conversation::Presence::Away) => "away",
                None => "active",
            })
            .unwrap_or("inactive"));
        chat["typing"] = json!(activity
            .iter()
            .filter(|a| a.author != own && a.typing)
            .map(|a| transport::hex(&a.author))
            .collect::<Vec<_>>());
        Ok(())
    }
    pub(super) fn mobile_search(
        &mut self,
        query: &str,
        after: Option<i64>,
        category: Option<&str>,
    ) -> Result<Value, Error> {
        let now = conversations::now();
        let own = self.account_reference()?;
        let page = self.recent_search_conversations(query, after, now)?;
        let mut peers = std::collections::BTreeMap::new();
        for peer in self.mobile_peers()? {
            peers.insert(
                self.direct_conversation(peer.id)?,
                self.mobile_peer_display(&peer)?,
            );
        }
        peers.insert(self.mobile_conversation("self")?, "self".into());
        for group in self.mobile_groups()? {
            let peer = group["id"].as_str().ok_or(Error::InvalidStore)?;
            peers.insert(
                id(peer.strip_prefix("group:").ok_or(Error::InvalidStore)?)?,
                peer.to_owned(),
            );
        }
        let mut hits = Vec::new();
        for hit in page.hits {
            let m = &hit.message;
            let prefs = self.conversation_preferences(hit.conversation)?;
            if prefs.hidden {
                continue;
            }
            let noted = self.mobile_is_note(hit.conversation, m)?;
            if match category.unwrap_or("") {
                "Notes" => !noted && peers.get(&hit.conversation).is_none_or(|p| p != "self"),
                "Pinned" => !m.pinned,
                "Images" | "Videos" | "Places" | "Links" => {
                    Some(body_kind(m.body.as_ref())) != category
                }
                "Unread" => m.reference.author == own || m.seen || m.read.contains(&own),
                "Conversations" | "Requests" => true,
                _ => false,
            } {
                continue;
            }
            hits.push(json!({"peer":peers.get(&hit.conversation).cloned().unwrap_or_else(||format!("history:{}",transport::hex(&hit.conversation))),
                "id":transport::hex(&m.reference.message),"author":transport::hex(&m.reference.author),
                "text":m.body.as_ref().map(body_text).transpose()?.unwrap_or_default().chars().take(512).collect::<String>(),"timestamp":m.timestamp,
                "pinned":m.pinned,"noted":noted || peers.get(&hit.conversation).is_some_and(|p|p=="self"),"kind":body_kind(m.body.as_ref()),
                "thread":m.thread.is_some(), "thread_author":m.thread.as_ref().map(|v| transport::hex(&v.author)), "thread_message":m.thread.as_ref().map(|v| transport::hex(&v.message))}));
        }
        Ok(json!({"hits":hits,"next":page.next}))
    }
}
