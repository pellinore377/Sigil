//! Conversations are MLS groups. This module creates them, sends and
//! accepts Welcomes through the requests slot, derives the slot from the
//! epoch secret, and moves events in and out.

use crate::account::{check_credential, mls_credential, CIPHERSUITE};
use crate::provider::SigilProvider;
use crate::state::Conversation;
use crate::{Link, State};
use openmls::prelude::*;
use sigil_protocol::encoding::Reader;
use sigil_protocol::epoch::{self, EpochMaterial};
use sigil_protocol::identity::ContactCard;
use sigil_protocol::wire::{Frame, Request, Response};
use sigil_protocol::{envelope, requests};
use tls_codec::Deserialize as _;

/// Keep secrets for a few past epochs, so a message encrypted just before
/// a rotation can still be read after it.
pub const PAST_EPOCHS: usize = 3;

pub fn create_config() -> MlsGroupCreateConfig {
    MlsGroupCreateConfig::builder()
        .ciphersuite(CIPHERSUITE)
        .use_ratchet_tree_extension(true)
        .max_past_epochs(PAST_EPOCHS)
        .build()
}
pub fn join_config() -> MlsGroupJoinConfig {
    MlsGroupJoinConfig::builder()
        .use_ratchet_tree_extension(true)
        .max_past_epochs(PAST_EPOCHS)
        .build()
}

/// Remember the current epoch's slot material on the conversation.
pub fn record_epoch(st: &mut State, conv: &Conversation, ep: &EpochMaterial) {
    if let Some(c) = st
        .conversations
        .iter_mut()
        .find(|c| c.group_id == conv.group_id)
    {
        let address = hex::encode(ep.address);
        if c.epochs
            .last()
            .map(|e| e.address == address)
            .unwrap_or(false)
        {
            return;
        }
        c.epochs.retain(|e| e.address != address);
        c.epochs.push(crate::state::EpochRecord {
            address,
            envelope_key: hex::encode(ep.envelope_key),
            read_cap: hex::encode(ep.read_cap),
            write_pub: hex::encode(ep.write_pub),
        });
        while c.epochs.len() > PAST_EPOCHS + 1 {
            c.epochs.remove(0);
        }
    }
}

/// The envelope key for `address`, current or recent.
pub fn envelope_key_for(st: &State, conv: &Conversation, address: &[u8; 32]) -> Option<[u8; 32]> {
    let a = hex::encode(address);
    st.conversations
        .iter()
        .find(|c| c.group_id == conv.group_id)?
        .epochs
        .iter()
        .find(|e| e.address == a)
        .and_then(|e| hex::decode(&e.envelope_key).ok())
        .and_then(|v| v.try_into().ok())
}

pub fn epoch_material(group: &MlsGroup, provider: &SigilProvider) -> anyhow::Result<EpochMaterial> {
    let secret = group
        .export_secret(provider, "sigil v1 epoch", b"", 32)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    Ok(epoch::derive(&secret.try_into().unwrap()))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Start a direct message with `username`: a group of two with no name.
pub async fn start_dm(
    link: &Link,
    st: &mut State,
    provider: &SigilProvider,
    username: &str,
    first_text: &str,
) -> anyhow::Result<Conversation> {
    crate::group::create(link, st, provider, "", &[username.to_string()], first_text).await
}

/// Decode a delivered requests-slot envelope into a pending request.
pub fn open_request(
    st: &State,
    address: &[u8; 32],
    sealed: &[u8],
) -> anyhow::Result<crate::state::PendingRequest> {
    let id = st.identity();
    let plain = requests::open(&id.kem, address, sealed).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let ev = envelope::Event::decode(&plain).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    if ev.kind != envelope::Kind::Welcome as u16 {
        anyhow::bail!("not a welcome");
    }
    let mut r = Reader::new(&ev.body);
    let welcome = r.bytes().map_err(|e| anyhow::anyhow!("{e:?}"))?.to_vec();
    let card_bytes = r.bytes().map_err(|e| anyhow::anyhow!("{e:?}"))?.to_vec();
    let first = r.str().map_err(|e| anyhow::anyhow!("{e:?}"))?.to_string();
    let policy = r.bytes().map(hex::encode).unwrap_or_default();
    let card = ContactCard::verify(&card_bytes).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    Ok(crate::state::PendingRequest {
        from: card.username,
        from_card: hex::encode(card_bytes),
        welcome: hex::encode(welcome),
        first_message: first,
        policy,
    })
}

/// Accept a pending request: join the group from its Welcome.
pub fn accept(
    st: &mut State,
    provider: &SigilProvider,
    req: &crate::state::PendingRequest,
) -> anyhow::Result<Conversation> {
    let card =
        ContactCard::verify(&hex::decode(&req.from_card)?).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let msg = MlsMessageIn::tls_deserialize_exact(&hex::decode(&req.welcome)?)?;
    let welcome = match msg.extract() {
        MlsMessageBodyIn::Welcome(w) => w,
        _ => anyhow::bail!("not a welcome message"),
    };
    let staged = StagedWelcome::new_from_welcome(provider, &join_config(), welcome, None)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    // The sender of the Welcome must be the identity on the card.
    let sender = staged
        .welcome_sender()
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    check_credential(sender.credential(), &card.identity_pub)?;
    let group = staged
        .into_group(provider)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    provider.save()?;
    let mut conv = Conversation {
        group_id: hex::encode(group.group_id().as_slice()),
        peers: vec![req.from.clone()],
        slot_server: card.slot_server.clone(),
        ..Default::default()
    };
    // A direct message from before policies, or a group: the policy names
    // everyone. Without one, it is the two of us.
    let me = st.username.clone();
    match hex::decode(&req.policy)
        .ok()
        .and_then(|b| serde_json::from_slice::<crate::group::Policy>(&b).ok())
    {
        Some(p) => p.apply(&mut conv, &me),
        None => {
            conv.members = vec![
                crate::state::Member {
                    username: card.username.clone(),
                    identity: hex::encode(card.identity_pub),
                },
                crate::state::Member {
                    username: me.clone(),
                    identity: hex::encode(st.identity().public()),
                },
            ];
        }
    }
    st.conversations.push(conv.clone());
    st.requests.retain(|r| r.welcome != req.welcome);
    st.save()?;
    Ok(conv)
}

pub fn load_group(provider: &SigilProvider, conv: &Conversation) -> anyhow::Result<MlsGroup> {
    let gid = GroupId::from_slice(&hex::decode(&conv.group_id)?);
    MlsGroup::load(provider.storage(), &gid)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?
        .ok_or_else(|| anyhow::anyhow!("group not in store"))
}

pub struct Sent {
    pub seq: u64,
    pub address: [u8; 32],
    /// Events processed while catching up before the send.
    pub caught_up: Vec<Caught>,
}

/// Catch up, then send any event into the conversation's current slot.
pub async fn send_event(
    link: &Link,
    st: &mut State,
    provider: &SigilProvider,
    conv: &Conversation,
    kind: envelope::Kind,
    reference: &[u8],
    body: &[u8],
) -> anyhow::Result<Sent> {
    let caught_up = catch_up(link, st, provider, conv).await?;
    let mut group = load_group(provider, conv)?;
    let (_, signer) = mls_credential(st);
    let ev = envelope::Event {
        kind: kind as u16,
        ts_ms: now_ms(),
        reference: reference.to_vec(),
        body: body.to_vec(),
    };
    let mls_out = group
        .create_message(provider, &signer, &ev.encode())
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    provider.save()?;
    let ep = epoch_material(&group, provider)?;
    record_epoch(st, conv, &ep);
    let sealed = envelope::seal(
        &ep.envelope_key,
        &ep.address,
        &rand::random(),
        &mls_out.to_bytes()?,
    )
    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let sig = epoch::sign_put(&ep.write_key, &ep.address, &sealed);
    let token = st.take_token()?;
    let resp = link
        .call(
            &conv.slot_server,
            &Request::SlotPut {
                address: ep.address,
                write_pub: ep.write_pub,
                sig,
                envelope: sealed,
                token,
            },
            None,
        )
        .await?;
    let Response::SlotPut { seq } = resp else {
        anyhow::bail!("unexpected")
    };
    if let Some(c) = st
        .conversations
        .iter_mut()
        .find(|c| c.group_id == conv.group_id)
    {
        // Text is kept for readback; other kinds only need the seq to be known as ours.
        let text = if kind == envelope::Kind::Text {
            String::from_utf8_lossy(body).to_string()
        } else {
            String::new()
        };
        c.sent.insert(
            format!("{}:{seq}", hex::encode(ep.address)),
            format!("{}\u{1f}{text}", ev.ts_ms),
        );
    }
    // The cursor stays where the catch-up left it: anything the other side
    // wrote between that read and this put sits below our seq, and moving
    // the cursor past it would drop it. The next catch-up recognises our
    // own envelope from the sent record and moves on.
    st.save()?;
    Ok(Sent {
        seq,
        address: ep.address,
        caught_up,
    })
}

/// Send a text event into the conversation's current slot.
pub async fn send_text(
    link: &Link,
    st: &mut State,
    provider: &SigilProvider,
    conv: &Conversation,
    text: &str,
) -> anyhow::Result<u64> {
    Ok(send_event(
        link,
        st,
        provider,
        conv,
        envelope::Kind::Text,
        &[],
        text.as_bytes(),
    )
    .await?
    .seq)
}

/// What this device sent at `seq` in `address`, if anything: the
/// timestamp and the text (empty for non-text events).
pub fn own_sent(
    st: &State,
    conv: &Conversation,
    address: &[u8; 32],
    seq: u64,
) -> Option<(u64, String)> {
    let v = st
        .conversations
        .iter()
        .find(|c| c.group_id == conv.group_id)?
        .sent
        .get(&format!("{}:{seq}", hex::encode(address)))?;
    match v.split_once('\u{1f}') {
        Some((ts, text)) => Some((ts.parse().unwrap_or(0), text.to_string())),
        None => Some((0, v.clone())),
    }
}

/// One event processed while catching up: which address and sequence it
/// came from, and what it was.
pub struct Caught {
    pub address: [u8; 32],
    pub seq: u64,
    pub incoming: Incoming,
}

/// Process everything written to the conversation's slots since our
/// cursors, following address rotations, so that we are on the latest
/// epoch. Must run before every send: a message encrypted under a stale
/// epoch lands in a slot the others have already left.
pub async fn catch_up(
    link: &Link,
    st: &mut State,
    provider: &SigilProvider,
    conv: &Conversation,
) -> anyhow::Result<Vec<Caught>> {
    let mut out = Vec::new();
    let me = st.identity().public();
    'epoch: for _ in 0..64 {
        let address = {
            let group = load_group(provider, conv)?;
            let ep = epoch_material(&group, provider)?;
            record_epoch(st, conv, &ep);
            ep.address
        };
        // Read the whole current slot: our own envelopes come back from the
        // sent record whatever the cursor says, everything else is
        // processed once, past the cursor.
        let mut after = 0;
        let mut rotated = false;
        loop {
            let items = backfill(link, provider, conv, after).await?;
            let n = items.len();
            for (seq, env) in items {
                after = seq;
                if let Some((ts_ms, text)) = own_sent(st, conv, &address, seq) {
                    set_cursor(st, conv, &address, seq);
                    if !text.is_empty() {
                        out.push(Caught {
                            address,
                            seq,
                            incoming: Incoming::Text {
                                from_identity: me,
                                ts_ms,
                                text,
                                reference: String::new(),
                            },
                        });
                    }
                    continue;
                }
                if seq <= cursor(st, conv, &address) {
                    continue;
                }
                set_cursor(st, conv, &address, seq);
                match receive(provider, conv, &env) {
                    Ok(Incoming::Rotated) => {
                        out.push(Caught {
                            address,
                            seq,
                            incoming: Incoming::Rotated,
                        });
                        rotated = true;
                        break;
                    }
                    Ok(inc) => out.push(Caught {
                        address,
                        seq,
                        incoming: inc,
                    }),
                    // not for us, or already consumed; say why when asked
                    Err(e) => {
                        if std::env::var_os("SIGIL_DEBUG_SKIPS").is_some() {
                            eprintln!("skipped {}:{seq}: {e:#}", hex::encode(&address[..4]));
                        }
                    }
                }
            }
            if rotated {
                continue 'epoch;
            }
            if n < 64 {
                break 'epoch;
            }
        }
    }
    st.save()?;
    Ok(out)
}

pub fn cursor(st: &State, conv: &Conversation, address: &[u8; 32]) -> u64 {
    st.conversations
        .iter()
        .find(|c| c.group_id == conv.group_id)
        .and_then(|c| c.cursors.get(&hex::encode(address)).copied())
        .unwrap_or(0)
}

pub fn set_cursor(st: &mut State, conv: &Conversation, address: &[u8; 32], seq: u64) {
    if let Some(c) = st
        .conversations
        .iter_mut()
        .find(|c| c.group_id == conv.group_id)
    {
        let e = c.cursors.entry(hex::encode(address)).or_insert(0);
        if seq > *e {
            *e = seq;
        }
    }
}

/// What came out of an envelope.
pub enum Incoming {
    Text {
        from_identity: [u8; 32],
        ts_ms: u64,
        text: String,
        reference: String,
    },
    /// A reaction, receipt, typing notice or other small event.
    Event {
        from_identity: [u8; 32],
        ts_ms: u64,
        kind: u16,
        reference: String,
        body: String,
    },
    Other {
        kind: u16,
    },
    /// A commit was applied; the address has rotated.
    Rotated,
}

/// Open and process one envelope from the conversation's slot.
pub fn receive(
    provider: &SigilProvider,
    conv: &Conversation,
    sealed: &[u8],
) -> anyhow::Result<Incoming> {
    let group = load_group(provider, conv)?;
    let ep = epoch_material(&group, provider)?;
    receive_with(provider, conv, &ep.envelope_key, &ep.address, sealed)
}

/// Open an envelope that arrived for `address`, which may be a recent
/// past epoch's, and process it.
pub fn receive_at(
    st: &State,
    provider: &SigilProvider,
    conv: &Conversation,
    address: &[u8; 32],
    sealed: &[u8],
) -> anyhow::Result<Incoming> {
    let key = envelope_key_for(st, conv, address)
        .ok_or_else(|| anyhow::anyhow!("no key for that address any more"))?;
    receive_with(provider, conv, &key, address, sealed)
}

fn receive_with(
    provider: &SigilProvider,
    conv: &Conversation,
    envelope_key: &[u8; 32],
    address: &[u8; 32],
    sealed: &[u8],
) -> anyhow::Result<Incoming> {
    let mut group = load_group(provider, conv)?;
    let mls_bytes =
        envelope::open(envelope_key, address, sealed).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let msg = MlsMessageIn::tls_deserialize_exact(&mls_bytes)?;
    let pm = msg
        .try_into_protocol_message()
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let processed = group
        .process_message(provider, pm)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let sender_identity: [u8; 32] = {
        let basic = BasicCredential::try_from(processed.credential().clone())
            .map_err(|_| anyhow::anyhow!("credential"))?;
        basic.identity()[..32].try_into().unwrap()
    };
    let out = match processed.into_content() {
        ProcessedMessageContent::ApplicationMessage(app) => {
            let ev =
                envelope::Event::decode(&app.into_bytes()).map_err(|e| anyhow::anyhow!("{e:?}"))?;
            let reference = String::from_utf8_lossy(&ev.reference).to_string();
            if ev.kind == envelope::Kind::Text as u16 {
                Incoming::Text {
                    from_identity: sender_identity,
                    ts_ms: ev.ts_ms,
                    text: String::from_utf8_lossy(&ev.body).to_string(),
                    reference,
                }
            } else {
                Incoming::Event {
                    from_identity: sender_identity,
                    ts_ms: ev.ts_ms,
                    kind: ev.kind,
                    reference,
                    body: String::from_utf8_lossy(&ev.body).to_string(),
                }
            }
        }
        ProcessedMessageContent::StagedCommitMessage(staged) => {
            group
                .merge_staged_commit(provider, *staged)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            Incoming::Rotated
        }
        ProcessedMessageContent::ProposalMessage(p) => {
            group
                .store_pending_proposal(provider.storage(), *p)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            Incoming::Other {
                kind: envelope::Kind::Proposal as u16,
            }
        }
        ProcessedMessageContent::ExternalJoinProposalMessage(_) => Incoming::Other { kind: 0 },
    };
    provider.save()?;
    Ok(out)
}

/// Subscribe to the conversation's current address; returns the handle.
pub async fn subscribe(
    link: &Link,
    st: &mut State,
    provider: &SigilProvider,
    conv: &Conversation,
) -> anyhow::Result<([u8; 32], EpochMaterial)> {
    let group = load_group(provider, conv)?;
    let ep = epoch_material(&group, provider)?;
    record_epoch(st, conv, &ep);
    st.save()?;
    let handle: [u8; 32] = rand::random();
    let token = st.take_token()?;
    link.call(
        &conv.slot_server,
        &Request::SlotSubscribe {
            address: ep.address,
            wake_handle: handle,
            proof: vec![],
            token,
        },
        Some(handle),
    )
    .await?;
    Ok((handle, ep))
}

/// Read everything in the current slot after `after_seq`.
pub async fn backfill(
    link: &Link,
    provider: &SigilProvider,
    conv: &Conversation,
    after_seq: u64,
) -> anyhow::Result<Vec<(u64, Vec<u8>)>> {
    let group = load_group(provider, conv)?;
    let ep = epoch_material(&group, provider)?;
    match link
        .call(
            &conv.slot_server,
            &Request::SlotGet {
                read_cap: ep.read_cap,
                write_pub: ep.write_pub,
                after_seq,
                limit: 64,
            },
            None,
        )
        .await
    {
        Ok(Response::SlotGet { items, .. }) => {
            Ok(items.into_iter().map(|i| (i.seq, i.envelope)).collect())
        }
        Ok(_) => Ok(vec![]),
        Err(e) if e.to_string().contains("NotFound") => Ok(vec![]),
        Err(e) => Err(e),
    }
}

pub fn ack_frame(handle: [u8; 32], queue_seq: u64) -> Frame {
    Frame::Ack {
        wake_handle: handle,
        queue_seq,
    }
}
