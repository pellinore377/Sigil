//! Native presentation boundary. Platform code supplies storage and foreground scheduling.
use crate::{
    connection::decode_id as id,
    conversations::{self, Action, Body, DeliveryState, Reference},
    *,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sigil_crypto::Secret32;
#[path = "mobile_account.rs"]
mod account;
#[path = "mobile_cards.rs"]
mod cards;
#[path = "mobile_contacts.rs"]
mod contacts;
#[path = "mobile_link.rs"]
mod device_link;
#[path = "mobile_files.rs"]
mod files;
#[path = "mobile_maps.rs"]
mod maps;
#[path = "mobile_calls.rs"]
mod mobile_calls;
#[path = "mobile_groups.rs"]
mod mobile_groups;
#[cfg(test)]
#[path = "mobile_tests.rs"]
mod presentation_tests;
#[path = "mobile_profile.rs"]
pub(crate) mod profile;
#[path = "mobile_views.rs"]
mod views;
#[path = "mobile_wallpaper.rs"]
mod wallpaper;

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    ContactRequest {
        peer: String,
        action: String,
    },
    ContactRefresh {},
    ContactPolicy {
        enabled: Option<bool>,
    },
    DeviceLink {
        action: String,
        qr: Option<String>,
    },
    Devices {
        cursor: Option<String>,
    },
    RevokeDevice {
        device: String,
    },
    SignOut {},
    Storage {},
    RecoveryGenerate {},
    RecoveryEnable {
        secret: Zeroizing<String>,
    },
    RecoveryPolicy {
        days: Option<u32>,
    },
    Notifications {},
    Presence {
        status: String,
    },
    ClearConversation {
        peer: String,
        request: String,
        timestamp: u64,
    },
    LeaveGroup {
        peer: String,
    },
    CallStart {
        peer: String,
        request: String,
        timestamp: u64,
    },
    CallRedial {
        call: String,
        request: String,
        timestamp: u64,
    },
    CallAnswer {
        call: String,
        accept: bool,
    },
    CallLeave {
        call: String,
    },
    CallInvite {
        call: String,
        peer: String,
    },
    Calls {},
    Place {
        peer: String,
        request: String,
        timestamp: u64,
        latitude_e6: i32,
        longitude_e6: i32,
        accuracy_cm: Option<u32>,
        sampled_at: u64,
        label: String,
        pin: bool,
        reply_author: Option<String>,
        reply_message: Option<String>,
        thread_author: Option<String>,
        thread_message: Option<String>,
    },
    CardAction {
        peer: String,
        author: String,
        message: String,
        card: String,
        item: Option<String>,
        checked: Option<bool>,
        choices: Option<Vec<String>>,
        timestamp: u64,
    },
    FileBegin {
        peer: String,
        request: String,
        timestamp: u64,
        length: u64,
        name: String,
        media_type: String,
        reply_author: Option<String>,
        reply_message: Option<String>,
        thread_author: Option<String>,
        thread_message: Option<String>,
    },
    FileFinish {
        request: String,
    },
    FileCancel {
        request: String,
    },
    Files {},
    FileWork {},
    FileGet {
        peer: String,
        author: String,
        message: String,
    },
    GroupCreate {
        request: String,
        timestamp: u64,
        name: String,
        description: String,
        peers: Vec<String>,
    },
    GroupInvitation {
        invitation: String,
        accept: bool,
    },
    State {},
    MarkRead {
        peer: String,
        request: String,
        timestamp: u64,
    },
    Snooze {
        peer: String,
        request: String,
        timestamp: u64,
        seconds: Option<u64>,
    },
    Forward {
        source: String,
        peer: String,
        author: String,
        message: String,
        request: String,
        timestamp: u64,
    },
    PostStatus {
        peer: String,
        request: String,
    },
    PhotoPublish {},
    PhotoRetry {},
    PhotoStatus {},
    PhotoCancel {},
    Profile {},
    SetProfile {
        revision: u64,
        name: String,
    },
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
    Sync {
        interactive: Option<bool>,
    },
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
        author: Option<String>,
        message: Option<String>,
        category: Option<String>,
        query: Option<String>,
        thread_author: Option<String>,
        thread_message: Option<String>,
    },
    Search {
        query: String,
        after: Option<i64>,
        category: Option<String>,
    },
    Draft {
        peer: String,
        request: String,
        timestamp: u64,
        text: String,
    },
    Organize {
        peer: Option<String>,
        request: String,
        timestamp: u64,
        value: Value,
    },
    Block {
        peer: String,
        active: bool,
    },
    Edit {
        peer: String,
        request: String,
        timestamp: u64,
        author: String,
        message: String,
        text: String,
    },
    Delete {
        peer: String,
        request: String,
        timestamp: u64,
        author: String,
        message: String,
    },
    Note {
        peer: String,
        request: String,
        timestamp: u64,
        author: String,
        message: String,
        active: bool,
    },
    Typing {
        peer: String,
        request: String,
        timestamp: u64,
        active: bool,
    },
    Post {
        peer: String,
        request: String,
        timestamp: u64,
        text: String,
        reply_author: Option<String>,
        reply_message: Option<String>,
        thread_author: Option<String>,
        thread_message: Option<String>,
        #[serde(default)]
        rich: bool,
        timezone: Option<String>,
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
fn optional_reference(
    author: Option<String>,
    message: Option<String>,
) -> Result<Option<Reference>, Error> {
    match (author, message) {
        (None, None) => Ok(None),
        (Some(author), Some(message)) => Ok(Some(reference(&author, &message)?)),
        _ => Err(Error::InvalidEvent),
    }
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
            let mut chat = public_peer(peer);
            let display = self.mobile_peer_display(peer)?;
            chat["id"] = json!(display);
            chat["preview"] = json!("");
            chat["timestamp"] = json!(0);
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
            if chat["verified"] == true && !self.mobile_contact_blocked(peer)? {
                let account = event::account_reference(&peer.binding.server, &peer.binding.account);
                chat["avatar"] = json!(transport::hex(&account));
                if let Some(name) = self.mobile_profile_name(account)? {
                    chat["name"] = json!(name);
                }
            }
            self.mobile_summary(&display, &mut chat)?;
            chat["contact_only"] =
                json!(chat["latest_message"].is_null() && chat["ui"]["opened"] != "true");
            chats.push(chat);
        }
        self.mobile_contact_chats(&mut chats)?;
        let mut chat = json!({"id":"self","address":session.address,"name":"Note to Self","self":true,"verified":true,"devices":[], "timestamp":0,"preview":""});
        let avatar = transport::hex(&self.account_reference()?);
        chat["avatar"] = json!(avatar);
        self.mobile_summary("self", &mut chat)?;
        if !chat["latest_message"].is_null() {
            chats.push(chat);
        }
        chats.extend(self.mobile_groups()?);
        let invitations = self.mobile_group_invitations()?;
        chats.sort_by_key(|v| {
            (
                std::cmp::Reverse(v["pinned"].as_bool().unwrap_or(false)),
                std::cmp::Reverse(v["timestamp"].as_u64().unwrap_or(0)),
            )
        });
        let prefs = self.conversation_preferences([0; 32])?;
        Ok(
            json!({"phase":phase,"address":session.address,"device":session.device_id,"fingerprint":transport::hex(&fingerprint),"chats":chats,"invitations":invitations,
                "profile_avatar":avatar,"photo_pending":self.photo_upload_pending()?,
                "read_receipts":prefs.read_receipts,"typing_indicators":prefs.typing_indicators,"presence_sharing":prefs.presence_sharing,
                "collections_enabled":prefs.collections_enabled,"ui":prefs.ui,"collections":prefs.collections.iter().map(|(id,name)|json!({"id":transport::hex(id),"name":name,"icon":prefs.ui.get(&format!("collection_icon.{}",transport::hex(id))).map(String::as_str).unwrap_or("folder")})).collect::<Vec<_>>()}),
        )
    }
    fn mobile_recipients(&self, peer: Id) -> Result<Vec<Id>, Error> {
        let chosen = self.peer(peer)?;
        if self.mobile_contact_blocked(&chosen)? {
            return Err(Error::Unprepared);
        }
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
        let operation = self.conversation_operation(id(request)?, action)?;
        if peer == "self" {
            self.note_to_self(&operation, timestamp, conversations::now())?;
        } else if let Some(group) = peer.strip_prefix("group:") {
            self.queue_group_operation(id(group)?, &operation, timestamp, conversations::now())?;
        } else {
            let peers = self.mobile_recipients(self.mobile_peer(peer)?)?;
            self.queue_direct_operation(&peers, &operation, timestamp, conversations::now())?;
        }
        Ok(json!({"queued":request}))
    }
    fn mobile_execute(&mut self, command: Command) -> Result<Value, Error> {
        match command {
            Command::ContactRequest { peer, action } => self.mobile_request(&peer, &action),
            Command::ContactRefresh {} => {
                self.mobile_contact_sync(true)?;
                self.mobile_state()
            }
            Command::ContactPolicy { enabled } => {
                let policy =
                    enabled.map(|enabled| sigil_protocol::contacts::RequestPolicy { enabled });
                Ok(serde_json::to_value(
                    self.connected_client()?
                        .contact_request_policy(policy.as_ref())?,
                )
                .map_err(|_| Error::InvalidStore)?)
            }

            Command::Presence { status } => self.mobile_presence(&status),
            Command::ClearConversation {
                peer,
                request,
                timestamp,
            } => {
                let conversation = self.mobile_conversation(&peer)?;
                self.clear_conversation(conversation, id(&request)?, timestamp)?;
                Ok(json!({}))
            }
            Command::LeaveGroup { peer } => self.mobile_leave_group(&peer),
            Command::DeviceLink { action, qr } => self.mobile_link(&action, qr.as_deref()),
            Command::Devices { cursor } => self.mobile_devices(cursor),
            Command::SignOut {} => {
                let session = self.connection_session()?.ok_or(Error::Unprepared)?;
                self.revoke_device_online(&session.device_id)?;
                Ok(json!({"revoked": true}))
            }
            Command::RevokeDevice { device } => {
                let session = self.connection_session()?.ok_or(Error::Unprepared)?;
                if device == session.device_id {
                    return Err(Error::InvalidEvent);
                }
                self.revoke_device_online(&device)?;
                self.mobile_devices(None)
            }
            Command::Storage {} => self.mobile_storage(),
            Command::RecoveryGenerate {} => self.mobile_recovery_generate(),
            Command::RecoveryEnable { secret } => {
                let binding = peers::parse(&self.own_device_binding()?)?.binding;
                let secret = Zeroizing::new(id(&secret)?);
                if *secret == [0; 32] {
                    return Err(Error::InvalidEvent);
                }
                self.configure_recovery(
                    &binding.server,
                    binding.account,
                    Secret32::from_bytes(*secret),
                )?;
                self.mobile_storage()
            }
            Command::RecoveryPolicy { days } => {
                self.set_recovery_policy(recovery::RecoveryPolicy { history_days: days })?;
                self.mobile_storage()
            }
            Command::Notifications {} => {
                let state = self.mobile_state()?;
                let eligible = state["chats"]
                    .as_array()
                    .map(|chats| {
                        chats
                            .iter()
                            .filter(|chat| {
                                chat["snoozed"] != true
                                    && chat["hidden"] != true
                                    && chat["blocked"] != true
                                    && chat["unread"].as_u64().unwrap_or(0) > 0
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let unread = eligible
                    .iter()
                    .map(|chat| chat["unread"].as_u64().unwrap_or(0))
                    .sum::<u64>();
                let stamp = eligible
                    .iter()
                    .map(|chat| {
                        (
                            &chat["conversation"],
                            &chat["unread"],
                            &chat["latest_message"],
                        )
                    })
                    .collect::<Vec<_>>();
                let revision = self.key.commitment(
                    &serde_json::to_vec(&stamp).map_err(|_| Error::InvalidStore)?,
                    b"Sigil/notification-revision/v1",
                )?;
                let now = conversations::now();
                let calls = self
                    .calls(now)?
                    .into_iter()
                    .filter(|call| call.phase == calls::Phase::Ringing)
                    .map(|call| json!({"id":transport::hex(&call.id),"until":call.ring_until}))
                    .collect::<Vec<_>>();
                Ok(json!({"unread":unread,"calls":calls,"revision":transport::hex(&revision)}))
            }
            Command::Place {
                peer,
                request,
                timestamp,
                latitude_e6,
                longitude_e6,
                accuracy_cm,
                sampled_at,
                label,
                pin,
                reply_author,
                reply_message,
                thread_author,
                thread_message,
            } => {
                let point = sigil_protocol::text::location::Point {
                    coordinates: sigil_protocol::text::service::Coordinates {
                        latitude_e6,
                        longitude_e6,
                    },
                    accuracy_cm,
                    sampled_at,
                };
                let card = self.location_card(
                    id(&request)?,
                    if pin {
                        structured::LocationKind::Pin
                    } else {
                        structured::LocationKind::Once
                    },
                    point,
                    sigil_protocol::text::Text::plain(&label, Default::default())
                        .map_err(|_| Error::InvalidEvent)?,
                    timestamp,
                )?;
                self.mobile_action(
                    &peer,
                    &request,
                    timestamp,
                    Action::Post {
                        body: Body::Rich(card.to_bytes().map_err(|_| Error::InvalidEvent)?),
                        reply: optional_reference(reply_author, reply_message)?,
                        thread: optional_reference(thread_author, thread_message)?,
                        expires_at: None,
                        view_once: false,
                    },
                )
            }
            Command::CardAction {
                peer,
                author,
                message,
                card,
                item,
                checked,
                choices,
                timestamp,
            } => self.mobile_card_action(
                &peer,
                reference(&author, &message)?,
                id(&card)?,
                item.as_deref().map(id).transpose()?,
                checked,
                choices
                    .map(|v| v.iter().map(|s| id(s)).collect())
                    .transpose()?,
                timestamp,
            ),
            Command::FileBegin {
                peer,
                request,
                timestamp,
                length,
                name,
                media_type,
                reply_author,
                reply_message,
                thread_author,
                thread_message,
            } => self.mobile_file_begin(files::Upload {
                peer,
                request: id(&request)?,
                timestamp,
                length,
                name,
                media_type,
                file: [0; 32],
                reply: optional_reference(reply_author, reply_message)?,
                thread: optional_reference(thread_author, thread_message)?,
            }),
            Command::FileFinish { request } => {
                let upload = self.mobile_upload(id(&request)?)?;
                self.mobile_cache()?.finish_staging(upload.file)?;
                Ok(json!({}))
            }
            Command::FileCancel { request } => {
                let request = id(&request)?;
                let upload = self.mobile_upload(request)?;
                self.mobile_cache()?.cancel(upload.file)?;
                self.db.execute(
                    "DELETE FROM mobile_uploads WHERE id=?1",
                    [request.as_slice()],
                )?;
                Ok(json!({}))
            }
            Command::Files {} => self.mobile_files(),
            Command::FileWork {} => self.mobile_file_work(),
            Command::FileGet {
                peer,
                author,
                message,
            } => self.mobile_file_get(&peer, reference(&author, &message)?),
            Command::GroupCreate {
                request,
                timestamp,
                name,
                description,
                peers,
            } => self.mobile_create_group(id(&request)?, timestamp, name, description, peers),
            Command::GroupInvitation { invitation, accept } => {
                if accept {
                    self.accept_group_invitation(id(&invitation)?, conversations::now())?;
                } else {
                    self.cancel_group_invitation(id(&invitation)?)?;
                }
                Ok(json!({}))
            }
            Command::State {} => self.mobile_state(),
            Command::Snooze {
                peer,
                request,
                timestamp,
                seconds,
            } => {
                let until = seconds
                    .map(|v| conversations::now().checked_add(v).ok_or(Error::Limit))
                    .transpose()?;
                self.mobile_execute(Command::Organize {
                    peer: Some(peer),
                    request,
                    timestamp,
                    value: json!({"Snooze":until}),
                })
            }
            Command::MarkRead {
                peer,
                request,
                timestamp,
            } => {
                let conversation = self.mobile_conversation(&peer)?;
                let (_, own) = structured::account_context(&self.db, &self.key)?;
                let mut before = None;
                loop {
                    let page =
                        self.recent_conversation_page(conversation, before, conversations::now())?;
                    for message in page.messages {
                        if message.reference.author == own || message.seen || message.view_once {
                            continue;
                        }
                        let token: Id = Sha256::digest(
                            [
                                b"Sigil/mobile-read-all/v1".as_slice(),
                                &id(&request)?,
                                &message.reference.author,
                                &message.reference.message,
                            ]
                            .concat(),
                        )
                        .into();
                        self.mobile_execute(Command::Read {
                            peer: peer.clone(),
                            request: transport::hex(&token),
                            timestamp,
                            author: transport::hex(&message.reference.author),
                            message: transport::hex(&message.reference.message),
                        })?;
                    }
                    before = page.next;
                    if before.is_none() {
                        break;
                    }
                }
                self.mobile_execute(Command::Organize {
                    peer: Some(peer),
                    request,
                    timestamp,
                    value: json!({"Unread":false}),
                })
            }
            Command::Forward {
                source,
                peer,
                author,
                message,
                request,
                timestamp,
            } => {
                let conversation = self.mobile_conversation(&source)?;
                let original = self.conversation_message(
                    conversation,
                    reference(&author, &message)?,
                    conversations::now(),
                )?;
                if original.deleted || original.view_once {
                    return Err(Error::Unprepared);
                }
                let body = original.body.ok_or(Error::Unprepared)?;
                if matches!(body, Body::File(_)) {
                    return Err(Error::Unprepared);
                }
                let rich = matches!(body, Body::Rich(_));
                self.mobile_execute(Command::Post {
                    peer,
                    request,
                    timestamp,
                    text: body_text(&body)?,
                    rich,
                    timezone: None,
                    reply_author: None,
                    reply_message: None,
                    thread_author: None,
                    thread_message: None,
                })
            }
            Command::PostStatus { peer, request } => {
                let conversation = self.mobile_conversation(&peer)?;
                let (_, author) = structured::account_context(&self.db, &self.key)?;
                let queued = match self.conversation_message(
                    conversation,
                    Reference {
                        author,
                        message: id(&request)?,
                    },
                    conversations::now(),
                ) {
                    Ok(_) => true,
                    Err(Error::NotFound) => false,
                    Err(error) => return Err(error),
                };
                Ok(json!({"queued":queued}))
            }
            Command::PhotoPublish {} => self.mobile_photo_publish(),
            Command::PhotoRetry {} => self.mobile_photo_retry(),
            Command::PhotoStatus {} => self.mobile_photo_status(),
            Command::PhotoCancel {} => {
                self.db
                    .execute("DELETE FROM mobile_photo_upload WHERE id=1", [])?;
                self.mobile_photo_status()
            }
            Command::Profile {} => Ok(serde_json::to_value(self.connected_client()?.profile()?)
                .map_err(|_| Error::InvalidStore)?),
            Command::SetProfile { revision, name } => Ok(serde_json::to_value(
                self.connected_client()?
                    .set_profile(&sigil_protocol::profile::Profile {
                        revision,
                        display_name: name,
                    })?,
            )
            .map_err(|_| Error::InvalidStore)?),
            Command::Search {
                query,
                after,
                category,
            } => self.mobile_search(&query, after, category.as_deref()),
            Command::Draft {
                peer,
                request,
                timestamp,
                text,
            } => {
                let conversation = self.mobile_conversation(&peer)?;
                let prefs = self.conversation_preferences(conversation)?;
                let device = device_fingerprint(&self.own_device_binding()?)?;
                if prefs
                    .drafts
                    .iter()
                    .any(|v| v.version.device == device && v.text == text)
                {
                    return Ok(json!({}));
                }
                let operation = self.conversation_operation(
                    id(&request)?,
                    Action::Private {
                        conversation,
                        value: conversations::Private::Draft {
                            text,
                            observed: Vec::new(),
                        },
                    },
                )?;
                self.apply_private_operation(&operation, timestamp)?;
                Ok(json!({}))
            }
            Command::Organize {
                peer,
                request,
                timestamp,
                mut value,
            } => {
                for kind in ["Collection", "CollectionMember"] {
                    if let Some(collection) = value.get_mut(kind) {
                        if let Some(raw) = collection["id"].as_str() {
                            collection["id"] = json!(id(raw)?);
                        }
                    }
                }
                let value = serde_json::from_value(value).map_err(|_| Error::InvalidEvent)?;
                let conversation = peer
                    .as_deref()
                    .map(|p| self.mobile_conversation(p))
                    .transpose()?
                    .unwrap_or([0; 32]);
                let operation = self.conversation_operation(
                    id(&request)?,
                    Action::Private {
                        conversation,
                        value,
                    },
                )?;
                self.apply_private_operation(&operation, timestamp)?;
                Ok(json!({}))
            }
            Command::Block { peer, active } => self.mobile_block(&peer, active),
            Command::Edit {
                peer,
                request,
                timestamp,
                author,
                message,
                text,
            } => self.mobile_action(
                &peer,
                &request,
                timestamp,
                Action::Edit {
                    target: reference(&author, &message)?,
                    body: Body::Text(text),
                },
            ),
            Command::Delete {
                peer,
                request,
                timestamp,
                author,
                message,
            } => self.mobile_action(
                &peer,
                &request,
                timestamp,
                Action::Delete {
                    target: reference(&author, &message)?,
                },
            ),
            Command::Note {
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
                Action::Note {
                    target: reference(&author, &message)?,
                    active,
                },
            ),
            Command::Typing {
                peer,
                request,
                timestamp,
                active,
            } => {
                let conversation = self.mobile_conversation(&peer)?;
                if !self
                    .conversation_preferences(conversation)?
                    .typing_indicators
                    || peer == "self"
                {
                    return Ok(json!({}));
                }
                self.mobile_action(
                    &peer,
                    &request,
                    timestamp,
                    Action::Typing {
                        active,
                        until: conversations::now().saturating_add(20),
                    },
                )
            }
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
            Command::CallStart {
                peer,
                request,
                timestamp,
            } => self.mobile_call_start(&peer, id(&request)?, timestamp),
            Command::CallRedial {
                call,
                request,
                timestamp,
            } => self.mobile_call_redial(id(&call)?, id(&request)?, timestamp),
            Command::CallAnswer { call, accept } => {
                self.answer_call(id(&call)?, accept, conversations::now())?;
                self.mobile_calls()
            }
            Command::CallLeave { call } => {
                self.leave_call(id(&call)?, conversations::now())?;
                self.mobile_calls()
            }
            Command::CallInvite { call, peer } => {
                let call = id(&call)?;
                if self.call(call, conversations::now())?.direct {
                    return Err(Error::InvalidEvent);
                }
                self.invite_to_call(call, self.mobile_peer(&peer)?, conversations::now())?;
                self.mobile_calls()
            }
            Command::Calls {} => self.mobile_calls(),
            Command::Sync { interactive } => {
                let result = if interactive == Some(true) {
                    self.sync_foreground_online()?
                } else {
                    self.sync_due_online()?
                };
                let mut issue = result.scheduling_error.as_ref().map(error_message);
                if self.mobile_contact_sync(false).is_err() {
                    issue = Some("Contact requests could not refresh. Messaging sync continues independently.");
                }
                if let Some(step) = &result.step {
                    if step.failure.is_some() {
                        issue = Some("Sync incomplete. Saved messages remain queued for retry.");
                    }
                    if step.incoming.iter().any(|v| v.result.is_err()) {
                        issue = Some("A received message needs device verification or recovery.");
                    }
                }
                let pending: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM send_intents) OR EXISTS(SELECT 1 FROM outbox o JOIN sessions s ON s.id=o.session WHERE o.packet IS NOT NULL AND s.retired=0) OR EXISTS(SELECT 1 FROM group_delivery WHERE status=0) OR EXISTS(SELECT 1 FROM call_jobs)", [], |row| row.get(0))?;
                let generated = result.step.as_ref().is_some_and(|step| {
                    step.conversation_copies > 0
                        || step.delivery_receipts > 0
                        || step.structured > 0
                });
                Ok(
                    json!({"next_at":result.next_at,"ran":result.step.is_some(),"pending":pending || generated,"issue":issue}),
                )
            }
            Command::Publish {} => {
                self.publish_device_binding_online()?;
                Ok(json!({}))
            }
            Command::Find { address } => self.mobile_find(&address),
            Command::Confirm { peer, fingerprint } => {
                let peer = id(&peer)?;
                self.confirm_peer(peer, id(&fingerprint)?)?;
                self.allow_peer_sender_online(peer)?;
                self.share_peer_profile(peer)?;
                self.mobile_state()
            }
            Command::Timeline {
                peer,
                before,
                author,
                message,
                category,
                query,
                thread_author,
                thread_message,
            } => {
                let conversation = self.mobile_conversation(&peer)?;
                let before = match (before, author, message) {
                    (None, Some(author), Some(message)) => Some(
                        self.conversation_position(conversation, &reference(&author, &message)?)?
                            .checked_add(1)
                            .ok_or(Error::Limit)?,
                    ),
                    (before, None, None) => before,
                    _ => return Err(Error::InvalidEvent),
                };
                let (_, own) = structured::account_context(&self.db, &self.key)?;
                let thread = match (thread_author, thread_message) {
                    (Some(a), Some(m)) => Some(reference(&a, &m)?),
                    (None, None) => None,
                    _ => return Err(Error::InvalidEvent),
                };
                let page = if let Some(query) = query {
                    self.recent_conversation_search(
                        conversation,
                        before,
                        &query,
                        conversations::now(),
                    )?
                } else {
                    self.recent_conversation_page(conversation, before, conversations::now())?
                };
                let mut messages = Vec::new();
                for message in page.messages {
                    if if let Some(thread) = &thread {
                        &message.reference != thread && message.thread.as_ref() != Some(thread)
                    } else {
                        match category.as_deref() {
                            Some("Notes") => {
                                peer != "self" && !self.mobile_is_note(conversation, &message)?
                            }
                            Some("Pins") => !message.pinned,
                            Some("Threads") => message.thread.is_none(),
                            Some("Timeline") => message.thread.is_some(),
                            Some("Search") | None => false,
                            _ => return Err(Error::InvalidEvent),
                        }
                    } {
                        continue;
                    }
                    let mine = message.reference.author == own;
                    let delivery = if !mine {
                        ""
                    } else if !message.read.is_empty() {
                        "Read"
                    } else if !message.delivered.is_empty() {
                        "Delivered"
                    } else if mine {
                        match if peer == "self" || peer.starts_with("group:") {
                            Err(Error::NotFound)
                        } else {
                            self.operation_delivery_state(
                                self.mobile_peer(&peer)?,
                                message.reference.message,
                            )
                        } {
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
                    let mut preview = |target: &Reference| match self.conversation_message(
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
                        Err(Error::NotFound | Error::Obsolete) => Ok("Earlier message".into()),
                        Err(e) => Err(e),
                    };
                    let reply = message.reply.as_ref().map(&mut preview).transpose()?;
                    let thread_preview = message.thread.as_ref().map(&mut preview).transpose()?;
                    messages.push(json!({"id":transport::hex(&message.reference.message),"author":transport::hex(&message.reference.author),
                        "text":message.body.as_ref().map(body_text).transpose()?.unwrap_or_else(||"View-once message".into()),
                        "mine":mine,"timestamp":message.timestamp,"delivery":delivery,"pinned":message.pinned,"attachment":files::metadata(message.body.as_ref())?,
                        "reactions":message.reactions.iter().map(|(_,emoji)|emoji).collect::<Vec<_>>(),
                        "my_reactions":message.reactions.iter().filter(|(actor,_)| *actor == own).map(|(_,emoji)|emoji).collect::<Vec<_>>(),
                        "read_by_me": message.seen || message.view_once || message.read.contains(&own), "reply":reply,
                        "readers":message.read.iter().map(|v|transport::hex(v)).collect::<Vec<_>>(),
                        "noted":message.noted || self.mobile_is_note(conversation, &message)?,
                        "thread_author":message.thread.as_ref().map(|v|transport::hex(&v.author)),
                        "thread_message":message.thread.as_ref().map(|v|transport::hex(&v.message)),
                        "thread_preview":thread_preview,
                        "editable":message.body.as_ref().is_some_and(Body::editable),
                        "kind":views::body_kind(message.body.as_ref()), "parts":self.mobile_parts(conversation, message.body.as_ref())?}));
                }
                let activity = self.conversation_activity(conversation, conversations::now())?;
                Ok(
                    json!({"peer":peer,"messages":messages,"next":page.next,"people":self.mobile_names(&peer)?,
                    "typing":activity.iter().filter(|v|v.typing && v.author != own).map(|v|transport::hex(&v.author)).collect::<Vec<_>>()}),
                )
            }
            Command::Post {
                peer,
                request,
                timestamp,
                text,
                reply_author,
                reply_message,
                thread_author,
                thread_message,
                rich,
                timezone,
            } => {
                let reply = optional_reference(reply_author, reply_message)?;
                let thread = optional_reference(thread_author, thread_message)?;
                let body = if rich {
                    let conversation = self.mobile_conversation(&peer)?;
                    let doc = self.prepare_sigiltext(crate::rich_text::SigilTextDraft {
                        conversation,
                        message: id(&request)?,
                        source: &text,
                        created_at: timestamp,
                        timezone: timezone.as_deref(),
                        date_order: None,
                    })?;
                    if matches!(doc, sigil_protocol::text::Document::Text(_)) {
                        self.discard_sigiltext_draft(conversation, id(&request)?)?;
                        return Err(Error::InvalidEvent);
                    }
                    Body::Rich(doc.to_bytes().map_err(|_| Error::InvalidEvent)?)
                } else {
                    Body::Text(text)
                };
                let result = self.mobile_action(
                    &peer,
                    &request,
                    timestamp,
                    Action::Post {
                        body,
                        reply,
                        thread,
                        expires_at: None,
                        view_once: false,
                    },
                )?;
                if rich {
                    let conversation = self.mobile_conversation(&peer)?;
                    self.discard_sigiltext_draft(conversation, id(&request)?)?;
                }
                Ok(result)
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
            } => {
                let conversation = self.mobile_conversation(&peer)?;
                let target = reference(&author, &message)?;
                let original =
                    self.conversation_message(conversation, target.clone(), conversations::now())?;
                if !original.seen {
                    let local_id = Sha256::digest(
                        [b"Sigil/mobile-seen/v1".as_slice(), &id(&request)?].concat(),
                    )
                    .into();
                    let local = self.conversation_operation(
                        local_id,
                        Action::Private {
                            conversation,
                            value: conversations::Private::Seen(target.clone()),
                        },
                    )?;
                    self.apply_private_operation(&local, timestamp)?;
                }
                if self.conversation_preferences(conversation)?.read_receipts && peer != "self" {
                    self.mobile_action(
                        &peer,
                        &request,
                        timestamp,
                        Action::Receipt { target, read: true },
                    )
                } else {
                    Ok(json!({}))
                }
            }
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
                sigil_protocol::text::Document::Card(c) => {
                    c.body().map_err(|_| Error::InvalidStore)?
                }
                sigil_protocol::text::Document::Composition(c) => {
                    c.body().map_err(|_| Error::InvalidStore)?
                }
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
