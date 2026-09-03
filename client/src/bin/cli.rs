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
        /// Sets up backup and recovery.
        #[arg(long)]
        password: Option<String>,
    },
    /// Upload an encrypted backup of this device's state.
    Backup,
    /// Print the recovery code (keep it on paper).
    Code,
    /// Change the backup password.
    SetPassword { password: String },
    /// Restore on this device from username and password, with the printed
    /// recovery code or, where the server keeps an escrow, the sign-in
    /// token as --gate instead.
    Recover {
        #[arg(long)]
        username: String,
        #[arg(long)]
        password: String,
        #[arg(long, default_value = "")]
        code: String,
        #[arg(long, default_value = "")]
        gate: String,
        #[arg(long)]
        envoy: String,
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
    /// Create a group with a name and members.
    Group { name: String, users: Vec<String> },
    /// Invite a username into conversation `n`.
    Invite { n: usize, username: String },
    /// Rename conversation `n`.
    Rename { n: usize, name: String },
    /// Leave conversation `n`.
    Leave { n: usize },
    /// Send a raw event of `kind` into conversation `n`, with an optional
    /// reference (an event id) and a body: how a script votes on a poll,
    /// shares a place, or answers in a thread.
    Event {
        n: usize,
        kind: u16,
        body: String,
        #[arg(long, default_value = "")]
        reference: String,
    },
    /// Send a file into conversation `n`.
    Sendfile {
        n: usize,
        path: PathBuf,
        #[arg(long, default_value = "")]
        caption: String,
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
    let h = hex::encode(identity);
    conv.members
        .iter()
        .find(|m| m.identity == h)
        .map(|m| m.username.clone())
        .or_else(|| conv.peers.first().cloned())
        .unwrap_or_else(|| h[..8].to_string())
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
        Cmd::Register { invite, password } => {
            let mut st = State::load(&cli.state)?;
            let provider = SigilProvider::open(&st.mls_path())?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            account::register(&link, &mut st, &invite).await?;
            account::publish_key_packages(&link, &mut st, &provider, 10).await?;
            if let Some(pw) = password {
                sigil_client::backup::enable(&link, &mut st, &pw).await?;
                if sigil_client::backup::escrow_put(&link, &st, &pw).await? {
                    println!("recovery escrowed: the password and the sign-in bring this account back");
                } else {
                    println!(
                        "recovery code: {}",
                        sigil_client::backup::code(&st).unwrap()
                    );
                }
            }
            println!(
                "registered {}: credential, {} tokens, 10 key packages",
                st.username,
                st.tokens.len()
            );
        }
        Cmd::Backup => {
            let mut st = State::load(&cli.state)?;
            let provider = SigilProvider::open(&st.mls_path())?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            let n = sigil_client::backup::upload(&link, &mut st, &provider, &[]).await?;
            println!("backed up in {n} chunk(s)");
        }
        Cmd::Code => {
            let st = State::load(&cli.state)?;
            println!(
                "{}",
                sigil_client::backup::code(&st)
                    .ok_or_else(|| anyhow::anyhow!("no password set; register with --password"))?
            );
        }
        Cmd::SetPassword { password } => {
            let st = State::load(&cli.state)?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            sigil_client::backup::set_password(&link, &st, &password).await?;
            println!("password changed");
        }
        Cmd::Recover {
            username,
            password,
            code,
            gate,
            envoy,
        } => {
            let username = username.to_lowercase();
            let key = if !code.is_empty() {
                sigil_protocol::recovery::parse_recovery_code(&code)
                    .map_err(|_| anyhow::anyhow!("that is not a valid recovery code"))?
            } else {
                let device_id = hex::encode(rand::random::<[u8; 32]>());
                let link = Link::connect(&envoy, &device_id).await?;
                sigil_client::backup::escrow_get(&link, &username, &password, gate.as_bytes())
                    .await?
            };
            let (st, _extra) = sigil_client::backup::restore(
                &cli.state,
                &envoy,
                &username.to_lowercase(),
                &password,
                &key,
            )
            .await?;
            println!(
                "restored {} with {} conversations and {} tokens",
                st.username,
                st.conversations.len(),
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
                let title = if c.name.is_empty() {
                    format!("with {}", c.peers.join(", "))
                } else {
                    format!("\"{}\" ({} members)", c.name, c.members.len())
                };
                println!("#{i} {} {title} on {}", &c.group_id[..8], c.slot_server);
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
        Cmd::Group { name, users } => {
            let mut st = State::load(&cli.state)?;
            let provider = SigilProvider::open(&st.mls_path())?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            let conv =
                sigil_client::group::create(&link, &mut st, &provider, &name, &users, "").await?;
            println!(
                "created {name} ({}) as #{}",
                &conv.group_id[..8],
                st.conversations.len() - 1
            );
        }
        Cmd::Invite { n, username } => {
            let mut st = State::load(&cli.state)?;
            let provider = SigilProvider::open(&st.mls_path())?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            let conv = st
                .conversations
                .get(n)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no conversation #{n}"))?;
            sigil_client::group::invite(&link, &mut st, &provider, &conv, &username).await?;
            println!("invited {username}");
        }
        Cmd::Rename { n, name } => {
            let mut st = State::load(&cli.state)?;
            let provider = SigilProvider::open(&st.mls_path())?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            let conv = st
                .conversations
                .get(n)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no conversation #{n}"))?;
            sigil_client::group::rename(&link, &mut st, &provider, &conv, &name).await?;
            println!("renamed");
        }
        Cmd::Leave { n } => {
            let mut st = State::load(&cli.state)?;
            let provider = SigilProvider::open(&st.mls_path())?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            let conv = st
                .conversations
                .get(n)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no conversation #{n}"))?;
            sigil_client::group::leave(&link, &mut st, &provider, &conv).await?;
            println!("left");
        }
        Cmd::Event {
            n,
            kind,
            body,
            reference,
        } => {
            let mut st = State::load(&cli.state)?;
            let provider = SigilProvider::open(&st.mls_path())?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            let conv = st
                .conversations
                .get(n)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no conversation #{n}"))?;
            let kind = sigil_protocol::envelope::Kind::try_from(kind)
                .map_err(|_| anyhow::anyhow!("unknown event kind {kind}"))?;
            let sent = conversation::send_event(
                &link,
                &mut st,
                &provider,
                &conv,
                kind,
                reference.as_bytes(),
                body.as_bytes(),
            )
            .await?;
            let conv = st
                .conversations
                .iter()
                .find(|c| c.group_id == conv.group_id)
                .cloned()
                .unwrap_or(conv);
            print_caught(&st, &conv, &sent.caught_up);
            st.save()?;
            println!("sent {}:{}", hex::encode(sent.address), sent.seq);
        }
        Cmd::Sendfile { n, path, caption } => {
            let mut st = State::load(&cli.state)?;
            let provider = SigilProvider::open(&st.mls_path())?;
            let link = Link::connect(&st.envoy, &st.device_id).await?;
            let conv = st
                .conversations
                .get(n)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no conversation #{n}"))?;
            let m = sigil_client::media::upload(&link, &mut st, &path, &caption).await?;
            let sent = conversation::send_event(
                &link,
                &mut st,
                &provider,
                &conv,
                sigil_protocol::envelope::Kind::Media,
                &[],
                &serde_json::to_vec(&m)?,
            )
            .await?;
            println!(
                "sent {} ({} bytes, {} chunks) as seq {}",
                m.filename,
                m.size,
                m.chunks.len(),
                sent.seq
            );
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
                // policies and leaves first, so names resolve when printing
                for c in &caught {
                    if let conversation::Incoming::Event {
                        kind,
                        body,
                        from_identity,
                        ..
                    } = &c.incoming
                    {
                        if *kind == sigil_protocol::envelope::Kind::Policy as u16
                            || *kind == sigil_protocol::envelope::Kind::Membership as u16
                        {
                            let _ = sigil_client::group::apply_control(
                                &link,
                                &mut st,
                                &provider,
                                &conv,
                                *kind,
                                from_identity,
                                body,
                            )
                            .await;
                        }
                    }
                }
                let conv = st
                    .conversations
                    .iter()
                    .find(|c| c.group_id == conv.group_id)
                    .cloned()
                    .unwrap_or(conv.clone());
                got += print_caught(&st, &conv, &caught);
                for c in &caught {
                    if let conversation::Incoming::Event { kind, body, .. } = &c.incoming {
                        if *kind == sigil_protocol::envelope::Kind::Media as u16 {
                            if let Ok(m) =
                                serde_json::from_str::<sigil_client::media::Manifest>(body)
                            {
                                let dest = std::path::PathBuf::from("downloads").join(&m.filename);
                                match sigil_client::media::download(
                                    &link,
                                    &conv.slot_server,
                                    &m,
                                    &dest,
                                )
                                .await
                                {
                                    Ok(()) => println!("{} saved {}", c.seq, dest.display()),
                                    Err(e) => println!("{} download failed: {e}", c.seq),
                                }
                            }
                        }
                    }
                }
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
                // anything written between the catch-up and the subscription
                let late = conversation::catch_up(&link, &mut st, &provider, &conv).await?;
                if !late.is_empty() {
                    // control events count here too, or a rename in this
                    // window is printed and forgotten
                    for c in &late {
                        if let conversation::Incoming::Event {
                            kind,
                            body,
                            from_identity,
                            ..
                        } = &c.incoming
                        {
                            if *kind == sigil_protocol::envelope::Kind::Policy as u16
                                || *kind == sigil_protocol::envelope::Kind::Membership as u16
                            {
                                let _ = sigil_client::group::apply_control(
                                    &link,
                                    &mut st,
                                    &provider,
                                    &conv,
                                    *kind,
                                    from_identity,
                                    body,
                                )
                                .await;
                            }
                        }
                    }
                    got += print_caught(&st, &conv, &late);
                    if late
                        .iter()
                        .any(|c| matches!(c.incoming, conversation::Incoming::Rotated))
                    {
                        continue 'epoch;
                    }
                    if count > 0 && got >= count {
                        break;
                    }
                }
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
                        Ok(conversation::Incoming::Event {
                            kind,
                            body,
                            from_identity,
                            ..
                        }) => {
                            describe_event(&st, &conv, slot_seq, kind, &body, &from_identity);
                            if kind == sigil_protocol::envelope::Kind::Media as u16 {
                                if let Ok(m) =
                                    serde_json::from_str::<sigil_client::media::Manifest>(&body)
                                {
                                    let dest =
                                        std::path::PathBuf::from("downloads").join(&m.filename);
                                    match sigil_client::media::download(
                                        &link,
                                        &conv.slot_server,
                                        &m,
                                        &dest,
                                    )
                                    .await
                                    {
                                        Ok(()) => println!("{slot_seq} saved {}", dest.display()),
                                        Err(e) => println!("{slot_seq} download failed: {e}"),
                                    }
                                }
                                got += 1;
                            } else if kind == sigil_protocol::envelope::Kind::Policy as u16
                                || kind == sigil_protocol::envelope::Kind::Membership as u16
                            {
                                let _ = sigil_client::group::apply_control(
                                    &link,
                                    &mut st,
                                    &provider,
                                    &conv,
                                    kind,
                                    &from_identity,
                                    &body,
                                )
                                .await;
                                // a leave we committed rotated the address
                                drop(rx);
                                continue 'epoch;
                            }
                        }
                        Ok(conversation::Incoming::Other { kind }) => {
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
            conversation::Incoming::Event {
                kind,
                body,
                from_identity,
                ..
            } => {
                describe_event(st, conv, c.seq, *kind, body, from_identity);
                if *kind == sigil_protocol::envelope::Kind::Media as u16 {
                    n += 1;
                }
            }
            conversation::Incoming::Other { kind } => println!("{} (event kind {kind})", c.seq),
        }
    }
    n
}

fn describe_event(
    st: &State,
    conv: &sigil_client::state::Conversation,
    seq: u64,
    kind: u16,
    body: &str,
    from_identity: &[u8; 32],
) {
    use sigil_protocol::envelope::Kind;
    if kind == Kind::Media as u16 {
        match serde_json::from_str::<sigil_client::media::Manifest>(body) {
            Ok(m) => println!(
                "{seq} {}: [file] {} ({} bytes, {})",
                who(st, conv, from_identity),
                m.filename,
                m.size,
                m.mime
            ),
            Err(_) => println!("{seq} (bad media manifest)"),
        }
    } else if kind == Kind::Policy as u16 {
        println!("{seq} (policy updated by {})", who(st, conv, from_identity));
    } else if kind == Kind::Membership as u16 {
        println!("{seq} (membership: {body})");
    } else {
        println!("{seq} (event kind {kind})");
    }
}

fn names_addr(identity_pub: &[u8; 32], period: u32) -> [u8; 32] {
    sigil_protocol::names::requests_address(identity_pub, period)
}
