//! Passkey ceremonies run in a small same-site window without cross-origin isolation:
//! password-manager extensions cannot show their prompts inside the isolated messenger.
use crate::{fail, get, passkey};
use futures_channel::oneshot;
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::{prelude::*, JsCast};
use web_sys::{BroadcastChannel, MessageEvent};

const CHANNEL: &str = "sigil-passkey-v1";
thread_local! {static WINDOW:RefCell<Option<web_sys::Window>>=const{RefCell::new(None)};}

/// Opens the passkey window inside the click so popup blockers allow it.
#[wasm_bindgen]
pub fn passkey_window_open() -> bool {
    let window = web_sys::window()
        .and_then(|w| w.open_with_url_and_target_and_features("/passkey", "sigil-passkey", "popup,width=460,height=560").ok().flatten());
    let opened = window.is_some();
    WINDOW.with(|slot| *slot.borrow_mut() = window);
    opened
}

fn message(kind: &str, id: &str, body: &str) -> Result<JsValue, JsValue> {
    let value = serde_json::json!({"kind": kind, "id": id, "body": body});
    Ok(JsValue::from_str(&value.to_string()))
}

/// Hands `request` to the passkey window and resolves with its ceremony result.
#[wasm_bindgen]
pub async fn passkey_window_run(kind: String, request: String) -> Result<String, JsValue> {
    if !matches!(kind.as_str(), "create" | "get") {
        return Err(fail("Invalid passkey request"));
    }
    let id = crate::host::request_id()?;
    let channel = BroadcastChannel::new(CHANNEL)?;
    let (send, receive) = oneshot::channel::<Result<String, String>>();
    let send = Rc::new(RefCell::new(Some(send)));
    let offer = message(&kind, &id, &request)?;
    let (output, expected, reply) = (channel.clone(), id.clone(), send.clone());
    let handler = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let Some(raw) = event.data().as_string().filter(|s| s.len() <= 16384) else { return };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else { return };
        match (value["kind"].as_str(), value["id"].as_str()) {
            // A window that loaded after the offer asks for it again.
            (Some("ready"), _) => {
                let _ = output.post_message(&offer);
            }
            (Some("done"), Some(id)) if id == expected => {
                let result = match value["error"].as_str() {
                    Some(error) => Err(error.to_owned()),
                    None => Ok(value["body"].as_str().unwrap_or_default().to_owned()),
                };
                if let Some(send) = reply.borrow_mut().take() {
                    let _ = send.send(result);
                }
            }
            _ => {}
        }
    });
    channel.set_onmessage(Some(handler.as_ref().unchecked_ref()));
    channel.post_message(&message(&kind, &id, &request)?)?;
    // Closing the window without finishing cancels.
    let watcher = send.clone();
    let poll = Closure::<dyn FnMut()>::new(move || {
        let closed = WINDOW.with(|slot| slot.borrow().as_ref().is_none_or(|w| w.closed().unwrap_or(true)));
        if closed {
            if let Some(send) = watcher.borrow_mut().take() {
                let _ = send.send(Err("The passkey window was closed.".into()));
            }
        }
    });
    let window = web_sys::window().ok_or_else(|| fail("Missing window"))?;
    let timer = window.set_interval_with_callback_and_timeout_and_arguments_0(poll.as_ref().unchecked_ref(), 500)?;
    let result = receive.await.unwrap_or_else(|_| Err("The passkey request stopped.".into()));
    window.clear_interval_with_handle(timer);
    channel.set_onmessage(None);
    channel.close();
    drop((handler, poll));
    if let Some(popup) = WINDOW.with(|slot| slot.borrow_mut().take()) {
        let _ = popup.close();
    }
    result.map_err(|error| fail(&error))
}

/// Runs inside the passkey window: waits for a request, then a click, then the ceremony.
#[wasm_bindgen]
pub async fn passkey_window() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| fail("Missing window"))?;
    let document = window.document().ok_or_else(|| fail("Missing document"))?;
    let status = document.get_element_by_id("passkey-status").ok_or_else(|| fail("Missing status"))?;
    let button = document.get_element_by_id("passkey-continue").ok_or_else(|| fail("Missing button"))?;
    let channel = BroadcastChannel::new(CHANNEL)?;
    let (send, receive) = oneshot::channel::<String>();
    let send = Rc::new(RefCell::new(Some(send)));
    let handler = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let Some(raw) = event.data().as_string().filter(|s| s.len() <= 16384) else { return };
        if serde_json::from_str::<serde_json::Value>(&raw).is_ok_and(|v| matches!(v["kind"].as_str(), Some("create" | "get"))) {
            if let Some(send) = send.borrow_mut().take() {
                let _ = send.send(raw);
            }
        }
    });
    channel.set_onmessage(Some(handler.as_ref().unchecked_ref()));
    channel.post_message(&JsValue::from_str(r#"{"kind":"ready"}"#))?;
    let raw = receive.await.map_err(|_| fail("No passkey request"))?;
    let request: serde_json::Value = serde_json::from_str(&raw).map_err(|_| fail("Invalid passkey request"))?;
    let (kind, id, body) = (
        request["kind"].as_str().unwrap_or_default().to_owned(),
        request["id"].as_str().unwrap_or_default().to_owned(),
        request["body"].as_str().unwrap_or_default().to_owned(),
    );
    status.set_text_content(Some(if kind == "create" {
        "Create a passkey so you can get your conversations back on a new device."
    } else {
        "Use your passkey to recover your account."
    }));
    button.set_text_content(Some(if kind == "create" { "Create passkey" } else { "Use passkey" }));
    button.remove_attribute("hidden")?;
    // The ceremony starts from a click in this window, which extensions and browsers require.
    let (clicked, click) = oneshot::channel::<()>();
    let clicked = Rc::new(RefCell::new(Some(clicked)));
    let on_click = Closure::<dyn FnMut()>::new(move || {
        if let Some(clicked) = clicked.borrow_mut().take() {
            let _ = clicked.send(());
        }
    });
    button.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
    click.await.map_err(|_| fail("Cancelled"))?;
    button.set_attribute("hidden", "")?;
    status.set_text_content(Some("Follow your passkey provider's prompt."));
    let result = if kind == "create" { passkey::passkey_create(body).await } else { passkey::passkey_get(body).await };
    let reply = match &result {
        Ok(value) => serde_json::json!({"kind": "done", "id": id, "body": value}),
        Err(error) => serde_json::json!({"kind": "done", "id": id, "error": error.as_string().or_else(|| get(error, "message").ok().and_then(|m| m.as_string())).unwrap_or_else(|| "The passkey step failed.".into())}),
    };
    channel.post_message(&JsValue::from_str(&reply.to_string()))?;
    channel.close();
    drop((handler, on_click));
    let _ = window.close();
    Ok(())
}
