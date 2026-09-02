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
    /// New device: show a link offer and wait for an existing device to scan it.
    LinkOffer {
        #[arg(long)]
        username: String,
        #[arg(long)]
        envoy: String,
        #[arg(long)]
        offer_file: Option<PathBuf>,
    },
    /// Existing device: scan an offer (the text, or @file), confirm the emoji, transfer.
    LinkScan {
        offer: String,
        #[arg(long)]
        yes: bool,
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
            let sent = conversation::send_event(
                &link,
                &mut st,
                &provider,
                &conv,
                sigil_protocol::envelope::Kind::Text,
                &[],
                text.as_bytes(),
            )
            .await?;
            print_caught(&st, &conv, &sent.caught_up);
            println!("sent as seq {}", sent.seq);
        }
        Cmd::LinkOffer {
            username,
            envoy,
            offer_file,
        } => link_offer(cli.state.clone(), username, envoy, offer_file).await?,
        Cmd::LinkScan { offer, yes } => link_scan(cli.state.clone(), offer, yes).await?,
        Cmd::Listen { n, count } => {
            let mut st = State::load(&cli.state)?;
            let provider = SigilProvider::open(&st.mls_path())?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            let conv = st
                .conversations
                .get(n)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no conversation #{n}"))?;
            // Catch up (which follows rotations), then subscribe to the
            // current address and deliver live; a commit sends us round again.
            let mut got = 0;
            'epoch: loop {
                let caught = conversation::catch_up(&link, &mut st, &provider, &conv).await?;
                got += print_caught(&st, &conv, &caught);
                if count > 0 && got >= count {
                    break;
                }
                let (handle, ep) =
                    conversation::subscribe(&link, &mut st, &provider, &conv).await?;
                eprintln!(
                    "listening on {} (slot {}…)",
                    conv.peers.join(", "),
                    hex::encode(&ep.address[..8])
                );
                let mut rx = link.deliveries.lock().await;
                while count == 0 || got < count {
                    let Some(f) = rx.recv().await else {
                        break 'epoch;
                    };
                    let Frame::Deliver {
                        wake_handle,
                        queue_seq,
                        slot_seq,
                        envelope,
                    } = f
                    else {
                        continue;
                    };
                    link.tx
                        .send(Frame::Ack {
                            wake_handle,
                            queue_seq,
                        })
                        .await?;
                    if wake_handle != handle
                        || slot_seq <= conversation::cursor(&st, &conv, &ep.address)
                    {
                        continue;
                    }
                    conversation::set_cursor(&mut st, &conv, &ep.address, slot_seq);
                    st.save()?;
                    if let Some((_, text)) =
                        conversation::own_sent(&st, &conv, &ep.address, slot_seq)
                    {
                        if !text.is_empty() {
                            println!("{slot_seq} me: {text}");
                            got += 1;
                        }
                        continue;
                    }
                    match conversation::receive(&provider, &conv, &envelope) {
                        Ok(conversation::Incoming::Text {
                            from_identity,
                            ts_ms,
                            text,
                            ..
                        }) => {
                            println!(
                                "{slot_seq} [{ts_ms}] {}: {text}",
                                who(&st, &conv, &from_identity)
                            );
                            got += 1;
                        }
                        Ok(conversation::Incoming::Rotated) => {
                            println!("{slot_seq} (epoch changed)");
                            drop(rx);
                            continue 'epoch;
                        }
                        Ok(conversation::Incoming::Event { kind, .. })
                        | Ok(conversation::Incoming::Other { kind }) => {
                            println!("{slot_seq} (event kind {kind})")
                        }
                        Err(e) => println!("{slot_seq} (cannot process: {e})"),
                    }
                }
                break;
            }
        }
    }
    Ok(())
}

async fn link_offer(
    state: PathBuf,
    username: String,
    envoy: String,
    offer_file: Option<PathBuf>,
) -> anyhow::Result<()> {
    let (_, server) = sigil_protocol::names::parse_username(&username)
        .map_err(|_| anyhow::anyhow!("bad username"))?;
    let offer = sigil_client::linking::Offer::new();
    println!("{}", offer.text());
    if let Some(f) = &offer_file {
        std::fs::write(f, offer.text())?;
    }
    eprintln!("scan this on a device that is signed in as {username}");
    let (st, _extra) =
        sigil_client::linking::wait_for_link(&state, server, &envoy, &offer, |p| match p {
            sigil_client::linking::Progress::Sas(s) => {
                eprintln!("emoji: {s}   (confirm on the other device)")
            }
            sigil_client::linking::Progress::Welcomed(w) => {
                eprintln!("joined conversation with {w}")
            }
            sigil_client::linking::Progress::Done => eprintln!("linked"),
        })
        .await?;
    println!(
        "linked as {} with {} conversations and {} tokens",
        st.username,
        st.conversations.len(),
        st.tokens.len()
    );
    Ok(())
}

async fn link_scan(state: PathBuf, offer: String, yes: bool) -> anyhow::Result<()> {
    let offer = if let Some(f) = offer.strip_prefix('@') {
        std::fs::read_to_string(f)?
    } else {
        offer
    };
    let mut st = State::load(&state)?;
    let provider = SigilProvider::open(&st.mls_path())?;
    let link = Link::connect(&st.envoy, &st.device_id).await?;
    let scanned = sigil_client::linking::scan(&link, &mut st, &offer).await?;
    println!("emoji: {}", scanned.sas);
    if !yes {
        eprint!("do they match what the new device shows? [y/N] ");
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if !line.trim().eq_ignore_ascii_case("y") {
            anyhow::bail!("not confirmed; nothing was sent");
        }
    }
    sigil_client::linking::transfer(&link, &mut st, &provider, scanned, vec![], |p| match p {
        sigil_client::linking::Progress::Welcomed(w) => {
            eprintln!("added the new device to the conversation with {w}")
        }
        sigil_client::linking::Progress::Done => eprintln!("done"),
        _ => {}
    })
    .await?;
    println!("linked; {} tokens left here", st.tokens.len());
    Ok(())
}

/// Print events processed while catching up. Returns how many were messages.
fn print_caught(
    st: &State,
    conv: &sigil_client::state::Conversation,
    caught: &[conversation::Caught],
) -> usize {
    let mut n = 0;
    for c in caught {
        match &c.incoming {
            conversation::Incoming::Text {
                from_identity,
                ts_ms,
                text,
                ..
            } => {
                println!(
                    "{} [{ts_ms}] {}: {text}",
                    c.seq,
                    who(st, conv, from_identity)
                );
                n += 1;
            }
            conversation::Incoming::Rotated => println!("{} (epoch changed)", c.seq),
            conversation::Incoming::Event { kind, .. } | conversation::Incoming::Other { kind } => {
                println!("{} (event kind {kind})", c.seq)
            }
        }
    }
    n
}

fn names_addr(identity_pub: &[u8; 32], period: u32) -> [u8; 32] {
    sigil_protocol::names::requests_address(identity_pub, period)
}
