//! MatrixRTC state/message events: membership (MSC3401 session format), ring (MSC4075),
//! decline, and delayed leave events (MSC4140).
use anyhow::Context;
use matrix_sdk::{Client, Room};
use ruma::events::call::member::{ActiveFocus, ActiveLivekitFocus, Application, CallApplicationContent, CallMemberEventContent, CallMemberStateKey, CallScope, Focus, LivekitFocus};
use ruma::events::rtc::notification::CallIntent;
use ruma::events::{AnyStateEventContent, StateEventType};
use ruma::{OwnedUserId, UserId};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

pub const MEMBER_EVENT: &str = "org.matrix.msc3401.call.member";

/// `io.element.call.encryption_keys` to-device payload (Element Call wire format).
#[derive(Clone, Debug, Serialize, Deserialize, ruma::events::macros::EventContent)]
#[ruma_event(type = "io.element.call.encryption_keys", kind = ToDevice)]
pub struct RtcEncryptionKeyEventContent {
    pub keys: RtcKey,
    #[serde(default)]
    pub room_id: String,
    #[serde(default)]
    pub member: RtcKeyMember,
    #[serde(default)]
    pub session: Option<serde_json::Value>,
    #[serde(default)]
    pub sent_ts: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RtcKey { pub key: String, pub index: u8 }
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RtcKeyMember {
    #[serde(default)]
    pub claimed_device_id: String,
    #[serde(default)]
    pub id: String,
}

/// Unstable-prefixed ring notification (what Element sends today).
#[derive(Clone, Debug, Serialize, Deserialize, ruma::events::macros::EventContent)]
#[ruma_event(type = "org.matrix.msc4075.rtc.notification", kind = MessageLike)]
pub struct Msc4075NotificationContent {
    #[serde(default)]
    pub notification_type: String,
    #[serde(default)]
    pub sender_ts: u64,
    #[serde(default)]
    pub lifetime: u64,
    #[serde(rename = "m.call.intent", default)]
    pub call_intent: Option<String>,
    #[serde(default)]
    pub device_id: Option<String>,
}

pub fn state_key_for(user: &UserId, device_id: &str) -> CallMemberStateKey {
    CallMemberStateKey::new(user.to_owned(), Some(format!("{device_id}_m.call")), true)
}

pub async fn send_call_open(room: &Room) -> anyhow::Result<()> {
    let body = serde_json::json!({"m.intent": "m.prompt", "m.type": "m.video"});
    let raw = ruma::serde::Raw::<AnyStateEventContent>::from_json(serde_json::value::to_raw_value(&body)?);
    let req = ruma::api::client::state::send_state_event::v3::Request::new_raw(room.room_id().to_owned(), StateEventType::from("org.matrix.msc3401.call"), String::new(), raw);
    room.client().send(req).await.context("send org.matrix.msc3401.call")?;
    Ok(())
}

pub fn membership_content(device_id: &str, identity: &str, service_url: &str, alias: &str, video: bool, expires: Option<std::time::Duration>, created_ts: ruma::MilliSecondsSinceUnixEpoch) -> anyhow::Result<serde_json::Value> {
    let mut app = CallApplicationContent::new(String::new(), CallScope::Room);
    app.call_intent = Some(if video { CallIntent::Video } else { CallIntent::Audio });
    let content = CallMemberEventContent::new(
        Application::Call(app),
        device_id.into(),
        ActiveFocus::Livekit(ActiveLivekitFocus::new()),
        vec![Focus::Livekit(LivekitFocus::new(alias.to_owned(), service_url.to_owned()))],
        Some(created_ts),
        expires,
    );
    let mut body = serde_json::to_value(&content)?;
    body["membershipID"] = serde_json::Value::String(identity.to_owned());
    Ok(body)
}

pub async fn send_member_join(room: &Room, device_id: &str, identity: &str, service_url: &str, video: bool, expires: Option<std::time::Duration>, created_ts: ruma::MilliSecondsSinceUnixEpoch) -> anyhow::Result<()> {
    let user = room.own_user_id();
    let key = state_key_for(user, device_id);
    let body = membership_content(device_id, identity, service_url, room.room_id().as_str(), video, expires, created_ts)?;
    let raw = ruma::serde::Raw::<AnyStateEventContent>::from_json(serde_json::value::to_raw_value(&body)?);
    let req = ruma::api::client::state::send_state_event::v3::Request::new_raw(room.room_id().to_owned(), StateEventType::from(MEMBER_EVENT), key.as_ref().to_owned(), raw);
    room.client().send(req).await.context("send call.member join")?;
    debug!("rtc: membership join sent (key {})", key.as_ref());
    Ok(())
}

pub async fn send_member_leave(room: &Room, device_id: &str) -> anyhow::Result<()> {
    let key = state_key_for(room.own_user_id(), device_id);
    let raw = ruma::serde::Raw::<AnyStateEventContent>::from_json(serde_json::value::to_raw_value(&serde_json::json!({}))?);
    let req = ruma::api::client::state::send_state_event::v3::Request::new_raw(room.room_id().to_owned(), StateEventType::from(MEMBER_EVENT), key.as_ref().to_owned(), raw);
    room.client().send(req).await.context("send call.member leave")?;
    info!("rtc: membership leave sent");
    Ok(())
}

/// MSC4140: server sends our leave automatically unless we keep restarting the timer.
pub async fn schedule_delayed_leave(client: &Client, room: &Room, device_id: &str, timeout: std::time::Duration) -> anyhow::Result<String> {
    use ruma::api::client::delayed_events::{delayed_state_event, DelayParameters};
    let key = state_key_for(room.own_user_id(), device_id);
    let raw = ruma::serde::Raw::<AnyStateEventContent>::from_json(serde_json::value::to_raw_value(&serde_json::json!({}))?);
    let req = delayed_state_event::unstable::Request::new_raw(room.room_id().to_owned(), key.as_ref().to_owned(), StateEventType::from(MEMBER_EVENT), DelayParameters::Timeout { timeout }, raw);
    let resp = client.send(req).await.context("delayed leave")?;
    Ok(resp.delay_id)
}

pub async fn update_delayed(client: &Client, delay_id: &str, action: ruma::api::client::delayed_events::update_delayed_event::unstable::UpdateAction) -> anyhow::Result<()> {
    use ruma::api::client::delayed_events::update_delayed_event;
    client.send(update_delayed_event::unstable::Request::new(delay_id.to_owned(), action)).await.context("update delayed")?;
    Ok(())
}

/// Any other device currently in the call (non-empty session membership, not ours)?
pub async fn other_active_members(room: &Room) -> Vec<(OwnedUserId, String)> {
    let mut out = Vec::new();
    let own = room.own_user_id().to_owned();
    let Ok(events) = room.get_state_events_static::<CallMemberEventContent>().await else { return out };
    for raw in events {
        let Ok(ev) = raw.deserialize() else { continue };
        let matrix_sdk::deserialized_responses::SyncOrStrippedState::Sync(ruma::events::SyncStateEvent::Original(o)) = ev else { continue };
        if o.sender == own { continue; }
        let active = o.content.active_memberships(Some(o.origin_server_ts));
        for m in active {
            out.push((o.sender.clone(), m.device_id().to_string()));
        }
    }
    out
}

pub async fn send_ring(room: &Room, video: bool, device_id: &str) -> anyhow::Result<String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    let body = serde_json::json!({
        "notification_type": "ring",
        "sender_ts": now,
        "lifetime": 30_000u64,
        "device_id": device_id,
        "m.mentions": { "room": true },
        "m.call.intent": if video { "video" } else { "audio" },
    });
    let resp = room.send_raw("org.matrix.msc4075.rtc.notification", body).await.context("send ring")?;
    Ok(resp.response.event_id.to_string())
}

pub async fn send_decline(room: &Room, notification_event_id: &str) -> anyhow::Result<()> {
    let eid = ruma::EventId::parse(notification_event_id).context("bad notification event id")?;
    match room.make_decline_call_event(&eid).await {
        Ok(content) => { room.send(content).await.context("send decline")?; }
        Err(e) => warn!("decline event not sent: {e}"),
    }
    Ok(())
}
