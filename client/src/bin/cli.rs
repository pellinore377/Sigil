//! sigil-cli: a command-line Sigil client. Registers a name, publishes key
//! packages, starts direct messages by username, accepts requests, sends
//! and receives through MLS-derived slots.

use clap::{Parser, Subcommand};
use sigil_client::provider::SigilProvider;
use sigil_client::{account, conversation, Link, State};
use sigil_protocol::wire::Frame;
use std::path::PathBuf;

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
    /// Register the username, draw a credential and tokens, publish key packages.
    Register {
        #[arg(long, default_value = "")]
        invite: String,
    },
    /// Draw more daily tokens.
    Tokens {
        #[arg(default_value_t = 20)]
        n: u16,
    },
    /// Publish `n` fresh key packages to the shelf.
    Publish {
        #[arg(default_value_t = 10)]
        n: u16,
    },
    /// Look a username up.
    Lookup { username: String },
    /// Start a direct message with a username; `text` is the first message.
    Dm { username: String, text: String },
    /// Wait for incoming requests and print them; `--accept` joins them.
    Requests {
        #[arg(long)]
        accept: bool,
        #[arg(long, default_value_t = 1)]
        count: usize,
    },
    /// List conversations.
    List,
    /// Send a text into conversation `n` (from `list`).
    Send { n: usize, text: String },
    /// Listen on conversation `n`; prints backfill then live messages.
    Listen {
        n: usize,
        #[arg(long, default_value_t = 0)]
        count: usize,
    },
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn who(st: &State, conv: &sigil_client::state::Conversation, identity: &[u8; 32]) -> String {
    if *identity == st.identity().public() {
        return "me".into();
    }
    conv.peers
        .first()
        .cloned()
        .unwrap_or_else(|| hex::encode(&identity[..4]))
}

async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Init { username, envoy } => {
            let st = State::create(&cli.state, &username, &envoy)?;
            let fp = sigil_protocol::identity::fingerprint(&st.identity().public());
            println!(
                "{} fingerprint {}",
                st.username,
                sigil_protocol::identity::fingerprint_display(&fp)
            );
        }
        Cmd::Register { invite } => {
            let mut st = State::load(&cli.state)?;
            let provider = SigilProvider::open(&st.mls_path())?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            account::register(&link, &mut st, &invite).await?;
            account::publish_key_packages(&link, &mut st, &provider, 10).await?;
            println!(
                "registered {}: credential, {} tokens, 10 key packages",
                st.username,
                st.tokens.len()
            );
        }
        Cmd::Tokens { n } => {
            let mut st = State::load(&cli.state)?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            account::draw_tokens(&link, &mut st, n).await?;
            println!("{} tokens", st.tokens.len());
        }
        Cmd::Publish { n } => {
            let mut st = State::load(&cli.state)?;
            let provider = SigilProvider::open(&st.mls_path())?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            account::publish_key_packages(&link, &mut st, &provider, n).await?;
            println!("published {n}");
        }
        Cmd::Lookup { username } => {
            let st = State::load(&cli.state)?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            let c = account::lookup(&link, &username).await?;
            let fp = sigil_protocol::identity::fingerprint(&c.identity_pub);
            println!(
                "{} on {} fingerprint {}",
                c.username,
                c.slot_server,
                sigil_protocol::identity::fingerprint_display(&fp)
            );
        }
        Cmd::Dm { username, text } => {
            let mut st = State::load(&cli.state)?;
            let provider = SigilProvider::open(&st.mls_path())?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            let conv = conversation::start_dm(&link, &mut st, &provider, &username, &text).await?;
            println!(
                "request sent to {username}; conversation {} is #{}",
                &conv.group_id[..8],
                st.conversations.len() - 1
            );
        }
        Cmd::Requests { accept, count } => {
            let mut st = State::load(&cli.state)?;
            let provider = SigilProvider::open(&st.mls_path())?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            let id = st.identity();
            let period = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs()
                / 2_592_000) as u32;
            let addresses = [
                names_addr(&id.public(), period),
                names_addr(&id.public(), period + 1),
            ];
            let handles = account::subscribe_requests(&link, &mut st).await?;
            // backfill both periods
            let mut got = 0;
            for (i, addr) in addresses.iter().enumerate() {
                let _ = (i, addr);
            }
            let mut rx = link.deliveries.lock().await;
            while got < count {
                let Some(Frame::Deliver {
                    wake_handle,
                    queue_seq,
                    envelope,
                    ..
                }) = rx.recv().await
                else {
                    break;
                };
                link.tx
                    .send(Frame::Ack {
                        wake_handle,
                        queue_seq,
                    })
                    .await?;
                let addr = if wake_handle == handles[0] {
                    addresses[0]
                } else {
                    addresses[1]
                };
                match conversation::open_request(&st, &addr, &envelope) {
                    Ok(req) => {
                        println!("request from {}: {}", req.from, req.first_message);
                        if accept {
                            let conv = conversation::accept(&mut st, &provider, &req)?;
                            println!(
                                "accepted; conversation {} is #{}",
                                &conv.group_id[..8],
                                st.conversations.len() - 1
                            );
                        } else {
                            st.requests.push(req);
                            st.save()?;
                        }
                        got += 1;
                    }
                    Err(e) => eprintln!("(unreadable request: {e})"),
                }
            }
        }
        Cmd::List => {
            let st = State::load(&cli.state)?;
            for (i, c) in st.conversations.iter().enumerate() {
                println!(
                    "#{i} {} with {} on {}",
                    &c.group_id[..8],
                    c.peers.join(", "),
                    c.slot_server
                );
            }
            for r in &st.requests {
                println!("pending request from {}: {}", r.from, r.first_message);
            }
        }
        Cmd::Send { n, text } => {
            let mut st = State::load(&cli.state)?;
            let provider = SigilProvider::open(&st.mls_path())?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            let conv = st
                .conversations
                .get(n)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no conversation #{n}"))?;
            let seq = conversation::send_text(&link, &mut st, &provider, &conv, &text).await?;
            println!("sent as seq {seq}");
        }
        Cmd::Listen { n, count } => {
            let mut st = State::load(&cli.state)?;
            let provider = SigilProvider::open(&st.mls_path())?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            let conv = st
                .conversations
                .get(n)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no conversation #{n}"))?;
            let (mut handle, ep) =
                conversation::subscribe(&link, &mut st, &provider, &conv).await?;
            eprintln!(
                "listening on {} (slot {}…)",
                conv.peers.join(", "),
                hex::encode(&ep.address[..8])
            );
            let mut seen = std::collections::HashSet::new();
            let mut got = 0;
            let show = |st: &State, seq: u64, env: &[u8]| -> bool {
                if let Some(text) = conversation::own_sent(st, &conv, &ep.address, seq) {
                    println!("{seq} me: {text}");
                    return true;
                }
                match conversation::receive(&provider, &conv, env) {
                    Ok(conversation::Incoming::Text {
                        from_identity,
                        ts_ms,
                        text,
                    }) => {
                        println!("{seq} [{ts_ms}] {}: {text}", who(st, &conv, &from_identity));
                        true
                    }
                    Ok(conversation::Incoming::Rotated) => {
                        println!("{seq} (epoch changed)");
                        false
                    }
                    Ok(conversation::Incoming::Other { kind }) => {
                        println!("{seq} (event kind {kind})");
                        false
                    }
                    Err(e) => {
                        println!("{seq} (cannot process: {e})");
                        false
                    }
                }
            };
            for (seq, env) in conversation::backfill(&link, &provider, &conv, 0).await? {
                if seen.insert(seq) && show(&st, seq, &env) {
                    got += 1;
                }
            }
            let mut rx = link.deliveries.lock().await;
            while count == 0 || got < count {
                let Some(f) = rx.recv().await else { break };
                if let Frame::Deliver {
                    wake_handle,
                    queue_seq,
                    slot_seq,
                    envelope,
                } = f
                {
                    link.tx
                        .send(Frame::Ack {
                            wake_handle,
                            queue_seq,
                        })
                        .await?;
                    if wake_handle != handle {
                        continue;
                    }
                    if seen.insert(slot_seq) && show(&st, slot_seq, &envelope) {
                        got += 1;
                    }
                    let _ = &mut handle;
                }
            }
        }
    }
    Ok(())
}

fn names_addr(identity_pub: &[u8; 32], period: u32) -> [u8; 32] {
    sigil_protocol::names::requests_address(identity_pub, period)
}
