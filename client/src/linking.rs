//! Device linking. The new device shows an offer; an existing device scans
//! it, writes a KEM ciphertext into the offer slot, both derive the link
//! slot and the emoji, the user confirms on the existing device, and the
//! transfer runs through the link slot:
//!
//!   existing → new   Transfer  (identity, credential, tokens, conversations, extra)
//!   new → existing   KeyPackage (the new device's MLS leaf)
//!   existing → new   Welcome × conversations, after an Add commit in each
//!   existing → new   Done
//!
//! The new device has no tokens until the Transfer arrives, so it polls
//! with free reads; the existing device pays for every write.

use crate::account::{mls_credential, CIPHERSUITE};
use crate::conversation::{self, join_config};
use crate::provider::SigilProvider;
use crate::state::Conversation;
use crate::{Link, State};
use openmls::prelude::*;
use sigil_protocol::encoding::{Reader, Writer};
use sigil_protocol::epoch::{self, EpochMaterial};
use sigil_protocol::linking::{self, LinkOffer};
use sigil_protocol::wire::{Request, Response};
use sigil_protocol::{envelope, kem};
use std::path::Path;
use std::time::Duration;
use tls_codec::{Deserialize as _, Serialize as _};

const TAG_TRANSFER: u8 = 1;
const TAG_KEY_PACKAGE: u8 = 2;
const TAG_WELCOME: u8 = 3;
const TAG_DONE: u8 = 4;

/// A pending offer on the new device.
pub struct Offer {
    secret: kem::SecretKey,
    pub offer: LinkOffer,
}

impl Default for Offer {
    fn default() -> Self {
        Self::new()
    }
}

impl Offer {
    pub fn new() -> Offer {
        let secret = kem::keypair(&rand::random());
        let offer = LinkOffer {
            kem_pub: secret.public().to_vec(),
            nonce: rand::random(),
        };
        Offer { secret, offer }
    }
    /// What the QR code shows.
    pub fn text(&self) -> String {
        format!("sigil-link:v1:{}", hex::encode(self.offer.encode()))
    }
}

pub fn parse_offer(text: &str) -> anyhow::Result<LinkOffer> {
    let hexs = text
        .trim()
        .strip_prefix("sigil-link:v1:")
        .ok_or_else(|| anyhow::anyhow!("not a Sigil link offer"))?;
    LinkOffer::decode(&hex::decode(hexs)?).map_err(|e| anyhow::anyhow!("{e:?}"))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Write one link event into a slot; the caller pays.
async fn put(
    link: &Link,
    st: &mut State,
    server: &str,
    slot: &EpochMaterial,
    body: Vec<u8>,
) -> anyhow::Result<u64> {
    let ev = envelope::Event {
        kind: envelope::Kind::Link as u16,
        ts_ms: now_ms(),
        reference: vec![],
        body,
    };
    let sealed = envelope::seal(
        &slot.envelope_key,
        &slot.address,
        &rand::random(),
        &ev.encode(),
    )
    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let sig = epoch::sign_put(&slot.write_key, &slot.address, &sealed);
    let token = st.take_token()?;
    let resp = link
        .call(
            server,
            &Request::SlotPut {
                address: slot.address,
                write_pub: slot.write_pub,
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
    Ok(seq)
}

/// Read link events after `after_seq`; free.
async fn get(
    link: &Link,
    server: &str,
    slot: &EpochMaterial,
    after_seq: u64,
) -> anyhow::Result<Vec<(u64, Vec<u8>)>> {
    let resp = link
        .call(
            server,
            &Request::SlotGet {
                read_cap: slot.read_cap,
                write_pub: slot.write_pub,
                after_seq,
                limit: 64,
            },
            None,
        )
        .await;
    let items = match resp {
        Ok(Response::SlotGet { items, .. }) => items,
        Ok(_) => vec![],
        Err(e) if e.to_string().contains("NotFound") => vec![],
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for it in items {
        let plain = envelope::open(&slot.envelope_key, &slot.address, &it.envelope)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let ev = envelope::Event::decode(&plain).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        out.push((it.seq, ev.body));
    }
    Ok(out)
}

/// Wait for the next link event after `after_seq`, polling.
async fn next(
    link: &Link,
    server: &str,
    slot: &EpochMaterial,
    after_seq: &mut u64,
    timeout: Duration,
) -> anyhow::Result<Vec<u8>> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let items = get(link, server, slot, *after_seq).await?;
        if let Some((seq, body)) = items.into_iter().next() {
            *after_seq = seq;
            return Ok(body);
        }
        if tokio::time::Instant::now() > deadline {
            anyhow::bail!("timed out waiting for the other device");
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
    }
}

pub struct Transfer {
    pub username: String,
    pub envoy: String,
    pub identity_seed: [u8; 32],
    pub credential: Vec<u8>,
    pub tokens: Vec<Vec<u8>>,
    pub conversations: Vec<Conversation>,
    /// Caller-defined bytes (the engine sends its history).
    pub extra: Vec<u8>,
    /// The recovery record (salt, recovery key, data key), so the new
    /// device can change the password and back up too; the vouching
    /// device is where the key lives, and it hands a copy over here.
    pub recovery: Option<crate::backup::Recovery>,
}

impl Transfer {
    fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new()
            .u8(TAG_TRANSFER)
            .str(&self.username)
            .str(&self.envoy)
            .fixed(&self.identity_seed)
            .bytes(&self.credential)
            .u16(self.tokens.len() as u16);
        for t in &self.tokens {
            w = w.bytes(t);
        }
        w.bytes(&serde_json::to_vec(&self.conversations).unwrap())
            .bytes(&self.extra)
            .bytes(&serde_json::to_vec(&self.recovery).unwrap())
            .finish()
    }
    fn decode(b: &[u8]) -> anyhow::Result<Transfer> {
        let mut r = Reader::new(b);
        if r.u8().map_err(|_| anyhow::anyhow!("short"))? != TAG_TRANSFER {
            anyhow::bail!("not a transfer");
        }
        let e = |_| anyhow::anyhow!("malformed transfer");
        let username = r.str().map_err(e)?.to_string();
        let envoy = r.str().map_err(e)?.to_string();
        let identity_seed = r.fixed().map_err(e)?;
        let credential = r.bytes().map_err(e)?.to_vec();
        let n = r.u16().map_err(e)?;
        let mut tokens = Vec::new();
        for _ in 0..n {
            tokens.push(r.bytes().map_err(e)?.to_vec());
        }
        let conversations = serde_json::from_slice(r.bytes().map_err(e)?)?;
        let extra = r.bytes().map_err(e)?.to_vec();
        let recovery = serde_json::from_slice(r.bytes().map_err(e)?).unwrap_or(None);
        Ok(Transfer {
            username,
            envoy,
            identity_seed,
            credential,
            tokens,
            conversations,
            extra,
            recovery,
        })
    }
}

/// Progress reported to the caller on either side.
pub enum Progress {
    /// Both sides show this; the existing device must confirm it.
    Sas(String),
    Welcomed(String),
    Done,
}

// ---------------------------------------------------------------- new device

/// Run the new-device side to completion. `server` and `envoy` are where
/// the account lives (the user typed the username first). On success the
/// account file at `path` is written and the state returned, along with
/// the existing device's `extra` bytes.
pub async fn wait_for_link(
    path: &Path,
    server: &str,
    envoy: &str,
    offer: &Offer,
    mut progress: impl FnMut(Progress),
) -> anyhow::Result<(State, Vec<u8>)> {
    let device_id = hex::encode(rand::random::<[u8; 32]>());
    let link = Link::connect(envoy, &device_id).await?;
    // 1. the ciphertext in the offer slot
    let oslot = offer.offer.slot();
    let mut oseq = 0;
    let ct = next(&link, server, &oslot, &mut oseq, Duration::from_secs(600)).await?;
    let shared = offer
        .secret
        .decapsulate(&ct)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let lm = linking::derive(&shared, &offer.offer);
    progress(Progress::Sas(linking::sas_string(&lm.sas)));
    // 2. the transfer
    let mut lseq = 0;
    let body = next(&link, server, &lm.slot, &mut lseq, Duration::from_secs(300)).await?;
    let t = Transfer::decode(&body)?;
    let mut st = State {
        identity_seed: hex::encode(t.identity_seed),
        device_seed: hex::encode(rand::random::<[u8; 32]>()),
        username: t.username.clone(),
        envoy: t.envoy.clone(),
        device_id,
        credential: Some(hex::encode(&t.credential)),
        tokens: t.tokens.iter().map(hex::encode).collect(),
        conversations: Vec::new(),
        requests: Vec::new(),
        seen_requests: Vec::new(),
        recovery: t.recovery.clone(),
        path: path.to_path_buf(),
    };
    st.save()?;
    // the old device's wallet may be nearly empty; draw our own
    let _ = crate::account::ensure_tokens(&link, &mut st, 20, 40).await;
    let provider = SigilProvider::open(&st.mls_path())?;
    // 3. our key package, so the existing device can add this leaf
    let (cred, signer) = mls_credential(&st);
    let kp = KeyPackage::builder()
        .build(CIPHERSUITE, &provider, &signer, cred)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    provider.save()?;
    let kp_bytes = kp.key_package().tls_serialize_detached()?;
    put(
        &link,
        &mut st,
        server,
        &lm.slot,
        Writer::new().u8(TAG_KEY_PACKAGE).bytes(&kp_bytes).finish(),
    )
    .await?;
    lseq += 1; // our own write
               // 4. welcomes until done
    loop {
        let body = next(&link, server, &lm.slot, &mut lseq, Duration::from_secs(300)).await?;
        let mut r = Reader::new(&body);
        match r.u8().unwrap_or(0) {
            TAG_WELCOME => {
                let e = |_| anyhow::anyhow!("malformed welcome");
                let conv: Conversation = serde_json::from_slice(r.bytes().map_err(e)?)?;
                let welcome_bytes = r.bytes().map_err(e)?.to_vec();
                let msg = MlsMessageIn::tls_deserialize_exact(&welcome_bytes)?;
                let MlsMessageBodyIn::Welcome(w) = msg.extract() else {
                    anyhow::bail!("not a welcome")
                };
                let staged = StagedWelcome::new_from_welcome(&provider, &join_config(), w, None)
                    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
                let group = staged
                    .into_group(&provider)
                    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
                provider.save()?;
                let mut c = conv;
                c.group_id = hex::encode(group.group_id().as_slice());
                c.sent.clear();
                c.epochs.clear();
                c.cursors.clear();
                progress(Progress::Welcomed(c.peers.join(", ")));
                st.conversations.push(c);
                st.save()?;
            }
            TAG_DONE => break,
            TAG_KEY_PACKAGE => continue,
            _ => anyhow::bail!("unexpected link event"),
        }
    }
    progress(Progress::Done);
    Ok((st, t.extra))
}

// ---------------------------------------------------------------- existing device

/// Scan an offer: encapsulate, write the ciphertext, and return the SAS
/// plus a token to continue with once the user has confirmed it.
pub struct Scanned {
    pub sas: String,
    lm: linking::LinkMaterial,
}

pub async fn scan(link: &Link, st: &mut State, offer_text: &str) -> anyhow::Result<Scanned> {
    let offer = parse_offer(offer_text)?;
    let (ct, shared) =
        kem::encapsulate(&offer.kem_pub, &rand::random()).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let server = st.server();
    put(link, st, &server, &offer.slot(), ct).await?;
    let lm = linking::derive(&shared, &offer);
    Ok(Scanned {
        sas: linking::sas_string(&lm.sas),
        lm,
    })
}

/// After the user confirmed the emoji: run the transfer. Adds the new
/// device to every conversation, which rotates every address; the caller
/// re-subscribes afterwards.
pub async fn transfer(
    link: &Link,
    st: &mut State,
    provider: &SigilProvider,
    scanned: Scanned,
    extra: Vec<u8>,
    mut progress: impl FnMut(Progress),
) -> anyhow::Result<()> {
    let server = st.server();
    let slot = &scanned.lm.slot;
    let give = st.tokens.len() / 2;
    let tokens: Vec<Vec<u8>> = st
        .tokens
        .drain(..give)
        .map(|t| hex::decode(t).unwrap())
        .collect();
    st.save()?;
    let t = Transfer {
        username: st.username.clone(),
        envoy: st.envoy.clone(),
        identity_seed: hex::decode(&st.identity_seed)?.try_into().unwrap(),
        credential: hex::decode(st.credential.as_deref().unwrap_or(""))?,
        tokens,
        conversations: st
            .conversations
            .iter()
            .map(|c| Conversation {
                sent: Default::default(),
                ..c.clone()
            })
            .collect(),
        extra,
        recovery: st.recovery.clone(),
    };
    let mut lseq = put(link, st, &server, slot, t.encode()).await?;
    // the new device's key package
    let kp = loop {
        let body = next(link, &server, slot, &mut lseq, Duration::from_secs(300)).await?;
        let mut r = Reader::new(&body);
        if r.u8().unwrap_or(0) == TAG_KEY_PACKAGE {
            let bytes = r
                .bytes()
                .map_err(|_| anyhow::anyhow!("malformed key package"))?;
            let kp_in = KeyPackageIn::tls_deserialize_exact(bytes)?;
            let kp = kp_in
                .validate(provider.crypto(), ProtocolVersion::Mls10)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            crate::account::check_credential(kp.leaf_node().credential(), &st.identity().public())?;
            break kp;
        }
    };
    // add the leaf to every conversation
    let (_, signer) = mls_credential(st);
    let convs = st.conversations.clone();
    for conv in &convs {
        let mut group = conversation::load_group(provider, conv)?;
        let (commit, welcome, _) = group
            .add_members(provider, &signer, std::slice::from_ref(&kp))
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        // the commit goes to the conversation's current slot, for the other members
        let ep = conversation::epoch_material(&group, provider)?;
        conversation::record_epoch(st, conv, &ep);
        let sealed = envelope::seal(
            &ep.envelope_key,
            &ep.address,
            &rand::random(),
            &commit.to_bytes()?,
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
        if let Response::SlotPut { seq } = resp {
            if let Some(c) = st
                .conversations
                .iter_mut()
                .find(|c| c.group_id == conv.group_id)
            {
                c.sent.insert(
                    format!("{}:{seq}", hex::encode(ep.address)),
                    format!("{}\u{1f}", now_ms()),
                );
            }
        }
        group
            .merge_pending_commit(provider)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        provider.save()?;
        st.save()?;
        let body = Writer::new()
            .u8(TAG_WELCOME)
            .bytes(&serde_json::to_vec(&Conversation {
                sent: Default::default(),
                ..conv.clone()
            })?)
            .bytes(&welcome.to_bytes()?)
            .finish();
        put(link, st, &server, slot, body).await?;
        progress(Progress::Welcomed(conv.peers.join(", ")));
    }
    put(link, st, &server, slot, vec![TAG_DONE]).await?;
    progress(Progress::Done);
    Ok(())
}
