use super::*;
use std::{cell::RefCell, rc::Rc};
async fn result(request: &IdbRequest) -> Result<JsValue, JsValue> {
    let (send, receive) = futures_channel::oneshot::channel();
    let send = Rc::new(RefCell::new(Some(send)));
    let success = {
        let send = send.clone();
        let request = request.clone();
        Closure::<dyn FnMut(Event)>::new(move |_| {
            if let Some(send) = send.borrow_mut().take() {
                let _ = send.send(request.result());
            }
        })
    };
    let error = Closure::<dyn FnMut(Event)>::new(move |_| {
        if let Some(send) = send.borrow_mut().take() {
            let _ = send.send(Err(fail("Notification storage failed")));
        }
    });
    request.set_onsuccess(Some(success.as_ref().unchecked_ref()));
    request.set_onerror(Some(error.as_ref().unchecked_ref()));
    let value = receive
        .await
        .map_err(|_| fail("Notification storage interrupted"));
    request.set_onsuccess(None);
    request.set_onerror(None);
    value?
}
async fn open() -> Result<IdbDatabase, JsValue> {
    let factory = Reflect::get(&js_sys::global(), &"indexedDB".into())?.dyn_into::<IdbFactory>()?;
    let request = factory.open_with_u32("sigil-notifications", 1)?;
    let upgrade = {
        let request = request.clone();
        Closure::<dyn FnMut(Event)>::new(move |_| {
            if let Ok(db) = request.result().and_then(|v| v.dyn_into::<IdbDatabase>()) {
                if db.create_object_store("state").is_err() {
                    if let Some(tx) = request.transaction() {
                        let _ = tx.abort();
                    }
                }
            }
        })
    };
    request.set_onupgradeneeded(Some(upgrade.as_ref().unchecked_ref()));
    let value = result(&request).await;
    request.set_onupgradeneeded(None);
    value?.dyn_into()
}
async fn finish(tx: &IdbTransaction) -> Result<(), JsValue> {
    let (send, receive) = futures_channel::oneshot::channel();
    let send = Rc::new(RefCell::new(Some(send)));
    let done = {
        let send = send.clone();
        Closure::<dyn FnMut(Event)>::new(move |_| {
            if let Some(send) = send.borrow_mut().take() {
                let _ = send.send(Ok(()));
            }
        })
    };
    let error = Closure::<dyn FnMut(Event)>::new(move |_| {
        if let Some(send) = send.borrow_mut().take() {
            let _ = send.send(Err(fail("Notification storage commit failed")));
        }
    });
    tx.set_oncomplete(Some(done.as_ref().unchecked_ref()));
    tx.set_onabort(Some(error.as_ref().unchecked_ref()));
    tx.set_onerror(Some(error.as_ref().unchecked_ref()));
    let value = receive
        .await
        .map_err(|_| fail("Notification storage interrupted"));
    tx.set_oncomplete(None);
    tx.set_onabort(None);
    tx.set_onerror(None);
    value?
}
pub async fn get(key: &str) -> Result<JsValue, JsValue> {
    let db = open().await?;
    let value = async {
        let tx = db.transaction_with_str_and_mode("state", IdbTransactionMode::Readonly)?;
        result(&tx.object_store("state")?.get(&key.into())?).await
    }
    .await;
    db.close();
    value
}
pub async fn set(key: &str, value: &JsValue) -> Result<(), JsValue> {
    let db = open().await?;
    let value = async {
        let tx = db.transaction_with_str_and_mode("state", IdbTransactionMode::Readwrite)?;
        tx.object_store("state")?.put_with_key(value, &key.into())?;
        finish(&tx).await
    }
    .await;
    db.close();
    value
}
pub async fn compare_clear(key: &str, expected: &str) -> Result<(), JsValue> {
    let db = open().await?;
    let value = async {
        let tx = db.transaction_with_str_and_mode("state", IdbTransactionMode::Readwrite)?;
        let store = tx.object_store("state")?;
        if result(&store.get(&key.into())?)
            .await?
            .as_string()
            .as_deref()
            == Some(expected)
        {
            store.delete(&key.into())?;
        }
        finish(&tx).await
    }
    .await;
    db.close();
    value
}

pub async fn challenge(endpoint: &str, value: &str) -> Result<(), JsValue> {
    let db = open().await?;
    let value = async {
        let tx = db.transaction_with_str_and_mode("state", IdbTransactionMode::Readwrite)?;
        let store = tx.object_store("state")?;
        if result(&store.get(&"endpoint".into())?)
            .await?
            .as_string()
            .as_deref()
            == Some(endpoint)
        {
            store.put_with_key(&value.into(), &"challenge".into())?;
        }
        finish(&tx).await
    }
    .await;
    db.close();
    value
}
