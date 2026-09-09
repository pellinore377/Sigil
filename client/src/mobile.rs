//! Native presentation boundary. Platform code supplies storage and foreground scheduling.
use crate::{
    connection::decode_id as id,
    conversations::{self, Action, Body, DeliveryState, Reference},
    *,
};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    State {},
    Discover {
        server: String,
    },
    Username {
        username: String,
    },
    Password {
        server: String,
        username: String,
        password: Zeroizing<String>,
    },
    Enroll {
        server: String,
        invitation: String,
        label: String,
    },
    Oidc {
        server: String,
        username: Option<String>,
        label: String,
        replace_devices: bool,
    },
    Resume {},
    Callback {
        request_id: String,
        completion: String,
    },
    Sync {},
    Publish {},
    Find {
        address: String,
    },
    Confirm {
        peer: String,
        fingerprint: String,
    },
    Timeline {
        peer: String,
        before: Option<i64>,
    },
    Post {
        peer: String,
        request: String,
        timestamp: u64,
        text: String,
        reply_author: Option<String>,
        reply_message: Option<String>,
    },
    React {
        peer: String,
        request: String,
        timestamp: u64,
        author: String,
        message: String,
        emoji: String,
        active: bool,
    },
    Pin {
        peer: String,
        request: String,
        timestamp: u64,
        author: String,
        message: String,
        active: bool,
    },
    Read {
        peer: String,
        request: String,
        timestamp: u64,
        author: String,
        message: String,
    },
}
fn reference(author: &str, message: &str) -> Result<Reference, Error> {
    Ok(Reference {
        author: id(author)?,
        message: id(message)?,
    })
}
fn login_server(input: &str) -> Result<String, Error> {
    let input = input.trim();
    let host = input
        .strip_prefix("https://")
        .unwrap_or(input)
        .trim_end_matches('/')
        .to_ascii_lowercase();
    if !sigil_protocol::valid_server_name(&host) {
        return Err(Error::InvalidEvent);
    }
    Ok(host)
}
fn public_peer(peer: &Peer) -> Value {
    json!({"id":transport::hex(&peer.id), "address":format!("@{}:{}",peer.binding.username,peer.binding.server),
        "fingerprint":transport::hex(&peer.fingerprint), "verified":peer.verified,
        "blocked":peer.blocked, "changed":peer.changed_fingerprint.is_some(), "device":transport::hex(&peer.binding.device)})
}
fn error_message(error: &Error) -> &'static str {
    match error {
        Error::Network(network::Error::Status { code:428,.. }) => "This account already has a device. Device linking or recovery is required; those screens are still being integrated.",
        Error::Network(network::Error::Status { code: 401, .. }) => {
            "Sign-in expired or access was revoked."
        }
        Error::Network(network::Error::Status { code: 403, .. }) => {
            "Access denied. Both people must approve each other's device before messaging."
        }
        Error::Network(network::Error::Status { code: 404, .. }) => {
            "Not found. Check the address and that account discovery is enabled."
        }
        Error::Network(network::Error::Status { code: 429, .. }) => {
            "The server asked us to wait. Queued messages will retry."
        }
        Error::Network(_) => "Cannot reach or verify the server. Check the address and connection.",
        Error::Unprepared => "Complete sign-in and device verification first.",
        Error::Conflict => "State changed or verification does not match. Refresh before retrying.",
        Error::Limit => "A size or capacity limit was reached.",
        Error::InvalidEvent => "Invalid request or message.",
        Error::Expired | Error::Obsolete => {
            "This operation expired or its message is no longer available."
        }
        _ => "The operation could not complete. Your stored keys have not been reset.",
    }
}
impl ClientStore {
    pub fn mobile_command(&mut self, request: &str) -> String {
        let result = if request.len() > 131072 {
            Err(Error::Limit)
        } else {
            serde_json::from_str(request)
                .map_err(|_| Error::InvalidEvent)
                .and_then(|request| self.mobile_execute(request))
        };
        match result {
            Ok(value) => json!({"ok":true,"value":value}),
            Err(error) => json!({"ok":false,"error":error_message(&error)}),
        }
        .to_string()
    }
    fn mobile_peers(&self) -> Result<Vec<Peer>, Error> {
        let ids = self
            .db
            .prepare("SELECT id FROM peers WHERE obsolete=0 ORDER BY id")?
            .query_map([], |r| r.get::<_, Vec<u8>>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|v| self.peer(v.try_into().map_err(|_| Error::InvalidStore)?))
            .collect()
    }
    fn mobile_state(&mut self) -> Result<Value, Error> {
        let phase = self.enrollment_kind()?;
        if phase != "connected" {
            return Ok(json!({"phase":phase,"server":self.enrollment_server()?}));
        }
        let session = self.connection_session()?.ok_or(Error::Unprepared)?;
        let fingerprint = device_fingerprint(&self.own_device_binding()?)?;
        let peers = self.mobile_peers()?;
        let mut chats = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for peer in &peers {
            let conversation = self.direct_conversation(peer.id)?;
            if !seen.insert(conversation) {
                continue;
            }
            let page = self.recent_conversation_page(conversation, None, conversations::now())?;
            let message = page.messages.first();
            let preview = message
                .and_then(|m| m.body.as_ref())
                .map(body_text)
                .transpose()?
                .unwrap_or_default();
            let mut chat = public_peer(peer);
            chat["preview"] = json!(preview);
            chat["timestamp"] = json!(message.map(|v| v.timestamp).unwrap_or(0));
            let same: Vec<_> = peers
                .iter()
                .filter(|p| {
                    p.binding.account == peer.binding.account
                        && p.binding.server == peer.binding.server
                })
                .collect();
            chat["devices"] = json!(same.iter().map(|p| public_peer(p)).collect::<Vec<_>>());
            chat["verified"] = json!(same
                .iter()
                .all(|p| p.verified && !p.blocked && p.changed_fingerprint.is_none()));
            chats.push(chat);
        }
        chats.sort_by_key(|v| std::cmp::Reverse(v["timestamp"].as_u64().unwrap_or(0)));
        Ok(
            json!({"phase":phase,"address":session.address,"device":session.device_id,"fingerprint":transport::hex(&fingerprint),"chats":chats}),
        )
    }
    fn mobile_recipients(&self, peer: Id) -> Result<Vec<Id>, Error> {
        let chosen = self.peer(peer)?;
        let peers = self.mobile_peers()?;
        let same: Vec<_> = peers
            .into_iter()
            .filter(|p| {
                p.binding.account == chosen.binding.account
                    && p.binding.server == chosen.binding.server
            })
            .collect();
        if same
            .iter()
            .any(|p| !p.verified || p.blocked || p.changed_fingerprint.is_some())
        {
            return Err(Error::Unprepared);
        }
        Ok(same.into_iter().map(|p| p.id).collect())
    }
    fn mobile_action(
        &mut self,
        peer: &str,
        request: &str,
        timestamp: u64,
        action: Action,
    ) -> Result<Value, Error> {
        let peers = self.mobile_recipients(id(peer)?)?;
        let operation = self.conversation_operation(id(request)?, action)?;
        self.queue_direct_operation(&peers, &operation, timestamp, conversations::now())?;
        Ok(json!({"queued":request}))
    }
    fn mobile_execute(&mut self, command: Command) -> Result<Value, Error> {
        match command {
            Command::State {} => self.mobile_state(),
            Command::Username { username } => {
                self.choose_registration_username(&username)?;
                if self.enrollment_kind()? == "connected" {
                    self.publish_device_binding_online()?;
                }
                self.mobile_state()
            }
            Command::Discover { server } => {
                let server = login_server(&server)?;
                serde_json::to_value(network::HttpsClient::login_methods(&server, 443, &[])?)
                    .map_err(|_| Error::InvalidEvent)
            }
            Command::Password {
                server,
                username,
                password,
            } => {
                self.sign_in_password_online(&server, 443, &[], &username, &password)?;
                self.publish_device_binding_online()?;
                self.mobile_state()
            }
            Command::Enroll {
                server,
                invitation,
                label,
            } => {
                self.prepare_enrollment(&server, 443, &[], &invitation, &label, false)?;
                self.enroll_online()?;
                self.publish_device_binding_online()?;
                self.mobile_state()
            }
            Command::Oidc {
                server,
                username,
                label,
                replace_devices,
            } => {
                self.prepare_oidc_enrollment(
                    &server,
                    443,
                    &[],
                    username.as_deref(),
                    &label,
                    replace_devices,
                )?;
                Ok(serde_json::to_value(self.start_oidc_online()?)
                    .map_err(|_| Error::InvalidStore)?)
            }
            Command::Resume {} => {
                if self.enrollment_kind()? == "oidc" {
                    if self.finish_oidc_online()?.is_none() {
                        if self.enrollment_kind()? == "username" {
                            return self.mobile_state();
                        }
                        return serde_json::to_value(self.start_oidc_online()?)
                            .map_err(|_| Error::InvalidStore);
                    }
                } else {
                    self.enroll_online()?;
                }
                self.publish_device_binding_online()?;
                self.mobile_state()
            }
            Command::Callback {
                request_id,
                completion,
            } => {
                self.accept_oidc_callback(&request_id, &completion)?;
                if self.finish_oidc_online()?.is_none() {
                    return self.mobile_state();
                }
                self.publish_device_binding_online()?;
                self.mobile_state()
            }
            Command::Sync {} => {
                let result = self.sync_due_online()?;
                let mut issue = result.scheduling_error.as_ref().map(error_message);
                if let Some(step) = &result.step {
                    if step.failure.is_some() {
                        issue = Some("Sync incomplete. Saved messages remain queued for retry.");
                    }
                    if step.incoming.iter().any(|v| v.result.is_err()) {
                        issue = Some("A received message needs device verification or recovery.");
                    }
                }
                Ok(json!({"next_at":result.next_at,"ran":result.step.is_some(),"issue":issue}))
            }
            Command::Publish {} => {
                self.publish_device_binding_online()?;
                Ok(json!({}))
            }
            Command::Find { address } => {
                let found = self.discover_account_online(&address)?;
                let (username, server) = address
                    .strip_prefix('@')
                    .and_then(|v| v.split_once(':'))
                    .ok_or(Error::InvalidEvent)?;
                let own = self.connection_session()?.ok_or(Error::Unprepared)?;
                for device in &found.devices {
                    let peer =
                        if own.address.split_once(':').ok_or(Error::InvalidStore)?.1 == server {
                            self.fetch_peer_online(id(device)?)?
                        } else {
                            self.fetch_remote_peer_online(server, id(device)?)?
                        };
                    if peer.binding.username != username
                        || peer.binding.server != server
                        || transport::hex(&peer.binding.account) != found.account
                    {
                        return Err(Error::Conflict);
                    }
                }
                self.mobile_state()
            }
            Command::Confirm { peer, fingerprint } => {
                let peer = id(&peer)?;
                self.confirm_peer(peer, id(&fingerprint)?)?;
                self.allow_peer_sender_online(peer)?;
                self.mobile_state()
            }
            Command::Timeline { peer, before } => {
                let peer = id(&peer)?;
                let conversation = self.direct_conversation(peer)?;
                let (_, own) = structured::account_context(&self.db, &self.key)?;
                let receipts = self.conversation_preferences(conversation)?.read_receipts;
                let page =
                    self.recent_conversation_page(conversation, before, conversations::now())?;
                let mut messages = Vec::new();
                for message in page.messages {
                    let mine = message.reference.author == own;
                    let delivery = if !mine {
                        ""
                    } else if !message.read.is_empty() {
                        "Read"
                    } else if !message.delivered.is_empty() {
                        "Delivered"
                    } else if mine {
                        match self.operation_delivery_state(peer, message.reference.message) {
                            Ok(DeliveryState::ServerAccepted) => "Sent",
                            Ok(DeliveryState::Expired) => "Expired",
                            Ok(DeliveryState::Cancelled) => "Cancelled",
                            Ok(_) => "Queued",
                            Err(Error::NotFound) => "Stored",
                            Err(e) => return Err(e),
                        }
                    } else {
                        ""
                    };
                    let reply = message
                        .reply
                        .as_ref()
                        .map(|target| {
                            match self.conversation_message(
                                conversation,
                                target.clone(),
                                conversations::now(),
                            ) {
                                Ok(m) if m.view_once || m.deleted => Ok("Earlier message".into()),
                                Ok(m) => m
                                    .body
                                    .as_ref()
                                    .map(body_text)
                                    .transpose()
                                    .map(|v| v.unwrap_or_else(|| "Earlier message".into())),
                                Err(Error::NotFound | Error::Obsolete) => {
                                    Ok("Earlier message".into())
                                }
                                Err(e) => Err(e),
                            }
                        })
                        .transpose()?;
                    messages.push(json!({"id":transport::hex(&message.reference.message),"author":transport::hex(&message.reference.author),
                        "text":message.body.as_ref().map(body_text).transpose()?.unwrap_or_else(||"View-once message".into()),
                        "mine":mine,"timestamp":message.timestamp,"delivery":delivery,"pinned":message.pinned,
                        "reactions":message.reactions.iter().map(|(_,emoji)|emoji).collect::<Vec<_>>(),
                        "my_reactions":message.reactions.iter().filter(|(actor,_)| *actor == own).map(|(_,emoji)|emoji).collect::<Vec<_>>(),
                        "read_by_me": !receipts || message.view_once || message.read.contains(&own), "reply":reply}));
                }
                Ok(json!({"peer":transport::hex(&peer),"messages":messages,"next":page.next}))
            }
            Command::Post {
                peer,
                request,
                timestamp,
                text,
                reply_author,
                reply_message,
            } => {
                let reply = match (reply_author, reply_message) {
                    (None, None) => None,
                    (Some(a), Some(m)) => Some(reference(&a, &m)?),
                    _ => return Err(Error::InvalidEvent),
                };
                self.mobile_action(
                    &peer,
                    &request,
                    timestamp,
                    Action::Post {
                        body: Body::Text(text),
                        reply,
                        thread: None,
                        expires_at: None,
                        view_once: false,
                    },
                )
            }
            Command::React {
                peer,
                request,
                timestamp,
                author,
                message,
                emoji,
                active,
            } => self.mobile_action(
                &peer,
                &request,
                timestamp,
                Action::Reaction {
                    target: reference(&author, &message)?,
                    emoji,
                    active,
                },
            ),
            Command::Pin {
                peer,
                request,
                timestamp,
                author,
                message,
                active,
            } => self.mobile_action(
                &peer,
                &request,
                timestamp,
                Action::Pin {
                    target: reference(&author, &message)?,
                    active,
                },
            ),
            Command::Read {
                peer,
                request,
                timestamp,
                author,
                message,
            } => self.mobile_action(
                &peer,
                &request,
                timestamp,
                Action::Receipt {
                    target: reference(&author, &message)?,
                    read: true,
                },
            ),
        }
    }
}
fn body_text(body: &Body) -> Result<String, Error> {
    Ok(match body {
        Body::Text(text) => text.clone(),
        Body::File(_) => "Attachment".into(),
        Body::Rich(bytes) => {
            match sigil_protocol::text::Document::from_bytes(bytes)
                .map_err(|_| Error::InvalidStore)?
            {
                sigil_protocol::text::Document::Text(v) => v.body().to_owned(),
                _ => "Structured message".into(),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn command(store: &mut ClientStore, value: Value) -> Value {
        serde_json::from_str(&store.mobile_command(&value.to_string())).unwrap()
    }
    #[test]
    fn presentation_preserves_trust_and_reopens_the_same_queued_message() {
        let (dir, _server, mut alice, mut bob, now) = crate::claims::tests::pair();
        let binding = bob.own_device_binding().unwrap();
        let peer = alice.observe_peer_binding(&binding).unwrap();
        let peer_id = transport::hex(&peer.id);
        let post = json!({"command":"post","peer":peer_id,"request":"07".repeat(32),"timestamp":now,"text":"A real queue","reply_author":null,"reply_message":null});
        assert_eq!(command(&mut alice, post.clone())["ok"], false);
        assert_eq!(
            command(&mut alice, json!({"command":"state"}))["value"]["chats"][0]["verified"],
            false
        );
        assert_eq!(
            command(
                &mut alice,
                json!({"command":"confirm","peer":peer_id,"fingerprint":"00".repeat(32)})
            )["ok"],
            false
        );
        assert_eq!(
            command(
                &mut alice,
                json!({"command":"confirm","peer":peer_id,"fingerprint":transport::hex(&peer.fingerprint)})
            )["ok"],
            true
        );
        assert_eq!(command(&mut alice, post.clone())["ok"], true);
        drop(alice);
        let mut alice = ClientStore::open(
            &dir.path().join("alice.db"),
            StorageKey::new(sigil_crypto::Secret32::from_bytes([9; 32])).unwrap(),
        )
        .unwrap();
        assert_eq!(command(&mut alice, post)["ok"], true);
        let view = command(
            &mut alice,
            json!({"command":"timeline","peer":peer_id,"before":null}),
        );
        assert_eq!(view["ok"], true);
        assert_eq!(view["value"]["messages"].as_array().unwrap().len(), 1);
        assert_eq!(view["value"]["messages"][0]["text"], "A real queue");
        assert_eq!(view["value"]["messages"][0]["delivery"], "Queued");
        assert_eq!(
            command(&mut alice, json!({"command":"state","unexpected":true}))["ok"],
            false
        );
    }
    #[test]
    fn recipient_batch_rolls_back_if_a_later_device_is_unverified() {
        let (_dir, _server, mut alice, mut bob, now) = crate::claims::tests::pair();
        let known = alice
            .observe_peer_binding(&bob.own_device_binding().unwrap())
            .unwrap();
        alice.confirm_peer(known.id, known.fingerprint).unwrap();
        let own = alice.own_device_binding().unwrap();
        let unverified = alice.observe_peer_binding(&own).unwrap();
        assert!(alice
            .queue_peer_contents(
                &[(known.id, [71; 32]), (unverified.id, [72; 32])],
                sigil_protocol::event::Content::Text("atomic"),
                now,
                now
            )
            .is_err());
        assert_eq!(
            alice
                .db
                .query_row("SELECT count(*) FROM send_intents", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn recent_pages_advance_across_deleted_candidates() {
        let (_dir, _server, mut alice, mut bob, now) = crate::claims::tests::pair();
        let known = alice
            .observe_peer_binding(&bob.own_device_binding().unwrap())
            .unwrap();
        alice.confirm_peer(known.id, known.fingerprint).unwrap();
        let conv = alice.direct_conversation(known.id).unwrap();
        for index in 1..=70u8 {
            let operation = alice
                .conversation_operation(
                    [index; 32],
                    Action::Post {
                        body: Body::Text(format!("Message {index}")),
                        reply: None,
                        thread: None,
                        expires_at: None,
                        view_once: false,
                    },
                )
                .unwrap();
            alice
                .queue_peer_operation(known.id, &operation, now, now)
                .unwrap();
        }
        let page = alice.recent_conversation_page(conv, None, now).unwrap();
        assert_eq!(page.messages.len(), 64);
        assert_eq!(page.messages[0].reference.message, [70; 32]);
        let deleted = alice
            .conversation_operation(
                [90; 32],
                Action::Delete {
                    target: page.messages[0].reference.clone(),
                },
            )
            .unwrap();
        alice
            .queue_peer_operation(known.id, &deleted, now, now)
            .unwrap();
        let page = alice.recent_conversation_page(conv, None, now).unwrap();
        assert_eq!(page.messages.len(), 63);
        let rest = alice
            .recent_conversation_page(conv, page.next, now)
            .unwrap();
        assert_eq!(rest.messages.len(), 6);
        assert_eq!(rest.messages.last().unwrap().reference.message, [1; 32]);
        assert!(rest.next.is_none());
    }
}

#[cfg(test)]
mod privacy_tests {
    use super::*;
    #[test]
    fn a_reply_never_previews_view_once_content() {
        let (_dir, _server, mut alice, mut bob, now) = crate::claims::tests::pair();
        let peer = alice
            .observe_peer_binding(&bob.own_device_binding().unwrap())
            .unwrap();
        alice.confirm_peer(peer.id, peer.fingerprint).unwrap();
        let hidden = alice
            .conversation_operation(
                [80; 32],
                Action::Post {
                    body: Body::Text("never-preview-this".into()),
                    reply: None,
                    thread: None,
                    expires_at: None,
                    view_once: true,
                },
            )
            .unwrap();
        alice
            .queue_peer_operation(peer.id, &hidden, now, now)
            .unwrap();
        let (_, author) = structured::account_context(&alice.db, &alice.key).unwrap();
        let reply = alice
            .conversation_operation(
                [81; 32],
                Action::Post {
                    body: Body::Text("Reply".into()),
                    reply: Some(Reference {
                        author,
                        message: hidden.id,
                    }),
                    thread: None,
                    expires_at: None,
                    view_once: false,
                },
            )
            .unwrap();
        alice
            .queue_peer_operation(peer.id, &reply, now, now)
            .unwrap();
        let result = alice.mobile_command(
            &json!({"command":"timeline","peer":transport::hex(&peer.id),"before":null})
                .to_string(),
        );
        let value: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(value["value"]["messages"][0]["reply"], "Earlier message");
        assert_eq!(value["value"]["messages"][1]["read_by_me"], true);
        assert!(!result.contains("never-preview-this"));
    }
}
