//! The home role: open a bag, run one operation, seal the answer.

use crate::config::Config;
use crate::delivery::Delivery;
use crate::store::{
    key2, key_seq, today, SlotMeta, Store, ACKS, BACKUPS, BLOBS, BLOB_EXPIRY, ENVELOPES, INVITES,
    NAMES, OIDC_SUBS, REQ_OWNER, SHELVES, SLOTS, WRAPS,
};
use crate::store::ESCROW;
use crate::tokens::TokenService;
use ed25519_dalek::{Signature, Verifier as _, VerifyingKey};
use redb::ReadableTable;
use sigil_protocol::identity::ContactCard;
use sigil_protocol::wire::{Request, Response, ServerCard, Status, Stored, CHUNK_LEN};
use sigil_protocol::{bag, epoch, kem, names, token};
use std::sync::Arc;

/// Per-name recovery attempt backoff: 1 s after the first attempt in an
/// hour, doubling, capped at an hour. Lives in memory only; it is a rate
/// limit, not a record.
#[derive(Default)]
pub struct Backoff {
    until: std::collections::HashMap<String, (std::time::Instant, u64)>,
}

impl Backoff {
    /// Returns false if the name must wait.
    pub fn allow(&mut self, name: &str) -> bool {
        let now = std::time::Instant::now();
        let e = self.until.entry(name.to_string()).or_insert((now, 0));
        if now < e.0 {
            return false;
        }
        let secs = if e.1 == 0 { 1 } else { (e.1 * 2).min(3600) };
        if now.duration_since(e.0) > std::time::Duration::from_secs(3600) {
            *e = (now, 0);
        }
        e.1 = secs;
        e.0 = now + std::time::Duration::from_secs(secs);
        true
    }
}

pub struct Home {
    pub cfg: Config,
    pub backoff: std::sync::Mutex<Backoff>,
    pub store: Arc<Store>,
    pub kem: kem::SecretKey,
    pub tokens: TokenService,
    pub delivery: Delivery,
    pub card: Vec<u8>,
    /// The call forwarding unit, when `calls` is on.
    pub sfu: Option<Arc<crate::sfu::Sfu>>,
    /// The OIDC gate, when `registration = "oidc"`.
    pub oidc: Option<crate::oidc::Oidc>,
}

const SLOT_TTL_DAYS: u32 = 30;

impl Home {
    pub fn new(cfg: Config, store: Arc<Store>) -> anyhow::Result<Arc<Home>> {
        let kem = kem::keypair(&store.meta_seed("kem_seed")?);
        let signing = ed25519_dalek::SigningKey::from_bytes(&store.meta_seed("signing_seed")?);
        let tokens = TokenService::new();
        let token_key = tokens.current(&store, "token")?.spki.clone();
        let mut flags = 0u8;
        if cfg.registration == "open" {
            flags |= 0b100;
        }
        if cfg.registration == "oidc" {
            flags |= 0b010;
        }
        if cfg.recovery_mode() == "escrow" {
            flags |= 0b1000;
        }
        if crate::tpm::available() {
            flags |= 0b001;
        }
        let card = ServerCard {
            hostname: cfg.hostname.clone(),
            kem_pub: kem.public().to_vec(),
            token_key,
            flags,
            signing_pub: signing.verifying_key().to_bytes(),
        };
        let mut signed = card.encode();
        let mut msg = b"sigil v1 server card".to_vec();
        msg.extend_from_slice(&signed);
        signed.extend_from_slice(&ed25519_dalek::Signer::sign(&signing, &msg).to_bytes());
        let sfu = if cfg.calls {
            match crate::sfu::Sfu::start(&cfg.media_udp, cfg.media_public.as_deref()) {
                Ok(s) => {
                    tracing::info!("calls on: participants send media to {}", s.public);
                    Some(s)
                }
                Err(e) => {
                    tracing::warn!("calls disabled: forwarding unit failed to start: {e}");
                    None
                }
            }
        } else {
            None
        };
        let oidc = match (&cfg.oidc_issuer, &cfg.oidc_client_id) {
            (Some(i), Some(c)) if cfg.registration == "oidc" => {
                tracing::info!("registration through {i}");
                Some(crate::oidc::Oidc::new(i, c))
            }
            _ => None,
        };
        Ok(Arc::new(Home {
            backoff: std::sync::Mutex::new(Backoff::default()),
            cfg,
            store,
            kem,
            tokens,
            delivery: Delivery::new(),
            card: signed,
            sfu,
            oidc,
        }))
    }

    /// The whole request path. Returns the sealed response, or None if the
    /// bag itself could not be opened (there is no key to seal an answer with).
    pub async fn handle_bag(&self, bag_bytes: &[u8], envoy: &[u8; 32]) -> Option<Vec<u8>> {
        let (plain, keys) = bag::open_request(&self.kem, bag_bytes).ok()?;
        let response = match Request::decode(&plain) {
            Ok(req) => match self.handle(req, envoy).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("operation failed: {e}");
                    Response::Error(Status::Unavailable)
                }
            },
            Err(_) => Response::Error(Status::Malformed),
        };
        let nonce: [u8; 24] = rand::random();
        bag::seal_response(&keys, &nonce, &response.encode()).ok()
    }

    fn spend(&self, token: &[u8]) -> anyhow::Result<Result<(), Status>> {
        self.tokens.spend(&self.store, "token", token)
    }

    async fn handle(&self, req: Request, envoy: &[u8; 32]) -> anyhow::Result<Response> {
        use Request::*;
        Ok(match req {
            ServerInfo => Response::Bytes(self.card.clone()),

            CallSignal { room, body, token } => {
                let Some(sfu) = &self.sfu else {
                    return Ok(Response::Error(Status::Unavailable));
                };
                if body.len() > 65536 {
                    return Ok(Response::Error(Status::TooLarge));
                }
                // joining costs a token; the rest of a call is free
                if crate::sfu::kind_of(&body).as_deref() == Some("join") {
                    if let Err(s) = self.spend(&token)? {
                        return Ok(Response::Error(s));
                    }
                }
                match sfu.signal(room, &body).await {
                    Ok(v) => Response::Bytes(serde_json::to_vec(&v)?),
                    Err(e) => {
                        Response::Bytes(serde_json::to_vec(&serde_json::json!({"error": e}))?)
                    }
                }
            }

            SlotPut {
                address,
                write_pub,
                sig,
                envelope,
                token,
            } => {
                if !sigil_protocol::envelope::BUCKETS.contains(&envelope.len()) {
                    return Ok(Response::Error(Status::TooLarge));
                }
                if epoch::verify_put(&write_pub, &address, &envelope, &sig).is_err() {
                    return Ok(Response::Error(Status::Unauthorized));
                }
                if let Err(s) = self.spend(&token)? {
                    return Ok(Response::Error(s));
                }
                let seq = {
                    let w = self.store.db.begin_write()?;
                    let seq = {
                        let mut slots = w.open_table(SLOTS)?;
                        let mut meta = match slots
                            .get(address.as_slice())?
                            .map(|v| SlotMeta::decode(v.value()))
                        {
                            Some(Some(m)) => {
                                if m.kind != 0 || m.write_pub != write_pub {
                                    return Ok(Response::Error(Status::Conflict));
                                }
                                m
                            }
                            _ => SlotMeta {
                                write_pub,
                                next_seq: 1,
                                expiry_day: 0,
                                kind: 0,
                            },
                        };
                        let seq = meta.next_seq;
                        meta.next_seq += 1;
                        meta.expiry_day = today() + SLOT_TTL_DAYS;
                        slots.insert(address.as_slice(), meta.encode().as_slice())?;
                        w.open_table(ENVELOPES)?
                            .insert(key_seq(&address, seq).as_slice(), envelope.as_slice())?;
                        seq
                    };
                    w.commit()?;
                    seq
                };
                // a slot nobody has subscribed to (a cover write, or a stale
                // address) is kept a day, not a month
                if seq == 1 && self.delivery.subscribers(&self.store, &address)?.is_empty() {
                    let w = self.store.db.begin_write()?;
                    {
                        let mut slots = w.open_table(SLOTS)?;
                        let meta = slots
                            .get(address.as_slice())?
                            .and_then(|v| SlotMeta::decode(v.value()));
                        if let Some(mut m) = meta {
                            m.expiry_day = today() + 1;
                            slots.insert(address.as_slice(), m.encode().as_slice())?;
                        }
                    }
                    w.commit()?;
                }
                self.delivery
                    .deliver(&self.store, &address, seq, &envelope)
                    .await?;
                Response::SlotPut { seq }
            }

            RequestsPut {
                address,
                envelope,
                token,
            } => {
                if !sigil_protocol::requests::BUCKETS.contains(&envelope.len()) {
                    return Ok(Response::Error(Status::TooLarge));
                }
                {
                    let r = self.store.db.begin_read()?;
                    if r.open_table(REQ_OWNER)?.get(address.as_slice())?.is_none() {
                        return Ok(Response::Error(Status::NotFound));
                    }
                }
                if let Err(s) = self.spend(&token)? {
                    return Ok(Response::Error(s));
                }
                let seq = {
                    let w = self.store.db.begin_write()?;
                    let seq = {
                        let mut slots = w.open_table(SLOTS)?;
                        let mut meta = slots
                            .get(address.as_slice())?
                            .and_then(|v| SlotMeta::decode(v.value()))
                            .unwrap_or(SlotMeta {
                                write_pub: [0; 32],
                                next_seq: 1,
                                expiry_day: 0,
                                kind: 1,
                            });
                        let seq = meta.next_seq;
                        meta.next_seq += 1;
                        meta.expiry_day = today() + SLOT_TTL_DAYS;
                        slots.insert(address.as_slice(), meta.encode().as_slice())?;
                        w.open_table(ENVELOPES)?
                            .insert(key_seq(&address, seq).as_slice(), envelope.as_slice())?;
                        seq
                    };
                    w.commit()?;
                    seq
                };
                self.delivery
                    .deliver(&self.store, &address, seq, &envelope)
                    .await?;
                Response::Empty
            }

            SlotGet {
                read_cap,
                write_pub,
                after_seq,
                limit,
            } => {
                let address = epoch::slot_address(&read_cap, &write_pub);
                let r = self.store.db.begin_read()?;
                match r
                    .open_table(SLOTS)?
                    .get(address.as_slice())?
                    .and_then(|v| SlotMeta::decode(v.value()))
                {
                    Some(m) if m.kind == 0 && m.write_pub == write_pub => {}
                    _ => return Ok(Response::Error(Status::NotFound)),
                }
                let limit = limit.clamp(1, 64) as usize;
                let t = r.open_table(ENVELOPES)?;
                let lo = key_seq(&address, after_seq.saturating_add(1));
                let hi = key_seq(&address, u64::MAX);
                let mut items = Vec::new();
                let mut more = false;
                for item in t.range(lo.as_slice()..=hi.as_slice())? {
                    let (k, v) = item?;
                    if items.len() == limit {
                        more = true;
                        break;
                    }
                    let seq = u64::from_be_bytes(k.value()[32..40].try_into().unwrap());
                    items.push(Stored {
                        seq,
                        envelope: v.value().to_vec(),
                    });
                }
                Response::SlotGet { items, more }
            }

            SlotAck {
                read_cap,
                write_pub,
                seq,
            } => {
                let address = epoch::slot_address(&read_cap, &write_pub);
                let w = self.store.db.begin_write()?;
                {
                    let slots = w.open_table(SLOTS)?;
                    match slots
                        .get(address.as_slice())?
                        .and_then(|v| SlotMeta::decode(v.value()))
                    {
                        Some(m) if m.write_pub == write_pub => {}
                        _ => return Ok(Response::Error(Status::NotFound)),
                    }
                    w.open_table(ACKS)?
                        .insert(key2(&address, &read_cap).as_slice(), seq)?;
                }
                w.commit()?;
                Response::Empty
            }

            SlotSubscribe {
                address,
                wake_handle,
                proof,
                token,
            } => {
                if !proof.is_empty() {
                    // requests slot: prove ownership against a recent nonce
                    let Ok(proof64): Result<[u8; 64], _> = proof.as_slice().try_into() else {
                        return Ok(Response::Error(Status::Malformed));
                    };
                    let owner = self.find_requests_owner(&address, &proof64, envoy)?;
                    let Some(identity_pub) = owner else {
                        return Ok(Response::Error(Status::Unauthorized));
                    };
                    if let Err(s) = self.spend(&token)? {
                        return Ok(Response::Error(s));
                    }
                    let w = self.store.db.begin_write()?;
                    w.open_table(REQ_OWNER)?
                        .insert(address.as_slice(), identity_pub.as_slice())?;
                    w.commit()?;
                } else if let Err(s) = self.spend(&token)? {
                    return Ok(Response::Error(s));
                }
                self.delivery
                    .subscribe(&self.store, &address, &wake_handle, envoy)?;
                Response::Empty
            }

            SlotUnsubscribe {
                address,
                wake_handle,
            } => {
                self.delivery
                    .unsubscribe(&self.store, &address, &wake_handle)?;
                Response::Empty
            }

            ShelfPut {
                shelf,
                sealed,
                identity_pub,
                sig,
                token,
            } => {
                if names::shelf_address(&identity_pub) != shelf {
                    return Ok(Response::Error(Status::Unauthorized));
                }
                let mut msg = b"sigil v1 shelf put".to_vec();
                msg.extend_from_slice(&shelf);
                msg.extend_from_slice(&sealed);
                if !verify(&identity_pub, &msg, &sig) {
                    return Ok(Response::Error(Status::Unauthorized));
                }
                if let Err(s) = self.spend(&token)? {
                    return Ok(Response::Error(s));
                }
                let w = self.store.db.begin_write()?;
                w.open_table(SHELVES)?
                    .insert(shelf.as_slice(), sealed.as_slice())?;
                w.commit()?;
                Response::Empty
            }

            ShelfTake { shelf } => {
                let w = self.store.db.begin_write()?;
                let out = {
                    let mut t = w.open_table(SHELVES)?;
                    let Some(list) = t.get(shelf.as_slice())?.map(|v| v.value().to_vec()) else {
                        return Ok(Response::Error(Status::NotFound));
                    };
                    // SPE list: count u16, then (bytes)×count. Pop the last.
                    let mut r = sigil_protocol::encoding::Reader::new(&list);
                    let n = r.u16().unwrap_or(0);
                    if n == 0 {
                        return Ok(Response::Error(Status::NotFound));
                    }
                    let mut pkgs = Vec::new();
                    for _ in 0..n {
                        pkgs.push(
                            r.bytes()
                                .map_err(|_| anyhow::anyhow!("bad shelf"))?
                                .to_vec(),
                        );
                    }
                    let taken = pkgs.pop().unwrap();
                    let mut w2 = sigil_protocol::encoding::Writer::new().u16(pkgs.len() as u16);
                    for p in &pkgs {
                        w2 = w2.bytes(p);
                    }
                    t.insert(shelf.as_slice(), w2.finish().as_slice())?;
                    taken
                };
                w.commit()?;
                Response::ShelfTake { sealed: out }
            }

            BlobPut { chunk, token } => {
                if chunk.is_empty() || chunk.len() > CHUNK_LEN || chunk.len() % 4096 != 0 {
                    return Ok(Response::Error(Status::TooLarge));
                }
                if let Err(s) = self.spend(&token)? {
                    return Ok(Response::Error(s));
                }
                let id = sigil_protocol::kdf::hash(&chunk);
                let w = self.store.db.begin_write()?;
                w.open_table(BLOBS)?
                    .insert(id.as_slice(), chunk.as_slice())?;
                w.open_table(BLOB_EXPIRY)?
                    .insert(id.as_slice(), today() + SLOT_TTL_DAYS)?;
                w.commit()?;
                Response::BlobPut { id }
            }

            BlobGet { id } => {
                let r = self.store.db.begin_read()?;
                match r.open_table(BLOBS)?.get(id.as_slice())? {
                    Some(v) => Response::Bytes(v.value().to_vec()),
                    None => Response::Error(Status::NotFound),
                }
            }

            NameRegister { card, gate, token } => {
                let Ok(parsed) = ContactCard::verify(&card) else {
                    return Ok(Response::Error(Status::Unauthorized));
                };
                let Ok((local, server)) = names::parse_username(&parsed.username) else {
                    return Ok(Response::Error(Status::Malformed));
                };
                if server != self.cfg.hostname {
                    return Ok(Response::Error(Status::Unauthorized));
                }
                let local = local.to_string();
                // The OIDC gate: the token must verify, and one login holds
                // one name here. The mapping is the only trace the login
                // leaves, and it never meets a conversation.
                let mut oidc_sub = None;
                if self.cfg.registration == "oidc" {
                    let Some(o) = &self.oidc else {
                        return Ok(Response::Error(Status::Unavailable));
                    };
                    let token_str = String::from_utf8_lossy(&gate).to_string();
                    let claims = match o.verify(token_str.trim()).await {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::debug!("oidc: token refused: {e}");
                            return Ok(Response::Error(Status::Unauthorized));
                        }
                    };
                    let r = self.store.db.begin_read()?;
                    if let Some(held) = r.open_table(OIDC_SUBS)?.get(claims.sub.as_str())? {
                        if held.value() != local {
                            return Ok(Response::Error(Status::Unauthorized));
                        }
                    }
                    oidc_sub = Some(claims.sub);
                }
                if self.cfg.registration == "invite" {
                    let code = String::from_utf8_lossy(&gate).to_string();
                    let w = self.store.db.begin_write()?;
                    {
                        let mut t = w.open_table(INVITES)?;
                        if t.get(code.as_str())?.is_none() {
                            return Ok(Response::Error(Status::Unauthorized));
                        }
                        t.remove(code.as_str())?;
                    }
                    w.commit()?;
                }
                if !token.is_empty() {
                    if let Err(s) = self.spend(&token)? {
                        return Ok(Response::Error(s));
                    }
                }
                let w = self.store.db.begin_write()?;
                {
                    let mut t = w.open_table(NAMES)?;
                    if t.get(local.as_str())?.is_some() {
                        return Ok(Response::Error(Status::NameTaken));
                    }
                    t.insert(local.as_str(), card.as_slice())?;
                    if let Some(sub) = &oidc_sub {
                        w.open_table(OIDC_SUBS)?
                            .insert(sub.as_str(), local.as_str())?;
                    }
                }
                w.commit()?;
                Response::Empty
            }

            NameLookup { localpart } => {
                let r = self.store.db.begin_read()?;
                match r.open_table(NAMES)?.get(localpart.as_str())? {
                    Some(v) => Response::Bytes(v.value().to_vec()),
                    None => Response::Error(Status::NotFound),
                }
            }

            NameUpdate { card } => {
                let Ok(parsed) = ContactCard::verify(&card) else {
                    return Ok(Response::Error(Status::Unauthorized));
                };
                let Ok((local, _)) = names::parse_username(&parsed.username) else {
                    return Ok(Response::Error(Status::Malformed));
                };
                let local = local.to_string();
                let w = self.store.db.begin_write()?;
                {
                    let mut t = w.open_table(NAMES)?;
                    let Some(old) = t.get(local.as_str())?.map(|v| v.value().to_vec()) else {
                        return Ok(Response::Error(Status::NotFound));
                    };
                    if ContactCard::verify(&old).map(|c| c.identity_pub) != Ok(parsed.identity_pub)
                    {
                        return Ok(Response::Error(Status::Unauthorized));
                    }
                    t.insert(local.as_str(), card.as_slice())?;
                }
                w.commit()?;
                Response::Empty
            }

            BackupPut {
                label,
                index,
                chunk,
                token,
            } => {
                if chunk.len() > CHUNK_LEN {
                    return Ok(Response::Error(Status::TooLarge));
                }
                if let Err(s) = self.spend(&token)? {
                    return Ok(Response::Error(s));
                }
                let w = self.store.db.begin_write()?;
                w.open_table(BACKUPS)?.insert(
                    key2(&label, &index.to_be_bytes()).as_slice(),
                    chunk.as_slice(),
                )?;
                w.commit()?;
                Response::Empty
            }

            BackupGet { label, index } => {
                let r = self.store.db.begin_read()?;
                match r
                    .open_table(BACKUPS)?
                    .get(key2(&label, &index.to_be_bytes()).as_slice())?
                {
                    Some(v) => Response::Bytes(v.value().to_vec()),
                    None => Response::Error(Status::NotFound),
                }
            }

            WrapPut {
                username,
                salt,
                wrap,
                sig,
            } => {
                let Some(identity_pub) = self.identity_for(&username)? else {
                    return Ok(Response::Error(Status::NotFound));
                };
                let mut msg = b"sigil v1 wrap put".to_vec();
                msg.extend_from_slice(&salt);
                msg.extend_from_slice(&wrap);
                if !verify(&identity_pub, &msg, &sig) {
                    return Ok(Response::Error(Status::Unauthorized));
                }
                let w = self.store.db.begin_write()?;
                let mut v = salt.to_vec();
                v.extend_from_slice(&wrap);
                w.open_table(WRAPS)?
                    .insert(username.as_str(), v.as_slice())?;
                w.commit()?;
                Response::Empty
            }

            EscrowPut {
                username,
                escrow,
                sig,
            } => {
                if self.cfg.recovery_mode() != "escrow" {
                    return Ok(Response::Error(Status::Unavailable));
                }
                let Some(identity_pub) = self.identity_for(&username)? else {
                    return Ok(Response::Error(Status::NotFound));
                };
                let mut msg = b"sigil v1 escrow put".to_vec();
                msg.extend_from_slice(&escrow);
                if !verify(&identity_pub, &msg, &sig) {
                    return Ok(Response::Error(Status::Unauthorized));
                }
                let w = self.store.db.begin_write()?;
                w.open_table(ESCROW)?
                    .insert(username.as_str(), escrow.as_slice())?;
                w.commit()?;
                Response::Empty
            }

            // The escrow leaves only for the login that holds the name (when
            // the gate is on) and never faster than the per-name backoff, so
            // a password can only be guessed at the pace the server allows.
            EscrowGet { username, gate } => {
                if self.cfg.recovery_mode() != "escrow" {
                    return Ok(Response::Error(Status::Unavailable));
                }
                // Under the gate a try costs a fresh sign-in as the holder,
                // which is the limit; the restore's wrap.get still pays the
                // backoff. Without the gate the backoff is all there is.
                if self.cfg.registration != "oidc"
                    && !self.backoff.lock().unwrap().allow(&username)
                {
                    return Ok(Response::Error(Status::RateLimited));
                }
                if self.cfg.registration == "oidc" {
                    let Some(o) = &self.oidc else {
                        return Ok(Response::Error(Status::Unavailable));
                    };
                    let token_str = String::from_utf8_lossy(&gate).to_string();
                    let Ok(claims) = o.verify(token_str.trim()).await else {
                        return Ok(Response::Error(Status::Unauthorized));
                    };
                    let Ok((local, _)) = names::parse_username(&username) else {
                        return Ok(Response::Error(Status::Malformed));
                    };
                    let r = self.store.db.begin_read()?;
                    let held = r
                        .open_table(OIDC_SUBS)?
                        .get(claims.sub.as_str())?
                        .map(|v| v.value().to_string());
                    if held.as_deref() != Some(local) {
                        return Ok(Response::Error(Status::Unauthorized));
                    }
                }
                let r = self.store.db.begin_read()?;
                match r.open_table(ESCROW)?.get(username.as_str())? {
                    Some(v) => Response::Bytes(v.value().to_vec()),
                    None => Response::Error(Status::NotFound),
                }
            }

            WrapGet { username } => {
                if !self.backoff.lock().unwrap().allow(&username) {
                    return Ok(Response::Error(Status::RateLimited));
                }
                let r = self.store.db.begin_read()?;
                match r.open_table(WRAPS)?.get(username.as_str())? {
                    Some(v) => {
                        let v = v.value();
                        Response::WrapGet {
                            salt: v[..16].try_into().unwrap(),
                            wrap: v[16..].to_vec(),
                        }
                    }
                    None => Response::Error(Status::NotFound),
                }
            }

            TpmInfo => match crate::tpm::info() {
                Some((ek, chain)) => Response::TpmInfo {
                    ek_pub: ek,
                    cert_chain: chain,
                },
                None => Response::Error(Status::Unavailable),
            },
            TpmRelay { username, command } => {
                if !crate::tpm::available() {
                    return Ok(Response::Error(Status::Unavailable));
                }
                if !self.backoff.lock().unwrap().allow(&username) {
                    return Ok(Response::Error(Status::RateLimited));
                }
                match crate::tpm::relay(&command) {
                    Ok(resp) => Response::Bytes(resp),
                    Err(_) => Response::Error(Status::Unavailable),
                }
            }

            TokenCredential {
                identity_pub,
                sig,
                gate,
                blinded,
            } => {
                let mut msg = b"sigil v1 credential".to_vec();
                msg.extend_from_slice(&blinded);
                if !verify(&identity_pub, &msg, &sig) {
                    return Ok(Response::Error(Status::Unauthorized));
                }
                // An Envoy with an authenticated stream may hold a credential for
                // cover traffic: its signing key is its identity, and the bag
                // arrived on its own stream.
                let is_envoy =
                    gate == b"envoy" && self.cfg.cover_credentials && identity_pub == *envoy;
                if !is_envoy && !self.has_card(&identity_pub)? {
                    return Ok(Response::Error(Status::Unauthorized));
                }
                if token::SIG_LEN != blinded.len() {
                    return Ok(Response::Error(Status::Malformed));
                }
                if !self.tokens.credential_once(&self.store, &identity_pub)? {
                    return Ok(Response::Error(Status::RateLimited));
                }
                let issuer = self.tokens.current(&self.store, "credential")?;
                match issuer.sign(&blinded) {
                    Ok(bs) => Response::TokenCredential { blind_sig: bs },
                    Err(_) => Response::Error(Status::Malformed),
                }
            }

            TokenIssue {
                credential,
                blinded,
            } => {
                if blinded.len() > 64 || blinded.iter().any(|b| b.len() != token::SIG_LEN) {
                    return Ok(Response::Error(Status::Malformed));
                }
                let Ok(cred) = token::Token::decode(&credential) else {
                    return Ok(Response::Error(Status::TokenInvalid));
                };
                // a credential is verified but not spent: it is reused daily
                let ok = [self.tokens.current(&self.store, "credential")?]
                    .iter()
                    .any(|i| {
                        i.key_id == cred.key_id
                            && token::Verifier::from_spki(&i.spki)
                                .map(|v| v.verify(&cred).is_ok())
                                .unwrap_or(false)
                    });
                if !ok {
                    return Ok(Response::Error(Status::TokenInvalid));
                }
                if !self.tokens.quota(
                    &self.store,
                    &cred,
                    blinded.len() as u32,
                    self.cfg.tokens_per_day,
                )? {
                    return Ok(Response::Error(Status::RateLimited));
                }
                let issuer = self.tokens.current(&self.store, "token")?;
                let mut sigs = Vec::with_capacity(blinded.len());
                for b in &blinded {
                    match issuer.sign(b) {
                        Ok(s) => sigs.push(s),
                        Err(_) => return Ok(Response::Error(Status::Malformed)),
                    }
                }
                Response::TokenIssue { blind_sigs: sigs }
            }
        })
    }

    fn identity_for(&self, username: &str) -> anyhow::Result<Option<[u8; 32]>> {
        let Ok((local, _)) = names::parse_username(username) else {
            return Ok(None);
        };
        let r = self.store.db.begin_read()?;
        Ok(r.open_table(NAMES)?
            .get(local)?
            .and_then(|v| ContactCard::verify(v.value()).ok())
            .map(|c| c.identity_pub))
    }

    fn has_card(&self, identity_pub: &[u8; 32]) -> anyhow::Result<bool> {
        let r = self.store.db.begin_read()?;
        let t = r.open_table(NAMES)?;
        for item in t.iter()? {
            let (_, v) = item?;
            if ContactCard::verify(v.value())
                .map(|c| c.identity_pub == *identity_pub)
                .unwrap_or(false)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Which registered identity, if any, owns `address` as its requests
    /// slot for the current or next period, proven by `proof` against one
    /// of this Envoy's recent nonces.
    fn find_requests_owner(
        &self,
        address: &[u8; 32],
        proof: &[u8; 64],
        envoy: &[u8; 32],
    ) -> anyhow::Result<Option<[u8; 32]>> {
        let nonces = self.delivery.recent_nonces(envoy);
        let period = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs()
            / 2_592_000) as u32;
        let r = self.store.db.begin_read()?;
        let t = r.open_table(NAMES)?;
        for item in t.iter()? {
            let (_, v) = item?;
            let Ok(card) = ContactCard::verify(v.value()) else {
                continue;
            };
            for p in [period, period + 1] {
                if names::requests_address(&card.identity_pub, p) == *address {
                    for n in &nonces {
                        if names::verify_requests_read_proof(&card.identity_pub, address, n, proof)
                            .is_ok()
                        {
                            return Ok(Some(card.identity_pub));
                        }
                    }
                    return Ok(None);
                }
            }
        }
        Ok(None)
    }
}

fn verify(pk: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> bool {
    VerifyingKey::from_bytes(pk)
        .map(|vk| vk.verify(msg, &Signature::from_bytes(sig)).is_ok())
        .unwrap_or(false)
}
