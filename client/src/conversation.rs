//! Conversations are MLS groups. This module creates them, sends and
//! accepts Welcomes through the requests slot, derives the slot from the
//! epoch secret, and moves events in and out.

use crate::account::{check_credential, mls_credential, take_key_package, CIPHERSUITE};
use crate::provider::SigilProvider;
use crate::state::Conversation;
use crate::{Link, State};
use openmls::prelude::*;
use sigil_protocol::encoding::{Reader, Writer};
use sigil_protocol::epoch::{self, EpochMaterial};
use sigil_protocol::identity::ContactCard;
use sigil_protocol::wire::{Frame, Request, Response};
use sigil_protocol::{envelope, names, requests};
use tls_codec::Deserialize as _;

pub fn create_config() -> MlsGroupCreateConfig {
    MlsGroupCreateConfig::builder()
        .ciphersuite(CIPHERSUITE)
        .use_ratchet_tree_extension(true)
        .build()
}
pub fn join_config() -> MlsGroupJoinConfig {
    MlsGroupJoinConfig::builder()
        .use_ratchet_tree_extension(true)
        .build()
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

/// Start a direct message with `username`: take a key package, make the
/// group, send the Welcome and first message to their requests slot.
pub async fn start_dm(
    link: &Link,
    st: &mut State,
    provider: &SigilProvider,
    username: &str,
    first_text: &str,
) -> anyhow::Result<Conversation> {
    let card = crate::account::lookup(link, username).await?;
    let kp = take_key_package(link, provider, &card).await?;
    let (cred, signer) = mls_credential(st);
    let group_id = GroupId::from_slice(&rand::random::<[u8; 32]>());
    let mut group =
        MlsGroup::new_with_group_id(provider, &signer, &create_config(), group_id.clone(), cred)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let (_commit, welcome, _) = group
        .add_members(provider, &signer, &[kp])
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    group
        .merge_pending_commit(provider)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    provider.save()?;

    // The Welcome event: body = SPE(welcome bytes, our signed card, first text)
    let welcome_bytes = welcome.to_bytes()?;
    let body = Writer::new()
        .bytes(&welcome_bytes)
        .bytes(&crate::account::contact_card(st))
        .str(first_text)
        .finish();
    let ev = envelope::Event {
        kind: envelope::Kind::Welcome as u16,
        ts_ms: now_ms(),
        reference: vec![],
        body,
    };
    let period = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs()
        / 2_592_000) as u32;
    let address = names::requests_address(&card.identity_pub, period);
    let sealed = requests::seal(
        &card.kem_pub,
        &address,
        &rand::random(),
        &rand::random(),
        &ev.encode(),
    )
    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let token = st.take_token()?;
    link.call(
        &card.slot_server,
        &Request::RequestsPut {
            address,
            envelope: sealed,
            token,
        },
        None,
    )
    .await?;

    let conv = Conversation {
        group_id: hex::encode(group_id.as_slice()),
        peers: vec![username.to_string()],
        slot_server: st.server(),
        cursors: Default::default(),
        sent: Default::default(),
    };
    st.conversations.push(conv.clone());
    st.save()?;
    Ok(conv)
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
    let card = ContactCard::verify(&card_bytes).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    Ok(crate::state::PendingRequest {
        from: card.username,
        from_card: hex::encode(card_bytes),
        welcome: hex::encode(welcome),
        first_message: first,
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
    let conv = Conversation {
        group_id: hex::encode(group.group_id().as_slice()),
        peers: vec![req.from.clone()],
        slot_server: card.slot_server.clone(),
        cursors: Default::default(),
        sent: Default::default(),
    };
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

/// Send any event into the conversation's current slot. Returns the slot
/// sequence number and the address it landed in.
pub async fn send_event(
    link: &Link,
    st: &mut State,
    provider: &SigilProvider,
    conv: &Conversation,
    kind: envelope::Kind,
    reference: &[u8],
    body: &[u8],
) -> anyhow::Result<(u64, [u8; 32])> {
    let mut group = load_group(provider, conv)?;
    let (_, signer) = mls_credential(st);
    let ev = envelope::Event { kind: kind as u16, ts_ms: now_ms(), reference: reference.to_vec(), body: body.to_vec() };
    let mls_out = group.create_message(provider, &signer, &ev.encode()).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    provider.save()?;
    let ep = epoch_material(&group, provider)?;
    let sealed = envelope::seal(&ep.envelope_key, &ep.address, &rand::random(), &mls_out.to_bytes()?).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let sig = epoch::sign_put(&ep.write_key, &ep.address, &sealed);
    let token = st.take_token()?;
    let resp = link
        .call(&conv.slot_server, &Request::SlotPut { address: ep.address, write_pub: ep.write_pub, sig, envelope: sealed, token }, None)
        .await?;
    let Response::SlotPut { seq } = resp else { anyhow::bail!("unexpected") };
    if let Some(c) = st.conversations.iter_mut().find(|c| c.group_id == conv.group_id) {
        // Text is kept for readback; other kinds only need the seq to be known as ours.
        let text = if kind == envelope::Kind::Text { String::from_utf8_lossy(body).to_string() } else { String::new() };
        c.sent.insert(format!("{}:{seq}", hex::encode(ep.address)), text);
        st.save()?;
    }
    Ok((seq, ep.address))
}

/// Send a text event into the conversation's current slot.
pub async fn send_text(link: &Link, st: &mut State, provider: &SigilProvider, conv: &Conversation, text: &str) -> anyhow::Result<u64> {
    Ok(send_event(link, st, provider, conv, envelope::Kind::Text, &[], text.as_bytes()).await?.0)
}

/// Text this device sent at `seq` in `address`, if any.
pub fn own_sent(st: &State, conv: &Conversation, address: &[u8; 32], seq: u64) -> Option<String> {
    st.conversations
        .iter()
        .find(|c| c.group_id == conv.group_id)
        .and_then(|c| {
            c.sent
                .get(&format!("{}:{seq}", hex::encode(address)))
                .cloned()
        })
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
    let mut group = load_group(provider, conv)?;
    let ep = epoch_material(&group, provider)?;
    let mls_bytes = envelope::open(&ep.envelope_key, &ep.address, sealed)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
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
                Incoming::Text { from_identity: sender_identity, ts_ms: ev.ts_ms, text: String::from_utf8_lossy(&ev.body).to_string(), reference }
            } else {
                Incoming::Event { from_identity: sender_identity, ts_ms: ev.ts_ms, kind: ev.kind, reference, body: String::from_utf8_lossy(&ev.body).to_string() }
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
