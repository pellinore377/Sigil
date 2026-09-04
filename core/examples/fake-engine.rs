//! Fake sigil-engine: canned rooms, spaces, timeline and call state over the
//! JSON-lines protocol (engine/docs/protocol.md), so the QML can be developed
//! without a Matrix account. Every identifier here is a synthetic example.org
//! placeholder, not a real address.
use std::sync::{Arc, LazyLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rand::Rng;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;

const ME: &str = "@dave:example.org";
const USERS: [(&str, &str); 4] = [
    ("@alice:example.org", "Alice"),
    ("@bob:example.org", "Bob"),
    ("@carol:example.org", "Carol"),
    (ME, "Dave"),
];

/// Fixed at startup so every canned timestamp in one run agrees.
static NOW: LazyLock<i64> = LazyLock::new(now_ms);

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64
}

fn shm_path() -> String {
    format!("{}/sigil/video-test.shm", std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into()))
}

#[derive(Default)]
struct R {
    dm: bool,
    unread: i64,
    hl: i64,
    fav: bool,
    low: bool,
    invite: bool,
    parents: Vec<&'static str>,
    call: bool,
}

fn room(i: i64, name: &str, o: R) -> Value {
    json!({
        "id": format!("!room{i}:example.org"),
        "name": name,
        "topic": if o.dm { String::new() } else { format!("Topic of {name}") },
        "avatarPath": "",
        "canonicalAlias": if o.dm { String::new() } else { format!("#{}:example.org", name.to_lowercase().replace(' ', "-")) },
        "isDm": o.dm,
        "dmUserId": if o.dm { json!(USERS[(i % 3) as usize].0) } else { json!(null) },
        "isSpace": false,
        "spaceParents": o.parents,
        "isEncrypted": true,
        "isInvite": o.invite,
        "inviter": if o.invite { json!("@alice:example.org") } else { json!(null) },
        "isFavourite": o.fav,
        "isLowPriority": o.low,
        "joinedMembers": if o.dm { 2 } else { 12 },
        "unread": o.unread,
        "highlights": o.hl,
        "unreadMessages": o.unread,
        "markedUnread": false,
        "lastMessage": {
            "kind": "text", "sender": "@alice:example.org", "senderName": "Alice",
            "body": format!("Latest in {name}"), "ts": *NOW - i * 60000
        },
        "lastActivityTs": *NOW - i * 60000,
        "hasActiveCall": o.call,
        "callParticipants": []
    })
}

fn rooms() -> Vec<Value> {
    vec![
        room(1, "Ops", R { unread: 3, hl: 1, ..R::default() }),
        room(2, "Alice", R { dm: true, unread: 1, ..R::default() }),
        room(3, "Random", R { fav: true, ..R::default() }),
        room(4, "Bob", R { dm: true, ..R::default() }),
        room(5, "Announcements", R { low: true, ..R::default() }),
        room(6, "Design", R { parents: vec!["!space1:example.org"], ..R::default() }),
        room(7, "Project X", R { invite: true, ..R::default() }),
        room(8, "Family", R { call: true, ..R::default() }),
    ]
}

fn spaces() -> Vec<Value> {
    vec![json!({
        "id": "!space1:example.org", "name": "Work", "avatarPath": "", "level": 0,
        "children": ["!room6:example.org", "!room1:example.org"], "childrenCount": 2
    })]
}

fn item(i: i64, sender: (&str, &str), body: &str, kind: &str, extra: &[(&str, Value)]) -> Value {
    let (uid, name) = sender;
    let own = uid == ME;
    let mut o = json!({
        "id": format!("$ev{i}"), "kind": kind, "eventId": format!("$ev{i}"), "txnId": null,
        "sender": uid, "senderName": name, "senderAvatarPath": "", "ts": *NOW - (200 - i) * 90000,
        "isOwn": own, "isHighlighted": false, "body": body, "isEdited": false, "reactions": [],
        "sendState": "sent", "sendError": "", "readBy": [],
        "can": {"edit": own, "reply": true, "redact": own, "react": true}
    });
    for (k, v) in extra {
        o[*k] = v.clone();
    }
    o
}

/// Arm order is load-bearing: the modulo cases are tested first, so a fixed
/// index that also matches one of them never reaches its own arm.
fn timeline() -> Vec<Value> {
    let mut items = vec![json!({"id": "$start", "kind": "timelineStart"})];
    for i in 1..60i64 {
        let u = USERS[((i / 3) % 4) as usize];
        items.push(if i % 17 == 0 {
            let n = (i / 17 - 1) % 3 + 1;
            item(i, u, "", "image", &[("media", json!({
                "mxc": "mxc://x/y", "encrypted": true, "filename": "photo.png", "mime": "image/png",
                "width": 800, "height": 600, "size": 123456,
                "thumbnailPath": format!("/tmp/sigil-fake-img{n}.png"), "path": format!("/tmp/sigil-fake-img{n}.png")
            }))])
        } else if i % 11 == 0 {
            item(i, u, "Alice joined the room", "membership", &[("stateText", json!("Alice joined the room"))])
        } else if i % 13 == 0 {
            item(i, u, "**bold** and `code` https://example.org", "text", &[("html", json!(
                "<b>bold</b> and <code>code</code> <a href=\"https://example.org\">https://example.org</a>"
            ))])
        } else if i % 7 == 0 {
            item(i, u, "Replying to that", "text", &[
                ("replyTo", json!({
                    "eventId": format!("$ev{}", i - 1), "sender": "@bob:example.org",
                    "senderName": "Bob", "kind": "text", "body": "the earlier message"
                })),
                ("reactions", json!([
                    {"key": "👍", "count": 2, "senders": [ME, "@bob:example.org"]},
                    {"key": "🎉", "count": 1, "senders": ["@alice:example.org"]}
                ])),
            ])
        } else if i % 19 == 0 {
            item(i, u, "report.pdf", "file", &[("media", json!({
                "mxc": "mxc://x/f", "encrypted": true, "filename": "report.pdf",
                "mime": "application/pdf", "size": 3456789, "path": null
            }))])
        } else if i == 54 {
            item(i, USERS[0], "", "image", &[("media", json!({
                "mxc": "mxc://x/g", "encrypted": false, "filename": "party.gif", "mime": "image/gif",
                "width": 200, "height": 200, "size": 5555,
                "thumbnailPath": "/tmp/sigil-fake.gif", "path": "/tmp/sigil-fake.gif"
            }))])
        } else if i == 51 {
            item(i, USERS[0], "clip.mp4", "video", &[("media", json!({
                "mxc": "mxc://x/v", "encrypted": false, "filename": "clip.mp4", "mime": "video/mp4",
                "width": 640, "height": 360, "size": 987654,
                "thumbnailPath": "/tmp/sigil-fake-img1.png", "path": null
            }))])
        } else if i == 49 {
            item(i, u, "check this out https://example.org/article", "text", &[])
        } else if i == 53 {
            item(i, USERS[3], "own image with caption", "image", &[("media", json!({
                "mxc": "mxc://x/o1", "encrypted": true, "filename": "photo.png", "mime": "image/png",
                "width": 800, "height": 600, "size": 1234,
                "thumbnailPath": "/tmp/sigil-fake-img2.png", "path": "/tmp/sigil-fake-img2.png"
            }))])
        } else if i == 50 {
            item(i, USERS[3], "photo.png", "image", &[("media", json!({
                "mxc": "mxc://x/o2", "encrypted": true, "filename": "photo.png", "mime": "image/png",
                "width": 800, "height": 600, "size": 1234,
                "thumbnailPath": "/tmp/sigil-fake-img3.png", "path": "/tmp/sigil-fake-img3.png"
            }))])
        } else if i == 58 {
            item(i, USERS[3], "this one failed to send", "text",
                 &[("sendState", json!("failed")), ("sendError", json!("offline"))])
        } else if i == 59 {
            item(i, USERS[3], "PXL.mp4", "video", &[
                ("sendState", json!("sending")),
                ("txnId", json!("txn-vid")),
                ("media", json!({
                    "mxc": "", "filename": "PXL.mp4", "mime": "video/mp4", "size": 9999,
                    "width": 640, "height": 360, "thumbnailPath": "/tmp/sigil-fake-img1.png", "path": null
                })),
            ])
        } else if i == 55 {
            json!({"id": "$rm", "kind": "readMarker"})
        } else {
            let filler = "a fairly long line that should wrap nicely across the width of the timeline column when rendered in the panel. "
                .repeat((1 + i % 3) as usize);
            item(i, u, &format!("Message number {i} — {filler}"), "text", &[])
        });
    }
    items
}

type Writer = Arc<Mutex<OwnedWriteHalf>>;

async fn push(w: &Writer, obj: Value) {
    let mut g = w.lock().await;
    let _ = g.write_all(format!("{obj}\n").as_bytes()).await;
    let _ = g.flush().await;
}

fn status(logged: bool) -> Value {
    json!({
        "event": "status", "session": if logged { "loggedIn" } else { "loggedOut" },
        "homeserver": "https://matrix.example.org", "serverName": "example.org",
        "userId": if logged { ME } else { "" }, "deviceId": "SIGILDEV",
        "displayName": if logged { "Dave" } else { "" }, "avatarPath": "",
        "sync": if logged { "running" } else { "offline" }, "syncError": "",
        "verified": true, "login": {"url": ""}, "lastError": ""
    })
}

fn idle_call() -> Value {
    json!({"event": "call.state", "state": "idle", "roomId": "", "participants": [], "local": {}, "incoming": null, "error": ""})
}

async fn handle(stream: UnixStream, login_page: bool) {
    let (rd, wr) = stream.into_split();
    let w: Writer = Arc::new(Mutex::new(wr));
    let mut logged = !login_page;

    push(&w, json!({"event": "hello", "protocol": 1, "engine": "fake", "pid": std::process::id()})).await;
    push(&w, status(logged)).await;
    push(&w, json!({"event": "recovery.status", "recovery": "enabled", "backup": "enabled", "verified": true})).await;
    if logged {
        push(&w, json!({"event": "rooms.list", "loaded": true, "rooms": rooms()})).await;
        push(&w, json!({"event": "spaces.tree", "spaces": spaces()})).await;
    }
    push(&w, idle_call()).await;

    let mut lines = BufReader::new(rd).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(m) = serde_json::from_str::<Value>(&line) else { continue };
        let mut res = json!({});
        match m["req"].as_str().unwrap_or("") {
            "room.open" => {
                push(&w, json!({"event": "timeline.reset", "roomId": m["roomId"], "items": timeline()})).await;
                push(&w, json!({"event": "timeline.paginationState", "roomId": m["roomId"], "state": "timelineStart"})).await;
            }
            "message.send" => {
                let n: i64 = rand::rng().random_range(1000..=9999);
                let room_id = m["roomId"].clone();
                let body = m["body"].as_str().unwrap_or("").to_string();
                let sending = item(n, USERS[3], &body, "text", &[("sendState", json!("sending"))]);
                push(&w, json!({"event": "timeline.diff", "roomId": room_id.clone(), "ops": [{"op": "pushBack", "item": sending}]})).await;
                let w2 = w.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(600)).await;
                    let sent = item(n, USERS[3], &body, "text", &[]);
                    push(&w2, json!({"event": "timeline.diff", "roomId": room_id, "ops": [{"op": "set", "index": 60, "item": sent}]})).await;
                });
            }
            "room.members" => {
                let members: Vec<Value> = USERS.iter().enumerate().map(|(i, (u, n))| json!({
                    "userId": u, "displayName": n, "avatarPath": "",
                    "powerLevel": if i == 0 { 100 } else { 0 }, "membership": "join"
                })).collect();
                res = json!({"members": members});
            }
            "users.search" => {
                let q = m["query"].as_str().unwrap_or("").to_lowercase();
                // `results`, as the real engine answers and protocol.md
                // says; this mock used to say `users`, which is how a
                // frontend reading the wrong key went unnoticed.
                let results: Vec<Value> = USERS.iter()
                    .filter(|(u, n)| n.to_lowercase().contains(&q) || u.to_lowercase().contains(&q))
                    .map(|(u, n)| json!({"userId": u, "displayName": n, "avatarPath": "", "known": true}))
                    .collect();
                res = json!({"results": results});
            }
            "login.start" => {
                logged = true;
                push(&w, json!({"event": "login.finished", "userId": ME})).await;
                push(&w, status(logged)).await;
                push(&w, json!({"event": "rooms.list", "loaded": true, "rooms": rooms()})).await;
                push(&w, json!({"event": "spaces.tree", "spaces": spaces()})).await;
                res = json!({"url": "https://example.org/auth"});
            }
            "call.devices" => {
                res = json!({
                    "mics": [{"id": "m1", "name": "Mic", "default": true}],
                    "speakers": [{"id": "s1", "name": "Speakers", "default": true}],
                    "cameras": [{"id": "0", "name": "Webcam"}],
                    "selected": {}
                });
            }
            "call.start" => {
                let shm = shm_path();
                push(&w, json!({
                    "event": "call.state", "state": "connected", "step": "", "roomId": m["roomId"],
                    "intent": "video", "since": now_ms(), "encrypted": true, "error": "",
                    "local": {
                        "participantId": format!("{ME}:SIGILDEV"), "micMuted": false, "cameraOn": true,
                        "screenSharing": false,
                        "tracks": [{"key": "local-camera", "kind": "camera", "shmPath": shm, "width": 640, "height": 360}]
                    },
                    "participants": [
                        {
                            "participantId": "@alice:example.org:DEV", "userId": "@alice:example.org",
                            "deviceId": "DEV", "displayName": "Alice", "avatarPath": "", "micMuted": false,
                            "cameraOn": true, "screenSharing": false, "speaking": true, "quality": "good",
                            "tracks": [{"key": "remote-camera", "kind": "camera", "shmPath": shm, "width": 640, "height": 360}]
                        },
                        {
                            "participantId": "@bob:example.org:DEV", "userId": "@bob:example.org",
                            "deviceId": "DEV", "displayName": "Bob", "avatarPath": "", "micMuted": true,
                            "cameraOn": false, "screenSharing": false, "speaking": false, "quality": "poor",
                            "tracks": []
                        }
                    ],
                    "incoming": null
                })).await;
            }
            "call.leave" => push(&w, idle_call()).await,
            _ => {}
        }
        if let Some(rid) = m.get("id").filter(|v| !v.is_null()) {
            push(&w, json!({"reply": rid, "ok": true, "result": res})).await;
        }
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let sock = std::env::var("SIGIL_SOCKET").unwrap_or_else(|_| "/tmp/sigil-fake.sock".into());
    let login_page = std::env::args().any(|a| a == "--login-page");
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock)?;
    println!("fake engine on {sock}");
    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(handle(stream, login_page));
    }
}
