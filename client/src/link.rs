//! The connection to the Envoy: sealed bags in, sealed responses and
//! deliveries out. Server cards are fetched through the Envoy and verified.

use futures_util::{SinkExt, StreamExt};
use sigil_protocol::bag;
use sigil_protocol::wire::{Frame, Op, Request, Response, ServerCard};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::tungstenite::Message;

type WsSink = std::pin::Pin<
    Box<dyn futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Send>,
>;
type WsSource = std::pin::Pin<
    Box<
        dyn futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
            + Send,
    >,
>;

pub struct Link {
    pub tx: mpsc::Sender<Frame>,
    waiting: Arc<Mutex<HashMap<u32, oneshot::Sender<Vec<u8>>>>>,
    nonces: Arc<Mutex<HashMap<String, oneshot::Sender<[u8; 32]>>>>,
    pub deliveries: Mutex<mpsc::Receiver<Frame>>,
    next_id: std::sync::atomic::AtomicU32,
    envoy_http: String,
    cards: Mutex<HashMap<String, ServerCard>>,
    http: reqwest::Client,
}

impl Link {
    pub async fn connect(envoy_ws: &str, device_id: &str) -> anyhow::Result<Link> {
        Self::connect_with(envoy_ws, device_id, None).await
    }

    /// Connect, optionally through a SOCKS5 proxy (`host:port`, for example
    /// a local Tor daemon on 127.0.0.1:9050). Every request, delivery and
    /// card fetch then goes through it.
    pub async fn connect_with(
        envoy_ws: &str,
        device_id: &str,
        proxy: Option<&str>,
    ) -> anyhow::Result<Link> {
        let url = format!("{envoy_ws}?device={device_id}");
        let (sink, source) = dial(&url, proxy).await?;
        let (tx, rx) = mpsc::channel::<Frame>(64);
        let (dtx, drx) = mpsc::channel::<Frame>(256);
        let waiting: Arc<Mutex<HashMap<u32, oneshot::Sender<Vec<u8>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let nonces: Arc<Mutex<HashMap<String, oneshot::Sender<[u8; 32]>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        tokio::spawn(run(
            sink,
            source,
            rx,
            dtx,
            waiting.clone(),
            nonces.clone(),
            url,
            proxy.map(str::to_string),
        ));
        let envoy_http = envoy_ws
            .replacen("ws", "http", 1)
            .trim_end_matches("/envoy")
            .to_string();
        let mut http = reqwest::Client::builder();
        if let Some(px) = proxy {
            http = http.proxy(reqwest::Proxy::all(format!("socks5h://{px}"))?);
        }
        Ok(Link {
            tx,
            waiting,
            nonces,
            deliveries: Mutex::new(drx),
            next_id: 1.into(),
            envoy_http,
            cards: Mutex::new(HashMap::new()),
            http: http.build()?,
        })
    }

    pub async fn server_card(&self, server: &str) -> anyhow::Result<ServerCard> {
        if let Some(c) = self.cards.lock().await.get(server) {
            return Ok(c.clone());
        }
        let bytes = self
            .http
            .get(format!("{}/info/{server}", self.envoy_http))
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        if bytes.len() < 64 {
            anyhow::bail!("short server card");
        }
        let body = &bytes[..bytes.len() - 64];
        let card = ServerCard::decode(body).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let mut msg = b"sigil v1 server card".to_vec();
        msg.extend_from_slice(body);
        let vk = ed25519_dalek::VerifyingKey::from_bytes(&card.signing_pub)?;
        ed25519_dalek::Verifier::verify(
            &vk,
            &msg,
            &ed25519_dalek::Signature::from_slice(&bytes[bytes.len() - 64..])?,
        )?;
        self.cards
            .lock()
            .await
            .insert(server.to_string(), card.clone());
        Ok(card)
    }

    pub async fn credential_key(&self, server: &str) -> anyhow::Result<Vec<u8>> {
        Ok(self
            .http
            .get(format!("{}/info/{server}/credential-key", self.envoy_http))
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?
            .to_vec())
    }

    /// Seal a request into a bag, send it, open the response.
    pub async fn call(
        &self,
        server: &str,
        req: &Request,
        bind: Option<[u8; 32]>,
    ) -> anyhow::Result<Response> {
        let card = self.server_card(server).await?;
        let op = Op::from_u8(req.encode()[0]).unwrap();
        let eseed: [u8; 32] = rand::random();
        let nonce: [u8; 24] = rand::random();
        let (bag_bytes, keys) = bag::seal_request(&card.kem_pub, &eseed, &nonce, &req.encode())
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (s, r) = oneshot::channel();
        self.waiting.lock().await.insert(id, s);
        self.tx
            .send(Frame::Bag {
                id,
                server: server.to_string(),
                bind_handle: bind,
                bag: bag_bytes,
            })
            .await?;
        let sealed = tokio::time::timeout(std::time::Duration::from_secs(30), r).await??;
        if sealed.is_empty() {
            anyhow::bail!("no response from {server}");
        }
        let plain = bag::open_response(&keys, &sealed).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let resp = Response::decode(op, &plain).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        if let Response::Error(s) = resp {
            return Err(ServerRefused {
                server: server.to_string(),
                status: s,
                op,
            }
            .into());
        }
        Ok(resp)
    }

    /// The clocked tier toward the Envoy: send a bag every `period` whether or
    /// not there is anything to say, so the Envoy sees a steady cadence. The
    /// bag is a free `server.info` sealed to `server`; the Envoy cannot tell.
    pub fn start_clock(self: &Arc<Self>, server: String, period: std::time::Duration) {
        let me = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(period);
            loop {
                tick.tick().await;
                let _ = me.call(&server, &Request::ServerInfo, None).await;
            }
        });
    }

    /// The server's current requests-read nonce, via the Envoy.
    pub async fn nonce(&self, server: &str) -> anyhow::Result<[u8; 32]> {
        let (s, r) = oneshot::channel();
        self.nonces.lock().await.insert(server.to_string(), s);
        self.tx
            .send(Frame::Nonce {
                server: server.to_string(),
                nonce: [0; 32],
            })
            .await?;
        let n = tokio::time::timeout(std::time::Duration::from_secs(10), r).await??;
        if n == [0; 32] {
            anyhow::bail!("{server} has not issued a nonce yet; try again in a moment");
        }
        Ok(n)
    }
}

/// Open the WebSocket, directly or through a SOCKS5 proxy. Both kinds become
/// one boxed frame stream and sink, so the rest of the link does not care
/// whether a proxy is in the way.
async fn dial(url: &str, proxy: Option<&str>) -> anyhow::Result<(WsSink, WsSource)> {
    Ok(match proxy {
        None => {
            let (ws, _) = tokio_tungstenite::connect_async(url).await?;
            let (sk, src) = ws.split();
            (Box::pin(sk), Box::pin(src))
        }
        Some(px) => {
            let parsed = url::Url::parse(url)?;
            let host = parsed
                .host_str()
                .ok_or_else(|| anyhow::anyhow!("no host"))?
                .to_string();
            let port = parsed.port_or_known_default().unwrap_or(443);
            let tcp = tokio_socks::tcp::Socks5Stream::connect(px, (host.as_str(), port)).await?;
            let (ws, _) = tokio_tungstenite::client_async_tls(url, tcp).await?;
            let (sk, src) = ws.split();
            (Box::pin(sk), Box::pin(src))
        }
    })
}

/// The link's one task: frames out, frames in, and a reconnect with backoff
/// when the socket drops. The Envoy remembers this device's handles and
/// queues, so nothing has to be re-subscribed; deliveries missed while away
/// are drained on reconnect. Bags queued while away go out afterwards; a
/// bag that was in flight when the socket died times out at its caller.
#[allow(clippy::too_many_arguments)]
async fn run(
    mut sink: WsSink,
    mut source: WsSource,
    mut rx: mpsc::Receiver<Frame>,
    dtx: mpsc::Sender<Frame>,
    waiting: Arc<Mutex<HashMap<u32, oneshot::Sender<Vec<u8>>>>>,
    nonces: Arc<Mutex<HashMap<String, oneshot::Sender<[u8; 32]>>>>,
    url: String,
    proxy: Option<String>,
) {
    let mut backoff = std::time::Duration::from_millis(1500);
    loop {
        loop {
            tokio::select! {
                f = rx.recv() => match f {
                    Some(f) => {
                        if sink.send(Message::Binary(f.encode().into())).await.is_err() {
                            break;
                        }
                    }
                    None => return,
                },
                m = source.next() => match m {
                    Some(Ok(Message::Binary(b))) => match Frame::decode(&b) {
                        Ok(Frame::BagResponse { id, response }) => {
                            if let Some(s) = waiting.lock().await.remove(&id) {
                                let _ = s.send(response);
                            }
                        }
                        Ok(Frame::Nonce { server, nonce }) => {
                            if let Some(s) = nonces.lock().await.remove(&server) {
                                let _ = s.send(nonce);
                            }
                        }
                        Ok(f @ Frame::Deliver { .. }) => {
                            let _ = dtx.send(f).await;
                        }
                        _ => {}
                    },
                    Some(Ok(_)) => {} // ping, pong, text: nothing to do
                    Some(Err(_)) | None => break,
                },
            }
        }
        loop {
            tokio::time::sleep(backoff).await;
            match dial(&url, proxy.as_deref()).await {
                Ok((s, src)) => {
                    sink = s;
                    source = src;
                    backoff = std::time::Duration::from_millis(1500);
                    break;
                }
                Err(_) => backoff = (backoff * 2).min(std::time::Duration::from_secs(30)),
            }
        }
    }
}

/// A server's error status, as the error a call fails with; callers that
/// care which status it was downcast to this.
#[derive(Debug)]
pub struct ServerRefused {
    pub server: String,
    pub status: sigil_protocol::wire::Status,
    pub op: sigil_protocol::wire::Op,
}

impl std::fmt::Display for ServerRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} said {:?} to {:?}", self.server, self.status, self.op)
    }
}

impl std::error::Error for ServerRefused {}
