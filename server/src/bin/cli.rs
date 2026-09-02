//! sigil-cli: a command-line Sigil client for testing the server. It speaks
//! the real protocol through an Envoy: registers a name, draws tokens, and
//! exchanges envelopes in a slot derived from a shared epoch secret.

use clap::{Parser, Subcommand};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use sigil_protocol::identity::{ContactCard, Identity};
use sigil_protocol::wire::{Frame, Op, Request, Response, ServerCard};
use sigil_protocol::{bag, envelope, epoch, names, token};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::tungstenite::Message;

#[derive(Parser)]
#[command(name = "sigil-cli")]
struct Cli {
    #[arg(short, long, default_value = "sigil-cli.json")]
    state: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create an identity and remember which Envoy and username to use.
    Init {
        #[arg(long)]
        username: String,
        #[arg(long)]
        envoy: String,
    },
    /// Register the username on its server, then draw a credential and tokens.
    Register {
        #[arg(long, default_value = "")]
        invite: String,
    },
    /// Draw more daily tokens.
    Tokens {
        #[arg(default_value_t = 20)]
        n: u16,
    },
    /// Look a username up and print its card.
    Lookup { username: String },
    /// Send a text event into the slot for this epoch secret.
    Send {
        #[arg(long)]
        epoch: String,
        text: String,
    },
    /// Subscribe to the slot for this epoch secret and print what arrives.
    Listen {
        #[arg(long)]
        epoch: String,
        #[arg(long, default_value_t = 0)]
        count: usize,
    },
    /// Read the slot's history.
    History {
        #[arg(long)]
        epoch: String,
    },
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    identity_seed: String,
    username: String,
    envoy: String,
    device_id: String,
    credential: Option<String>,
    tokens: Vec<String>,
}

impl State {
    fn load(p: &PathBuf) -> anyhow::Result<State> {
        Ok(serde_json::from_str(&std::fs::read_to_string(p)?)?)
    }
    fn save(&self, p: &PathBuf) -> anyhow::Result<()> {
        Ok(std::fs::write(p, serde_json::to_string_pretty(self)?)?)
    }
    fn identity(&self) -> Identity {
        Identity::from_seed(
            &hex::decode(&self.identity_seed)
                .unwrap()
                .try_into()
                .unwrap(),
        )
    }
    fn server(&self) -> String {
        names::parse_username(&self.username)
            .map(|(_, s)| s.to_string())
            .unwrap()
    }
    fn take_token(&mut self) -> anyhow::Result<Vec<u8>> {
        let t = self
            .tokens
            .pop()
            .ok_or_else(|| anyhow::anyhow!("out of tokens; run `tokens`"))?;
        Ok(hex::decode(t)?)
    }
}

/// A connection to the Envoy: bags in, responses and deliveries out.
struct Link {
    tx: mpsc::Sender<Frame>,
    waiting: Arc<Mutex<HashMap<u32, oneshot::Sender<Vec<u8>>>>>,
    deliveries: Mutex<mpsc::Receiver<Frame>>,
    next_id: std::sync::atomic::AtomicU32,
    envoy_http: String,
}

impl Link {
    async fn connect(envoy_ws: &str, device_id: &str) -> anyhow::Result<Link> {
        let url = format!("{envoy_ws}?device={device_id}");
        let (ws, _) = tokio_tungstenite::connect_async(&url).await?;
        let (mut sink, mut source) = ws.split();
        let (tx, mut rx) = mpsc::channel::<Frame>(64);
        let (dtx, drx) = mpsc::channel::<Frame>(256);
        let waiting: Arc<Mutex<HashMap<u32, oneshot::Sender<Vec<u8>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let w2 = waiting.clone();
        tokio::spawn(async move {
            while let Some(f) = rx.recv().await {
                if sink.send(Message::Binary(f.encode().into())).await.is_err() {
                    break;
                }
            }
        });
        tokio::spawn(async move {
            while let Some(Ok(Message::Binary(b))) = source.next().await {
                match Frame::decode(&b) {
                    Ok(Frame::BagResponse { id, response }) => {
                        if let Some(s) = w2.lock().await.remove(&id) {
                            let _ = s.send(response);
                        }
                    }
                    Ok(f @ Frame::Deliver { .. }) | Ok(f @ Frame::Nonce { .. }) => {
                        let _ = dtx.send(f).await;
                    }
                    _ => {}
                }
            }
        });
        let envoy_http = envoy_ws
            .replacen("ws", "http", 1)
            .trim_end_matches("/envoy")
            .to_string();
        Ok(Link {
            tx,
            waiting,
            deliveries: Mutex::new(drx),
            next_id: 1.into(),
            envoy_http,
        })
    }

    async fn server_card(&self, server: &str) -> anyhow::Result<ServerCard> {
        let bytes = reqwest::get(format!("{}/info/{server}", self.envoy_http))
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        if bytes.len() < 64 {
            anyhow::bail!("short card");
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
        Ok(card)
    }

    /// Seal a request into a bag, send it, open the response.
    async fn call(
        &self,
        card: &ServerCard,
        req: &Request,
        bind: Option<[u8; 32]>,
    ) -> anyhow::Result<Response> {
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
                server: card.hostname.clone(),
                bind_handle: bind,
                bag: bag_bytes,
            })
            .await?;
        let sealed = tokio::time::timeout(std::time::Duration::from_secs(30), r).await??;
        if sealed.is_empty() {
            anyhow::bail!("no response from server");
        }
        let plain = bag::open_response(&keys, &sealed).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        Response::decode(op, &plain).map_err(|e| anyhow::anyhow!("{e:?}"))
    }
}

fn ok(resp: Response) -> anyhow::Result<Response> {
    if let Response::Error(s) = resp {
        anyhow::bail!("server said {s:?}");
    }
    Ok(resp)
}

async fn draw_tokens(link: &Link, card: &ServerCard, st: &mut State, n: u16) -> anyhow::Result<()> {
    let verifier =
        token::Verifier::from_spki(&card.token_key).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let cred = hex::decode(
        st.credential
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no credential"))?,
    )?;
    let mut rng = Rng;
    let pend: Vec<token::Pending> = (0..n)
        .map(|_| verifier.blind(&mut rng, rand::random()).unwrap())
        .collect();
    let resp = ok(link
        .call(
            card,
            &Request::TokenIssue {
                credential: cred,
                blinded: pend.iter().map(|p| p.blinded.clone()).collect(),
            },
            None,
        )
        .await?)?;
    let Response::TokenIssue { blind_sigs } = resp else {
        anyhow::bail!("unexpected")
    };
    for (p, bs) in pend.iter().zip(blind_sigs) {
        let t = verifier
            .finalize(p, &bs)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        st.tokens.push(hex::encode(t.encode()));
    }
    Ok(())
}

/// The OS RNG behind the trait generation blind RSA wants.
struct Rng;
impl rand_core10::TryRng for Rng {
    type Error = core::convert::Infallible;
    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(rand::random())
    }
    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        Ok(rand::random())
    }
    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, dst);
        Ok(())
    }
}
impl rand_core10::TryCryptoRng for Rng {}

fn parse_epoch(s: &str) -> anyhow::Result<[u8; 32]> {
    hex::decode(s)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("epoch must be 32 bytes hex"))
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Init { username, envoy } => {
            names::parse_username(&username).map_err(|_| anyhow::anyhow!("bad username"))?;
            let st = State {
                identity_seed: hex::encode(rand::random::<[u8; 32]>()),
                username,
                envoy,
                device_id: hex::encode(rand::random::<[u8; 32]>()),
                ..Default::default()
            };
            st.save(&cli.state)?;
            let id = st.identity();
            println!(
                "identity {}",
                sigil_protocol::identity::fingerprint_display(
                    &sigil_protocol::identity::fingerprint(&id.public())
                )
            );
        }
        Cmd::Register { invite } => {
            let mut st = State::load(&cli.state)?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            let card = link.server_card(&st.server()).await?;
            let id = st.identity();
            let contact = ContactCard {
                username: st.username.clone(),
                identity_pub: id.public(),
                kem_pub: id.kem.public().to_vec(),
                slot_server: st.server(),
                flags: 0,
            };
            ok(link
                .call(
                    &card,
                    &Request::NameRegister {
                        card: contact.sign(&id),
                        gate: invite.into_bytes(),
                        token: vec![],
                    },
                    None,
                )
                .await?)?;
            println!("registered {}", st.username);
            // credential
            let verifier = token::Verifier::from_spki(&card.token_key)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            let _ = verifier;
            // the credential key is separate; fetch it by asking for a credential and finalising under it
            let cred_spki = credential_key(&link, &card).await?;
            let cv =
                token::Verifier::from_spki(&cred_spki).map_err(|e| anyhow::anyhow!("{e:?}"))?;
            let pending = cv
                .blind(&mut Rng, rand::random())
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            let mut msg = b"sigil v1 credential".to_vec();
            msg.extend_from_slice(&pending.blinded);
            let sig = ed25519_dalek::Signer::sign(&id.signing, &msg).to_bytes();
            let resp = ok(link
                .call(
                    &card,
                    &Request::TokenCredential {
                        identity_pub: id.public(),
                        sig,
                        gate: vec![],
                        blinded: pending.blinded.clone(),
                    },
                    None,
                )
                .await?)?;
            let Response::TokenCredential { blind_sig } = resp else {
                anyhow::bail!("unexpected")
            };
            let cred = cv
                .finalize(&pending, &blind_sig)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            st.credential = Some(hex::encode(cred.encode()));
            draw_tokens(&link, &card, &mut st, 20).await?;
            st.save(&cli.state)?;
            println!("credential ok, {} tokens", st.tokens.len());
        }
        Cmd::Tokens { n } => {
            let mut st = State::load(&cli.state)?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            let card = link.server_card(&st.server()).await?;
            draw_tokens(&link, &card, &mut st, n).await?;
            st.save(&cli.state)?;
            println!("{} tokens", st.tokens.len());
        }
        Cmd::Lookup { username } => {
            let st = State::load(&cli.state)?;
            let (local, server) =
                names::parse_username(&username).map_err(|_| anyhow::anyhow!("bad username"))?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            let card = link.server_card(server).await?;
            let resp = ok(link
                .call(
                    &card,
                    &Request::NameLookup {
                        localpart: local.to_string(),
                    },
                    None,
                )
                .await?)?;
            let Response::Bytes(b) = resp else {
                anyhow::bail!("unexpected")
            };
            let c = ContactCard::verify(&b).map_err(|e| anyhow::anyhow!("{e:?}"))?;
            println!(
                "{} on {} fingerprint {}",
                c.username,
                c.slot_server,
                sigil_protocol::identity::fingerprint_display(
                    &sigil_protocol::identity::fingerprint(&c.identity_pub)
                )
            );
        }
        Cmd::Send { epoch: e, text } => {
            let mut st = State::load(&cli.state)?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            let card = link.server_card(&st.server()).await?;
            let ep = epoch::derive(&parse_epoch(&e)?);
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis() as u64;
            let ev = envelope::Event {
                kind: envelope::Kind::Text as u16,
                ts_ms: ts,
                reference: vec![],
                body: text.into_bytes(),
            };
            let nonce: [u8; 24] = rand::random();
            let sealed = envelope::seal(&ep.envelope_key, &ep.address, &nonce, &ev.encode())
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            let sig = epoch::sign_put(&ep.write_key, &ep.address, &sealed);
            let token = st.take_token()?;
            st.save(&cli.state)?; // a token is gone the moment it leaves the wallet
            let resp = ok(link
                .call(
                    &card,
                    &Request::SlotPut {
                        address: ep.address,
                        write_pub: ep.write_pub,
                        sig,
                        envelope: sealed,
                        token,
                    },
                    None,
                )
                .await?)?;
            if let Response::SlotPut { seq } = resp {
                println!("sent as seq {seq}");
            }
        }
        Cmd::Listen { epoch: e, count } => {
            let mut st = State::load(&cli.state)?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            let card = link.server_card(&st.server()).await?;
            let ep = epoch::derive(&parse_epoch(&e)?);
            let handle: [u8; 32] = rand::random();
            let token = st.take_token()?;
            st.save(&cli.state)?;
            ok(link
                .call(
                    &card,
                    &Request::SlotSubscribe {
                        address: ep.address,
                        wake_handle: handle,
                        proof: vec![],
                        token,
                    },
                    Some(handle),
                )
                .await?)?;
            eprintln!("listening on slot {}…", hex::encode(&ep.address[..8]));
            // Backfill: anything written before the subscription landed.
            let mut seen = std::collections::HashSet::new();
            let mut got = 0;
            let print =
                |seq: u64, env: &[u8]| match envelope::open(&ep.envelope_key, &ep.address, env)
                    .and_then(|p| envelope::Event::decode(&p))
                {
                    Ok(ev) => {
                        println!("{seq} [{}] {}", ev.ts_ms, String::from_utf8_lossy(&ev.body))
                    }
                    Err(e) => println!("{seq} (undecryptable envelope: {e:?})"),
                };
            if let Ok(Response::SlotGet { items, .. }) = link
                .call(
                    &card,
                    &Request::SlotGet {
                        read_cap: ep.read_cap,
                        write_pub: ep.write_pub,
                        after_seq: 0,
                        limit: 64,
                    },
                    None,
                )
                .await
            {
                for it in items {
                    if seen.insert(it.seq) {
                        print(it.seq, &it.envelope);
                        got += 1;
                    }
                }
            }
            let mut rx = link.deliveries.lock().await;
            while count == 0 || got < count {
                let Some(f) = rx.recv().await else { break };
                if let Frame::Deliver {
                    wake_handle,
                    queue_seq,
                    slot_seq,
                    envelope: env,
                } = f
                {
                    link.tx
                        .send(Frame::Ack {
                            wake_handle,
                            queue_seq,
                        })
                        .await?;
                    if seen.insert(slot_seq) {
                        print(slot_seq, &env);
                        got += 1;
                    }
                }
            }
        }
        Cmd::History { epoch: e } => {
            let st = State::load(&cli.state)?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            let card = link.server_card(&st.server()).await?;
            let ep = epoch::derive(&parse_epoch(&e)?);
            let resp = ok(link
                .call(
                    &card,
                    &Request::SlotGet {
                        read_cap: ep.read_cap,
                        write_pub: ep.write_pub,
                        after_seq: 0,
                        limit: 64,
                    },
                    None,
                )
                .await?)?;
            if let Response::SlotGet { items, .. } = resp {
                for it in items {
                    match envelope::open(&ep.envelope_key, &ep.address, &it.envelope)
                        .and_then(|p| envelope::Event::decode(&p))
                    {
                        Ok(ev) => println!(
                            "{} [{}] {}",
                            it.seq,
                            ev.ts_ms,
                            String::from_utf8_lossy(&ev.body)
                        ),
                        Err(e) => println!("{} (undecryptable: {e:?})", it.seq),
                    }
                }
            }
        }
    }
    Ok(())
}

/// The credential-issuing key is not in the v1 server card (it lands in
/// v1.1), so the CLI fetches it from the server's `/credential-key` route.
async fn credential_key(link: &Link, card: &ServerCard) -> anyhow::Result<Vec<u8>> {
    let bytes = reqwest::get(format!(
        "{}/info/{}/credential-key",
        link.envoy_http, card.hostname
    ))
    .await?
    .error_for_status()?
    .bytes()
    .await?;
    Ok(bytes.to_vec())
}
