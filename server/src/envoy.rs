//! The Envoy role: the courier. Sees client addresses and sealed bags,
//! never a slot. Holds per-device queues of deliveries.

use crate::config::Config;
use crate::home::Home;
use crate::store::{key2, key_seq, Store, HANDLES, PUSH, QUEUES, QUEUE_META};
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use redb::ReadableTable;
use sigil_protocol::wire::Frame;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// Per handle, the Envoy keeps at most this many undelivered envelopes.
const QUEUE_MAX_ITEMS: u64 = 1000;
/// Minimum gap between pushes to the same offline device.
const PUSH_INTERVAL: Duration = Duration::from_secs(30);

pub struct Envoy {
    pub cfg: Config,
    pub store: Arc<Store>,
    pub id: [u8; 32],
    signing: ed25519_dalek::SigningKey,
    /// When `both`, bags for our own hostname are handled in-process.
    pub local_home: Option<Arc<Home>>,
    devices: DashMap<[u8; 32], mpsc::Sender<Frame>>,
    handles: DashMap<[u8; 32], [u8; 32]>,
    streams: DashMap<String, Arc<StreamState>>,
    http: reqwest::Client,
    last_push: std::sync::Mutex<HashMap<[u8; 32], std::time::Instant>>,
}

pub struct StreamState {
    pub nonce: std::sync::Mutex<Option<[u8; 32]>>,
}

impl Envoy {
    pub fn new(
        cfg: Config,
        store: Arc<Store>,
        local_home: Option<Arc<Home>>,
    ) -> anyhow::Result<Arc<Envoy>> {
        let signing =
            ed25519_dalek::SigningKey::from_bytes(&store.meta_seed("envoy_signing_seed")?);
        let id = signing.verifying_key().to_bytes();
        let handles = DashMap::new();
        {
            let r = store.db.begin_read()?;
            for item in r.open_table(HANDLES)?.iter()? {
                let (k, v) = item?;
                handles.insert(k.value().try_into().unwrap(), v.value().try_into().unwrap());
            }
        }
        Ok(Arc::new(Envoy {
            cfg,
            store,
            id,
            signing,
            local_home,
            devices: DashMap::new(),
            handles,
            streams: DashMap::new(),
            http: reqwest::Client::new(),
            last_push: std::sync::Mutex::new(HashMap::new()),
        }))
    }

    /// Serve one connected device.
    pub async fn serve_device(
        self: Arc<Self>,
        device: [u8; 32],
        socket: axum::extract::ws::WebSocket,
    ) {
        use axum::extract::ws::Message;
        tracing::debug!("device connected");
        let (mut sink, mut source) = socket.split();
        let (tx, mut rx) = mpsc::channel::<Frame>(256);
        self.devices.insert(device, tx.clone());
        // drain queues for every handle this device owns
        if let Err(e) = self.drain(&device, &tx) {
            tracing::warn!("drain failed: {e}");
        }
        let writer = tokio::spawn(async move {
            while let Some(f) = rx.recv().await {
                if sink.send(Message::Binary(f.encode().into())).await.is_err() {
                    break;
                }
            }
        });
        while let Some(Ok(msg)) = source.next().await {
            let Message::Binary(b) = msg else { continue };
            let frame = match Frame::decode(&b) {
                Ok(f) => f,
                Err(e) => {
                    tracing::debug!(
                        "undecodable frame type {} ({} bytes): {e:?}",
                        b.first().copied().unwrap_or(0),
                        b.len()
                    );
                    continue;
                }
            };
            tracing::debug!("frame type {}", b[0]);
            let me = self.clone();
            let tx = tx.clone();
            match frame {
                Frame::Bag {
                    id,
                    server,
                    bind_handle,
                    bag,
                } => {
                    if let Some(h) = bind_handle {
                        me.bind(&device, &h);
                    }
                    tokio::spawn(async move {
                        let jitter = rand::random::<u64>() % me.cfg.jitter_max_ms.max(1);
                        tokio::time::sleep(Duration::from_millis(jitter)).await;
                        let response = me.forward(&server, &bag).await.unwrap_or_default();
                        let _ = tx.send(Frame::BagResponse { id, response }).await;
                    });
                }
                Frame::Ack {
                    wake_handle,
                    queue_seq,
                } => {
                    let _ = me.ack(&wake_handle, queue_seq);
                }
                Frame::Push { kind, token } => {
                    let mut v = vec![kind];
                    v.extend_from_slice(&token);
                    let _ = me.store_push(&device, &v);
                }
                Frame::Release { wake_handle } => {
                    let _ = me.release(&device, &wake_handle);
                }
                Frame::Nonce { server, .. } => {
                    tokio::spawn(async move {
                        let st = me.ensure_stream(&server).await;
                        let nonce = st.nonce.lock().unwrap().unwrap_or([0; 32]);
                        tracing::debug!("nonce for {server}: {}", nonce != [0; 32]);
                        let r = tx.send(Frame::Nonce { server, nonce }).await;
                        tracing::debug!("nonce reply sent: {}", r.is_ok());
                    });
                }
                _ => {}
            }
        }
        self.devices.remove(&device);
        writer.abort();
    }

    fn store_push(&self, device: &[u8; 32], v: &[u8]) -> anyhow::Result<()> {
        let w = self.store.db.begin_write()?;
        w.open_table(PUSH)?.insert(device.as_slice(), v)?;
        w.commit()?;
        Ok(())
    }

    fn bind(&self, device: &[u8; 32], handle: &[u8; 32]) {
        self.handles.insert(*handle, *device);
        let _ = self.bind_persist(device, handle);
    }

    fn bind_persist(&self, device: &[u8; 32], handle: &[u8; 32]) -> anyhow::Result<()> {
        let w = self.store.db.begin_write()?;
        w.open_table(HANDLES)?
            .insert(handle.as_slice(), device.as_slice())?;
        {
            let mut t = w.open_table(QUEUE_META)?;
            let k = key2(device, handle);
            if t.get(k.as_slice())?.is_none() {
                t.insert(
                    k.as_slice(),
                    [1u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0].as_slice(),
                )?;
            }
        }
        w.commit()?;
        Ok(())
    }

    fn release(&self, device: &[u8; 32], handle: &[u8; 32]) -> anyhow::Result<()> {
        self.handles.remove(handle);
        let w = self.store.db.begin_write()?;
        w.open_table(HANDLES)?.remove(handle.as_slice())?;
        w.open_table(QUEUE_META)?
            .remove(key2(device, handle).as_slice())?;
        {
            let mut q = w.open_table(QUEUES)?;
            let lo = key_seq(handle, 0);
            let hi = key_seq(handle, u64::MAX);
            let keys: Vec<Vec<u8>> = q
                .range(lo.as_slice()..=hi.as_slice())?
                .map(|i| i.map(|(k, _)| k.value().to_vec()))
                .collect::<Result<_, _>>()?;
            for k in keys {
                q.remove(k.as_slice())?;
            }
        }
        w.commit()?;
        Ok(())
    }

    /// A delivery arrived from a server for `handle`: queue it, and hand it
    /// to the device at once if connected.
    pub fn enqueue(&self, handle: &[u8; 32], slot_seq: u64, envelope: &[u8]) -> anyhow::Result<()> {
        let Some(device) = self.handles.get(handle).map(|d| *d) else {
            return Ok(());
        };
        let meta_key = key2(&device, handle);
        let w = self.store.db.begin_write()?;
        let seq = {
            let mut m = w.open_table(QUEUE_META)?;
            let cur = m
                .get(meta_key.as_slice())?
                .map(|v| v.value().to_vec())
                .unwrap_or_else(|| vec![1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            let next = u64::from_le_bytes(cur[..8].try_into().unwrap());
            let acked = u64::from_le_bytes(cur[8..16].try_into().unwrap());
            let mut nv = (next + 1).to_le_bytes().to_vec();
            nv.extend_from_slice(&acked.to_le_bytes());
            m.insert(meta_key.as_slice(), nv.as_slice())?;
            let mut stored = slot_seq.to_le_bytes().to_vec();
            stored.extend_from_slice(envelope);
            let mut q = w.open_table(QUEUES)?;
            q.insert(key_seq(handle, next).as_slice(), stored.as_slice())?;
            // Cap the queue: past QUEUE_MAX_ITEMS the oldest go, and the
            // client backfills from the slot on reconnect.
            if next > acked + QUEUE_MAX_ITEMS {
                let lo = key_seq(handle, 0);
                let hi = key_seq(handle, next - QUEUE_MAX_ITEMS);
                let keys: Vec<Vec<u8>> = q
                    .range(lo.as_slice()..=hi.as_slice())?
                    .map(|i| i.map(|(k, _)| k.value().to_vec()))
                    .collect::<Result<_, _>>()?;
                for k in keys {
                    q.remove(k.as_slice())?;
                }
            }
            next
        };
        w.commit()?;
        match self.devices.get(&device) {
            Some(tx) => {
                let _ = tx.try_send(Frame::Deliver {
                    wake_handle: *handle,
                    queue_seq: seq,
                    slot_seq,
                    envelope: envelope.to_vec(),
                });
            }
            None => self.push(&device),
        }
        Ok(())
    }

    /// Wake an offline device through its registered push channel, at most
    /// once per PUSH_INTERVAL while it stays away. The push carries only
    /// this Envoy's hostname.
    fn push(&self, device: &[u8; 32]) {
        let now = std::time::Instant::now();
        {
            let mut last = self.last_push.lock().unwrap();
            if let Some(t) = last.get(device) {
                if now.duration_since(*t) < PUSH_INTERVAL {
                    return;
                }
            }
            last.insert(*device, now);
        }
        let reg = (|| -> anyhow::Result<Option<Vec<u8>>> {
            let r = self.store.db.begin_read()?;
            Ok(r.open_table(PUSH)?
                .get(device.as_slice())?
                .map(|v| v.value().to_vec()))
        })()
        .ok()
        .flatten();
        let Some(reg) = reg else { return };
        if reg.is_empty() {
            return;
        }
        let (kind, token) = (reg[0], reg[1..].to_vec());
        let http = self.http.clone();
        let host = self.cfg.hostname.clone();
        tokio::spawn(async move {
            match kind {
                3 => {
                    // UnifiedPush: POST to the endpoint URL the app registered.
                    if let Ok(url) = String::from_utf8(token) {
                        let _ = http
                            .post(&url)
                            .body(host)
                            .timeout(Duration::from_secs(10))
                            .send()
                            .await;
                    }
                }
                1 | 2 => {
                    // APNs and FCM need operator credentials; Phase 3b.
                    tracing::debug!("push kind {kind} not configured");
                }
                _ => {}
            }
        });
    }

    fn ack(&self, handle: &[u8; 32], upto: u64) -> anyhow::Result<()> {
        let Some(device) = self.handles.get(handle).map(|d| *d) else {
            return Ok(());
        };
        let meta_key = key2(&device, handle);
        let w = self.store.db.begin_write()?;
        {
            let mut m = w.open_table(QUEUE_META)?;
            let cur = m.get(meta_key.as_slice())?.map(|v| v.value().to_vec());
            if let Some(cur) = cur {
                let mut nv = cur[..8].to_vec();
                nv.extend_from_slice(&upto.to_le_bytes());
                m.insert(meta_key.as_slice(), nv.as_slice())?;
            }
            let mut q = w.open_table(QUEUES)?;
            let lo = key_seq(handle, 0);
            let hi = key_seq(handle, upto);
            let keys: Vec<Vec<u8>> = q
                .range(lo.as_slice()..=hi.as_slice())?
                .map(|i| i.map(|(k, _)| k.value().to_vec()))
                .collect::<Result<_, _>>()?;
            for k in keys {
                q.remove(k.as_slice())?;
            }
        }
        w.commit()?;
        Ok(())
    }

    fn drain(&self, device: &[u8; 32], tx: &mpsc::Sender<Frame>) -> anyhow::Result<()> {
        let r = self.store.db.begin_read()?;
        let m = r.open_table(QUEUE_META)?;
        let q = r.open_table(QUEUES)?;
        let lo = key2(device, &[0u8; 32]);
        let hi = key2(device, &[0xffu8; 32]);
        for item in m.range(lo.as_slice()..=hi.as_slice())? {
            let (k, _) = item?;
            let handle: [u8; 32] = k.value()[32..64].try_into().unwrap();
            let qlo = key_seq(&handle, 0);
            let qhi = key_seq(&handle, u64::MAX);
            for qi in q.range(qlo.as_slice()..=qhi.as_slice())? {
                let (qk, qv) = qi?;
                let seq = u64::from_be_bytes(qk.value()[32..40].try_into().unwrap());
                let v = qv.value();
                let slot_seq = u64::from_le_bytes(v[..8].try_into().unwrap());
                let _ = tx.try_send(Frame::Deliver {
                    wake_handle: handle,
                    queue_seq: seq,
                    slot_seq,
                    envelope: v[8..].to_vec(),
                });
            }
        }
        Ok(())
    }

    /// Forward a bag to a server and return its sealed response.
    async fn forward(self: &Arc<Self>, server: &str, bag: &[u8]) -> Option<Vec<u8>> {
        if let Some(home) = &self.local_home {
            if server == home.cfg.hostname {
                return home.handle_bag(bag, &self.id).await;
            }
        }
        let _ = self.ensure_stream(server).await;
        let url = format!("{}/bag", self.cfg.base_url(server));
        let resp = self
            .http
            .post(&url)
            .header("x-sigil-envoy", hex::encode(self.id))
            .body(bag.to_vec())
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        resp.bytes().await.ok().map(|b| b.to_vec())
    }

    /// Make sure a delivery stream to `server` exists; spawn it if not.
    pub async fn ensure_stream(self: &Arc<Self>, server: &str) -> Arc<StreamState> {
        if let Some(st) = self.streams.get(server) {
            return st.clone();
        }
        let st = Arc::new(StreamState {
            nonce: std::sync::Mutex::new(None),
        });
        self.streams.insert(server.to_string(), st.clone());
        let me = self.clone();
        let server = server.to_string();
        let st2 = st.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = me.run_stream(&server, &st2).await {
                    tracing::debug!("stream to {server} ended: {e}");
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
        st
    }

    async fn run_stream(self: &Arc<Self>, server: &str, st: &StreamState) -> anyhow::Result<()> {
        if let Some(home) = &self.local_home {
            if server == home.cfg.hostname {
                // in-process stream
                let (tx, mut rx) = mpsc::channel::<Frame>(1024);
                home.delivery.attach(&home.store, self.id, tx)?;
                let nonce = home.delivery.new_nonce(&self.id);
                *st.nonce.lock().unwrap() = Some(nonce);
                let home2 = home.clone();
                let me = self.clone();
                let mut tick = tokio::time::interval(Duration::from_secs(30));
                loop {
                    tokio::select! {
                        Some(f) = rx.recv() => {
                            if let Frame::Deliver { wake_handle, slot_seq, envelope, .. } = f {
                                let _ = me.enqueue(&wake_handle, slot_seq, &envelope);
                            }
                        }
                        _ = tick.tick() => {
                            *st.nonce.lock().unwrap() = Some(home2.delivery.new_nonce(&me.id));
                        }
                    }
                }
            }
        }
        let base = self.cfg.base_url(server);
        let ws_url = base.replacen("http", "ws", 1) + "/stream";
        let (ws, _) = tokio_tungstenite::connect_async(&ws_url).await?;
        let (mut sink, mut source) = ws.split();
        use tokio_tungstenite::tungstenite::Message;
        sink.send(Message::Binary(self.id.to_vec().into())).await?;
        let challenge = match source.next().await {
            Some(Ok(Message::Binary(b))) if b.len() == 32 => b,
            _ => anyhow::bail!("bad challenge"),
        };
        let sig = ed25519_dalek::Signer::sign(&self.signing, &challenge).to_bytes();
        sink.send(Message::Binary(sig.to_vec().into())).await?;
        while let Some(msg) = source.next().await {
            match msg? {
                Message::Binary(b) => match Frame::decode(&b) {
                    Ok(Frame::Deliver {
                        wake_handle,
                        slot_seq,
                        envelope,
                        ..
                    }) => {
                        let _ = self.enqueue(&wake_handle, slot_seq, &envelope);
                    }
                    Ok(Frame::Keepalive { nonce }) => {
                        *st.nonce.lock().unwrap() = Some(nonce);
                    }
                    _ => {}
                },
                Message::Close(_) => break,
                _ => {}
            }
        }
        anyhow::bail!("closed")
    }
}

// ---------------------------------------------------------------- cover traffic

impl Envoy {
    /// The clocked tier: `per_minute` dummy writes to random addresses on
    /// `server`, paid with tokens from a credential the server grants this
    /// Envoy. The server cannot tell a cover write from a real one.
    pub fn start_cover(self: &Arc<Self>, server: String, per_minute: u32) {
        if per_minute == 0 {
            return;
        }
        let me = self.clone();
        tokio::spawn(async move {
            let period = Duration::from_millis(60_000 / per_minute as u64);
            let mut wallet: Vec<Vec<u8>> = Vec::new();
            let mut credential: Option<Vec<u8>> = None;
            loop {
                tokio::time::sleep(period).await;
                if wallet.is_empty() {
                    match me.cover_tokens(&server, &mut credential).await {
                        Ok(t) => wallet = t,
                        Err(e) => {
                            tracing::debug!("cover tokens from {server}: {e:#}");
                            tokio::time::sleep(Duration::from_secs(60)).await;
                            continue;
                        }
                    }
                }
                let Some(token) = wallet.pop() else { continue };
                if let Err(e) = me.cover_write(&server, token).await {
                    tracing::debug!("cover write to {server}: {e:#}");
                }
            }
        });
    }

    /// A card for `server`, through our own forwarding path.
    async fn server_card(
        self: &Arc<Self>,
        server: &str,
    ) -> anyhow::Result<sigil_protocol::wire::ServerCard> {
        let bytes = if let Some(home) = &self.local_home {
            if home.cfg.hostname == server {
                home.card.clone()
            } else {
                self.http
                    .get(format!("{}/info", self.cfg.base_url(server)))
                    .send()
                    .await?
                    .error_for_status()?
                    .bytes()
                    .await?
                    .to_vec()
            }
        } else {
            self.http
                .get(format!("{}/info", self.cfg.base_url(server)))
                .send()
                .await?
                .error_for_status()?
                .bytes()
                .await?
                .to_vec()
        };
        if bytes.len() < 64 {
            anyhow::bail!("short card");
        }
        sigil_protocol::wire::ServerCard::decode(&bytes[..bytes.len() - 64])
            .map_err(|e| anyhow::anyhow!("{e:?}"))
    }

    /// Seal a request to `server`, forward it as we would a client's, open the reply.
    async fn call(
        self: &Arc<Self>,
        server: &str,
        req: &sigil_protocol::wire::Request,
    ) -> anyhow::Result<sigil_protocol::wire::Response> {
        let card = self.server_card(server).await?;
        let op = sigil_protocol::wire::Op::from_u8(req.encode()[0]).unwrap();
        let (bag, keys) = sigil_protocol::bag::seal_request(
            &card.kem_pub,
            &rand::random(),
            &rand::random(),
            &req.encode(),
        )
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let sealed = self
            .forward(server, &bag)
            .await
            .ok_or_else(|| anyhow::anyhow!("no reply"))?;
        let plain = sigil_protocol::bag::open_response(&keys, &sealed)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let resp = sigil_protocol::wire::Response::decode(op, &plain)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        if let sigil_protocol::wire::Response::Error(s) = resp {
            anyhow::bail!("{server} said {s:?} to {op:?}");
        }
        Ok(resp)
    }

    /// Obtain (once) a credential as an Envoy, then a batch of tokens.
    async fn cover_tokens(
        self: &Arc<Self>,
        server: &str,
        credential: &mut Option<Vec<u8>>,
    ) -> anyhow::Result<Vec<Vec<u8>>> {
        use sigil_protocol::token;
        let mut rng = rand::rngs::OsRng;
        let mut adapter = crate::tokens::RngAdapter(&mut rng);
        if credential.is_none() {
            let spki = {
                let url = format!("{}/credential-key", self.cfg.base_url(server));
                match &self.local_home {
                    Some(h) if h.cfg.hostname == server => {
                        h.tokens.current(&h.store, "credential")?.spki.clone()
                    }
                    _ => self
                        .http
                        .get(&url)
                        .send()
                        .await?
                        .error_for_status()?
                        .bytes()
                        .await?
                        .to_vec(),
                }
            };
            let cv = token::Verifier::from_spki(&spki).map_err(|e| anyhow::anyhow!("{e:?}"))?;
            let pending = cv
                .blind(&mut adapter, rand::random())
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            let mut msg = b"sigil v1 credential".to_vec();
            msg.extend_from_slice(&pending.blinded);
            let sig = ed25519_dalek::Signer::sign(&self.signing, &msg).to_bytes();
            let resp = self
                .call(
                    server,
                    &sigil_protocol::wire::Request::TokenCredential {
                        identity_pub: self.id,
                        sig,
                        gate: b"envoy".to_vec(),
                        blinded: pending.blinded.clone(),
                    },
                )
                .await?;
            let sigil_protocol::wire::Response::TokenCredential { blind_sig } = resp else {
                anyhow::bail!("unexpected")
            };
            *credential = Some(
                cv.finalize(&pending, &blind_sig)
                    .map_err(|e| anyhow::anyhow!("{e:?}"))?
                    .encode(),
            );
        }
        let card = self.server_card(server).await?;
        let verifier =
            token::Verifier::from_spki(&card.token_key).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let pend: Vec<token::Pending> = (0..30)
            .map(|_| verifier.blind(&mut adapter, rand::random()).unwrap())
            .collect();
        let resp = self
            .call(
                server,
                &sigil_protocol::wire::Request::TokenIssue {
                    credential: credential.clone().unwrap(),
                    blinded: pend.iter().map(|p| p.blinded.clone()).collect(),
                },
            )
            .await?;
        let sigil_protocol::wire::Response::TokenIssue { blind_sigs } = resp else {
            anyhow::bail!("unexpected")
        };
        let mut out = Vec::new();
        for (p, bs) in pend.iter().zip(blind_sigs) {
            out.push(
                verifier
                    .finalize(p, &bs)
                    .map_err(|e| anyhow::anyhow!("{e:?}"))?
                    .encode(),
            );
        }
        Ok(out)
    }

    /// One dummy envelope into a slot nobody will ever read.
    async fn cover_write(self: &Arc<Self>, server: &str, token: Vec<u8>) -> anyhow::Result<()> {
        let ep = sigil_protocol::epoch::derive(&rand::random());
        let plain = vec![0u8; 200 + (rand::random::<u8>() as usize) * 3];
        let envelope =
            sigil_protocol::envelope::seal(&ep.envelope_key, &ep.address, &rand::random(), &plain)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let sig = sigil_protocol::epoch::sign_put(&ep.write_key, &ep.address, &envelope);
        self.call(
            server,
            &sigil_protocol::wire::Request::SlotPut {
                address: ep.address,
                write_pub: ep.write_pub,
                sig,
                envelope,
                token,
            },
        )
        .await?;
        Ok(())
    }
}
