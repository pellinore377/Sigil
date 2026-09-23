use crate::{fail, host};
use futures_channel::oneshot;
use serde::{Deserialize, Serialize};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::spawn_local;
use web_sys::{BroadcastChannel, MessageEvent};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Completion {
    request_id: String,
    completion: String,
}
impl Completion {
    fn valid(&self) -> bool {
        [&self.request_id, &self.completion]
            .into_iter()
            .all(|s| s.len() == 64 && s.bytes().all(|v| v.is_ascii_hexdigit()))
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ack {
    request_id: String,
    accepted: bool,
}
struct Listener {
    _channel: BroadcastChannel,
    _handler: Closure<dyn FnMut(MessageEvent)>,
}
impl Drop for Listener {
    fn drop(&mut self) {
        self._channel.close();
    }
}
thread_local! {static LISTENER:RefCell<Option<Listener>>=const{RefCell::new(None)};}
pub fn stop() {
    LISTENER.with(|slot| {
        slot.borrow_mut().take();
    });
}
pub fn listen() -> Result<(), JsValue> {
    let channel = BroadcastChannel::new("sigil-auth-v1")?;
    let output = channel.clone();
    let busy = Rc::new(Cell::new(false));
    let handler = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let Some(raw) = event.data().as_string().filter(|s| s.len() <= 512) else {
            return;
        };
        let Ok(value) = serde_json::from_str::<Completion>(&raw) else {
            return;
        };
        if !value.valid() || busy.replace(true) {
            return;
        }
        let output = output.clone();
        let busy = busy.clone();
        spawn_local(async move {
            let request=serde_json::json!({"command":"callback","request_id":value.request_id,"completion":value.completion}).to_string();
            let accepted = host::browser_command(request)
                .await
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                .is_some_and(|v| v["ok"] == true);
            let ack = serde_json::to_string(&Ack {
                request_id: value.request_id,
                accepted,
            })
            .unwrap();
            let _ = output.post_message(&ack.into());
            busy.set(false);
        });
    });
    channel.set_onmessage(Some(handler.as_ref().unchecked_ref()));
    LISTENER.with(|slot| {
        *slot.borrow_mut() = Some(Listener {
            _channel: channel,
            _handler: handler,
        })
    });
    Ok(())
}
#[wasm_bindgen]
pub async fn complete_browser_auth() -> Result<(), JsValue> {
    if complete().await.is_err() {
        if let Some(message) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("auth-status"))
        {
            message.set_text_content(Some("This sign-in link could not be completed. Return to your original Sigil tab and restart sign-in."));
        }
    }
    Ok(())
}
async fn complete() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| fail("Missing window"))?;
    let hash = window.location().hash()?;
    window
        .history()?
        .replace_state_with_url(&JsValue::NULL, "", Some("/auth/browser"))?;

    let fields: Vec<_> = hash.trim_start_matches('#').split('/').collect();
    if fields.len() != 3 || fields[0] != "oidc" {
        return Err(fail("Invalid sign-in callback"));
    }
    let value = Completion {
        request_id: fields[1].into(),
        completion: fields[2].into(),
    };
    if !value.valid() {
        return Err(fail("Invalid sign-in callback"));
    }
    let channel = BroadcastChannel::new("sigil-auth-v1")?;
    let (send, receive) = oneshot::channel();
    let send = Rc::new(RefCell::new(Some(send)));
    let id = value.request_id.clone();
    let receiver = send.clone();
    let handler = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let Some(raw) = event.data().as_string().filter(|s| s.len() <= 256) else {
            return;
        };
        if let Ok(ack) = serde_json::from_str::<Ack>(&raw) {
            if ack.request_id == id {
                if let Some(send) = receiver.borrow_mut().take() {
                    let _ = send.send(ack.accepted);
                }
            }
        }
    });
    channel.set_onmessage(Some(handler.as_ref().unchecked_ref()));
    let body = zeroize::Zeroizing::new(
        serde_json::to_string(&value).map_err(|_| fail("Invalid callback"))?,
    );
    channel.post_message(&JsValue::from_str(&body))?;
    let output = channel.clone();
    let retry = Closure::<dyn FnMut()>::new(move || {
        let _ = output.post_message(&JsValue::from_str(&body));
    });
    let interval = window.set_interval_with_callback_and_timeout_and_arguments_0(
        retry.as_ref().unchecked_ref(),
        2000,
    )?;
    let timeout = Closure::<dyn FnMut()>::new(move || {
        if let Some(send) = send.borrow_mut().take() {
            let _ = send.send(false);
        }
    });
    let timer = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        timeout.as_ref().unchecked_ref(),
        45_000,
    )?;
    let accepted = receive.await.unwrap_or(false);
    window.clear_interval_with_handle(interval);
    window.clear_timeout_with_handle(timer);
    channel.set_onmessage(None);
    channel.close();
    if let Some(message) = window
        .document()
        .and_then(|d| d.get_element_by_id("auth-status"))
    {
        message.set_text_content(Some(if accepted {
            "You're signed in. You can close this page."
        } else {
            "Return to the original Sigil tab to finish signing in, or restart sign-in there."
        }));
    }
    if accepted {
        let _ = window.close();
    }
    Ok(())
}

// Kotlin/Wasm rejects another window's Window object, so the SSO popup lives here.
thread_local! {static POPUP:RefCell<Option<web_sys::Window>>=const{RefCell::new(None)};}
/// Opens a blank popup inside the click so blockers allow it.
#[wasm_bindgen]
pub fn sso_open() -> bool {
    sso_close();
    let popup = web_sys::window().and_then(|w| w.open_with_url_and_target("about:blank", "_blank").ok().flatten());
    let opened = popup.is_some();
    POPUP.with(|slot| *slot.borrow_mut() = popup);
    opened
}
/// Sends the popup to the provider, or opens a new one when it was blocked or closed.
#[wasm_bindgen]
pub fn sso_navigate(url: &str) -> bool {
    if !url.starts_with("https://") {
        return false;
    }
    let live = POPUP.with(|slot| slot.borrow().as_ref().filter(|p| !p.closed().unwrap_or(true)).cloned());
    match live {
        Some(popup) => {
            let _ = popup.set_opener(&JsValue::NULL);
            popup.location().set_href(url).is_ok() && popup.focus().is_ok()
        }
        None => web_sys::window()
            .and_then(|w| w.open_with_url_and_target_and_features(url, "_blank", "noopener,noreferrer").ok())
            .is_some(),
    }
}
#[wasm_bindgen]
pub fn sso_close() {
    if let Some(popup) = POPUP.with(|slot| slot.borrow_mut().take()) {
        let _ = popup.close();
    }
}
