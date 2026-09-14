use crate::{fail, get, set};
use js_sys::{Array, Object, Uint8Array};
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::JsFuture;
use web_sys::*;
use zeroize::Zeroizing;

struct Database(IdbDatabase);
impl std::ops::Deref for Database {
    type Target = IdbDatabase;
    fn deref(&self) -> &IdbDatabase {
        &self.0
    }
}
impl Drop for Database {
    fn drop(&mut self) {
        self.0.close();
    }
}

async fn result(request: &IdbRequest) -> Result<JsValue, JsValue> {
    let (send, receive) = futures_channel::oneshot::channel();
    let send = Rc::new(RefCell::new(Some(send)));
    let success = {
        let send = send.clone();
        let r = request.clone();
        Closure::<dyn FnMut(Event)>::new(move |_| {
            if let Some(s) = send.borrow_mut().take() {
                let _ = s.send(r.result());
            }
        })
    };
    let error = {
        let send = send.clone();
        Closure::<dyn FnMut(Event)>::new(move |_| {
            if let Some(s) = send.borrow_mut().take() {
                let _ = s.send(Err(fail("Browser storage request failed")));
            }
        })
    };
    request.set_onsuccess(Some(success.as_ref().unchecked_ref()));
    request.set_onerror(Some(error.as_ref().unchecked_ref()));
    let value = receive
        .await
        .map_err(|_| fail("Browser storage interrupted"));
    request.set_onsuccess(None);
    request.set_onerror(None);
    value?
}
async fn commit(tx: &IdbTransaction) -> Result<(), JsValue> {
    let (send, receive) = futures_channel::oneshot::channel();
    let send = Rc::new(RefCell::new(Some(send)));
    let done = {
        let s = send.clone();
        Closure::<dyn FnMut(Event)>::new(move |_| {
            if let Some(s) = s.borrow_mut().take() {
                let _ = s.send(Ok(()));
            }
        })
    };
    let aborted = {
        let s = send.clone();
        Closure::<dyn FnMut(Event)>::new(move |_| {
            if let Some(s) = s.borrow_mut().take() {
                let _ = s.send(Err(fail("Could not save the browser key")));
            }
        })
    };
    tx.set_oncomplete(Some(done.as_ref().unchecked_ref()));
    tx.set_onabort(Some(aborted.as_ref().unchecked_ref()));
    tx.set_onerror(Some(aborted.as_ref().unchecked_ref()));
    let value = receive
        .await
        .map_err(|_| fail("Browser storage interrupted"));
    tx.set_oncomplete(None);
    tx.set_onabort(None);
    tx.set_onerror(None);
    value?
}
async fn open() -> Result<Database, JsValue> {
    let global = js_sys::global().dyn_into::<DedicatedWorkerGlobalScope>()?;
    let factory = global
        .indexed_db()?
        .ok_or_else(|| fail("IndexedDB is unavailable"))?;
    let request = factory.open_with_u32("sigil-device", 1)?;
    let upgrade = {
        let r = request.clone();
        Closure::<dyn FnMut(Event)>::new(move |_| {
            if let Ok(db) = r.result().and_then(|v| v.dyn_into::<IdbDatabase>()) {
                if db.create_object_store("vault").is_err() {
                    if let Some(tx) = r.transaction() {
                        let _ = tx.abort();
                    }
                }
            }
        })
    };
    request.set_onupgradeneeded(Some(upgrade.as_ref().unchecked_ref()));
    let opened = result(&request).await;
    request.set_onupgradeneeded(None);
    Ok(Database(opened?.dyn_into::<IdbDatabase>()?))
}

pub async fn unlock(existing: bool) -> Result<Zeroizing<Vec<u8>>, JsValue> {
    let global = js_sys::global().dyn_into::<DedicatedWorkerGlobalScope>()?;
    let db = open().await?;
    let tx = db.transaction_with_str_and_mode("vault", IdbTransactionMode::Readonly)?;
    let stored = result(&tx.object_store("vault")?.get(&"device".into())?).await?;
    let crypto = global.crypto()?.subtle();
    let value = if stored.is_undefined() {
        if existing {
            db.close();
            return Err(fail(
                "This browser's encryption key is missing. Recover or link this device again.",
            ));
        }
        let usages = Array::of2(&"encrypt".into(), &"decrypt".into());
        let algorithm = Object::new();
        set(&algorithm, "name", &"AES-GCM".into())?;
        set(&algorithm, "length", &256.into())?;
        let key =
            JsFuture::from(crypto.generate_key_with_object(&algorithm, false, &usages)?).await?;
        let mut master = Zeroizing::new(vec![0u8; 32]);
        getrandom::fill(&mut master).map_err(|_| fail("Randomness unavailable"))?;
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).map_err(|_| fail("Randomness unavailable"))?;
        let iv = Uint8Array::from(nonce.as_slice());
        let options = Object::new();
        set(&options, "name", &"AES-GCM".into())?;
        set(&options, "iv", &iv)?;
        set(
            &options,
            "additionalData",
            &Uint8Array::from(b"Sigil/browser-key/v1".as_slice()),
        )?;
        let key = key.dyn_into::<CryptoKey>()?;
        let sealed =
            JsFuture::from(crypto.encrypt_with_object_and_u8_array(&options, &key, &master)?)
                .await?;
        let record = Object::new();
        set(&record, "version", &1.into())?;
        set(&record, "key", &key)?;
        set(&record, "iv", &iv)?;
        set(&record, "sealed", &sealed)?;
        let opts = Object::new();
        set(&opts, "durability", &"strict".into())?;
        let transaction = get(&db.0, "transaction")?.dyn_into::<js_sys::Function>()?;
        let tx = transaction
            .call3(&db.0, &"vault".into(), &"readwrite".into(), &opts)?
            .dyn_into::<IdbTransaction>()?;
        tx.object_store("vault")?
            .add_with_key(&record, &"device".into())?;
        commit(&tx).await?;
        master
    } else {
        if get(&stored, "version")?.as_f64() != Some(1.0) {
            return Err(fail("Unsupported browser key format"));
        }
        let key = get(&stored, "key")?.dyn_into::<CryptoKey>()?;
        if key.extractable() {
            return Err(fail("Invalid browser key protection"));
        }
        let iv = Uint8Array::new(&get(&stored, "iv")?);
        let sealed = Uint8Array::new(&get(&stored, "sealed")?);
        if iv.length() != 12 || sealed.length() != 48 {
            return Err(fail("Invalid browser key record"));
        }
        let options = Object::new();
        set(&options, "name", &"AES-GCM".into())?;
        set(&options, "iv", &iv)?;
        set(
            &options,
            "additionalData",
            &Uint8Array::from(b"Sigil/browser-key/v1".as_slice()),
        )?;
        let plaintext =
            JsFuture::from(crypto.decrypt_with_object_and_buffer_source(&options, &key, &sealed)?)
                .await?;
        let bytes = Uint8Array::new(&plaintext);
        let master = Zeroizing::new(bytes.to_vec());
        bytes.fill(0, 0, bytes.length());
        if master.len() != 32 {
            return Err(fail("Invalid browser key length"));
        }
        master
    };
    db.close();
    Ok(value)
}

fn write_transaction(db: &Database) -> Result<IdbTransaction, JsValue> {
    let options = Object::new();
    set(&options, "durability", &"strict".into())?;
    get(&db.0, "transaction")?
        .dyn_into::<js_sys::Function>()?
        .call3(&db.0, &"vault".into(), &"readwrite".into(), &options)?
        .dyn_into::<IdbTransaction>()
}
pub async fn removing() -> Result<bool, JsValue> {
    let db = open().await?;
    let tx = db.transaction_with_str_and_mode("vault", IdbTransactionMode::Readonly)?;
    Ok(result(&tx.object_store("vault")?.get(&"removal".into())?)
        .await?
        .as_bool()
        == Some(true))
}
pub async fn mark_removal() -> Result<(), JsValue> {
    let db = open().await?;
    let tx = write_transaction(&db)?;
    tx.object_store("vault")?
        .put_with_key(&true.into(), &"removal".into())?;
    commit(&tx).await
}
pub async fn clear() -> Result<(), JsValue> {
    let db = open().await?;
    let tx = write_transaction(&db)?;
    tx.object_store("vault")?.clear()?;
    commit(&tx).await
}
