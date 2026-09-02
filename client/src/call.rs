//! Calls. A call lives in a conversation: whoever starts it picks a random
//! 32-byte room and tells the others inside an encrypted kind-10 event, so
//! the server only ever sees the room, never which conversation it belongs
//! to. Media then goes through the forwarding unit on the conversation's
//! server, driven by `call.signal` operations (wire spec 3.8): `join` with
//! an SDP offer, `poll` for renegotiation offers, `answer`, `leave`.
//!
//! What is here is the signalling; capture, encoding and playback belong
//! to the application, which hands SDP in and gets SDP out.

use crate::conversation::send_event;
use crate::link::Link;
use crate::provider::SigilProvider;
use crate::state::{Conversation, State};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sigil_protocol::envelope;
use sigil_protocol::wire::{Request, Response};

/// The body of a kind-10 event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallEvent {
    /// `start` or `end`.
    pub action: String,
    /// The forwarding unit's room, hex.
    pub room: String,
}

/// Start a call: pick a room and announce it to the conversation.
pub async fn start(
    link: &Link,
    st: &mut State,
    provider: &SigilProvider,
    conv: &Conversation,
) -> anyhow::Result<[u8; 32]> {
    let room: [u8; 32] = rand::random();
    let ev = CallEvent {
        action: "start".into(),
        room: hex::encode(room),
    };
    send_event(
        link,
        st,
        provider,
        conv,
        envelope::Kind::Call,
        &[],
        &serde_json::to_vec(&ev)?,
    )
    .await?;
    Ok(room)
}

/// Tell the conversation the call is over.
pub async fn end(
    link: &Link,
    st: &mut State,
    provider: &SigilProvider,
    conv: &Conversation,
    room: &[u8; 32],
) -> anyhow::Result<()> {
    let ev = CallEvent {
        action: "end".into(),
        room: hex::encode(room),
    };
    send_event(
        link,
        st,
        provider,
        conv,
        envelope::Kind::Call,
        &[],
        &serde_json::to_vec(&ev)?,
    )
    .await?;
    Ok(())
}

/// One signalling round trip with the forwarding unit on `server`. A
/// `join` costs a token; the rest of a call is free.
pub async fn signal(
    link: &Link,
    st: &mut State,
    server: &str,
    room: &[u8; 32],
    body: Value,
) -> anyhow::Result<Value> {
    let charge = body.get("kind").and_then(Value::as_str) == Some("join");
    let token = if charge { st.take_token()? } else { Vec::new() };
    let req = Request::CallSignal {
        room: *room,
        body: body.to_string().into_bytes(),
        token,
    };
    match link.call(server, &req, None).await? {
        Response::Bytes(b) => {
            let v: Value = serde_json::from_slice(&b)?;
            if let Some(e) = v.get("error").and_then(Value::as_str) {
                anyhow::bail!("forwarding unit: {e}");
            }
            Ok(v)
        }
        Response::Error(s) => anyhow::bail!("server refused call signalling: {s:?}"),
        other => anyhow::bail!("unexpected reply: {other:?}"),
    }
}

/// Join with an SDP offer. Returns the answer and this participant's peer id.
pub async fn join(
    link: &Link,
    st: &mut State,
    conv: &Conversation,
    room: &[u8; 32],
    offer: &str,
) -> anyhow::Result<(String, String)> {
    let v = signal(
        link,
        st,
        &conv.slot_server,
        room,
        json!({"kind": "join", "offer": offer}),
    )
    .await?;
    let answer = v
        .get("answer")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let peer = v
        .get("peer")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok((answer, peer))
}

/// Ask for a pending renegotiation offer. Returns it, if any, and how many
/// peers are in the room.
pub async fn poll(
    link: &Link,
    st: &mut State,
    conv: &Conversation,
    room: &[u8; 32],
    peer: &str,
) -> anyhow::Result<(Option<String>, u64)> {
    let v = signal(
        link,
        st,
        &conv.slot_server,
        room,
        json!({"kind": "poll", "peer": peer}),
    )
    .await?;
    let offer = v.get("offer").and_then(Value::as_str).map(str::to_string);
    let peers = v.get("peers").and_then(Value::as_u64).unwrap_or(0);
    Ok((offer, peers))
}

/// Complete a renegotiation the unit offered.
pub async fn answer(
    link: &Link,
    st: &mut State,
    conv: &Conversation,
    room: &[u8; 32],
    peer: &str,
    answer: &str,
) -> anyhow::Result<()> {
    signal(
        link,
        st,
        &conv.slot_server,
        room,
        json!({"kind": "answer", "peer": peer, "answer": answer}),
    )
    .await?;
    Ok(())
}

/// Leave the room.
pub async fn leave(
    link: &Link,
    st: &mut State,
    conv: &Conversation,
    room: &[u8; 32],
    peer: &str,
) -> anyhow::Result<()> {
    signal(
        link,
        st,
        &conv.slot_server,
        room,
        json!({"kind": "leave", "peer": peer}),
    )
    .await?;
    Ok(())
}
