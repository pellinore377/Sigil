//! Operations inside a bag, their responses, error codes, and the frames
//! on the Envoy control channel. All SPE.

use crate::encoding::{Reader, Writer};

/// Operation codes. The first byte of every request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Op {
    SlotPut = 1,
    SlotGet = 2,
    SlotAck = 3,
    SlotSubscribe = 4,
    SlotUnsubscribe = 5,
    ShelfPut = 6,
    ShelfTake = 7,
    BlobPut = 8,
    BlobGet = 9,
    NameRegister = 10,
    NameLookup = 11,
    NameUpdate = 12,
    BackupPut = 13,
    BackupGet = 14,
    WrapPut = 15,
    WrapGet = 16,
    TpmInfo = 17,
    TpmRelay = 18,
    TokenCredential = 19,
    TokenIssue = 20,
    ServerInfo = 21,
    RequestsPut = 22,
}

impl Op {
    pub fn from_u8(b: u8) -> crate::Result<Op> {
        use Op::*;
        Ok(match b {
            1 => SlotPut,
            2 => SlotGet,
            3 => SlotAck,
            4 => SlotSubscribe,
            5 => SlotUnsubscribe,
            6 => ShelfPut,
            7 => ShelfTake,
            8 => BlobPut,
            9 => BlobGet,
            10 => NameRegister,
            11 => NameLookup,
            12 => NameUpdate,
            13 => BackupPut,
            14 => BackupGet,
            15 => WrapPut,
            16 => WrapGet,
            17 => TpmInfo,
            18 => TpmRelay,
            19 => TokenCredential,
            20 => TokenIssue,
            21 => ServerInfo,
            22 => RequestsPut,
            _ => return Err(crate::Error::Malformed),
        })
    }
}

/// Status byte at the start of every response. `0` is success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Status {
    Ok = 0,
    Malformed = 1,
    Unauthorized = 2,
    NotFound = 3,
    Conflict = 4,
    TokenInvalid = 5,
    TokenSpent = 6,
    RateLimited = 7,
    TooLarge = 8,
    Unavailable = 9,
    NameTaken = 10,
}

impl Status {
    pub fn from_u8(b: u8) -> crate::Result<Status> {
        let s = match b {
            0 => Status::Ok,
            1 => Status::Malformed,
            2 => Status::Unauthorized,
            3 => Status::NotFound,
            4 => Status::Conflict,
            5 => Status::TokenInvalid,
            6 => Status::TokenSpent,
            7 => Status::RateLimited,
            8 => Status::TooLarge,
            9 => Status::Unavailable,
            10 => Status::NameTaken,
            _ => return Err(crate::Error::Malformed),
        };
        Ok(s)
    }
}

/// Media and backup chunks are exactly this size, except a final chunk.
pub const CHUNK_LEN: usize = 262_144;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    SlotPut {
        address: [u8; 32],
        write_pub: [u8; 32],
        sig: [u8; 64],
        envelope: Vec<u8>,
        token: Vec<u8>,
    },
    SlotGet {
        read_cap: [u8; 32],
        write_pub: [u8; 32],
        after_seq: u64,
        limit: u16,
    },
    SlotAck {
        read_cap: [u8; 32],
        write_pub: [u8; 32],
        seq: u64,
    },
    /// `proof` is empty for a conversation slot and a 64-byte requests-read
    /// proof (spec v1 section 5) for a requests slot.
    SlotSubscribe {
        address: [u8; 32],
        wake_handle: [u8; 32],
        proof: Vec<u8>,
        token: Vec<u8>,
    },
    SlotUnsubscribe {
        address: [u8; 32],
        wake_handle: [u8; 32],
    },
    ShelfPut {
        shelf: [u8; 32],
        sealed: Vec<u8>,
        identity_pub: [u8; 32],
        sig: [u8; 64],
        token: Vec<u8>,
    },
    ShelfTake {
        shelf: [u8; 32],
    },
    BlobPut {
        chunk: Vec<u8>,
        token: Vec<u8>,
    },
    BlobGet {
        id: [u8; 32],
    },
    /// `gate` is an invite code, an OIDC ID token, or empty, per server policy.
    NameRegister {
        card: Vec<u8>,
        gate: Vec<u8>,
        token: Vec<u8>,
    },
    NameLookup {
        localpart: String,
    },
    NameUpdate {
        card: Vec<u8>,
    },
    BackupPut {
        label: [u8; 32],
        index: u32,
        chunk: Vec<u8>,
        token: Vec<u8>,
    },
    BackupGet {
        label: [u8; 32],
        index: u32,
    },
    WrapPut {
        username: String,
        salt: [u8; 16],
        wrap: Vec<u8>,
        sig: [u8; 64],
    },
    WrapGet {
        username: String,
    },
    TpmInfo,
    TpmRelay {
        username: String,
        command: Vec<u8>,
    },
    TokenCredential {
        identity_pub: [u8; 32],
        sig: [u8; 64],
        gate: Vec<u8>,
        blinded: Vec<u8>,
    },
    TokenIssue {
        credential: Vec<u8>,
        blinded: Vec<Vec<u8>>,
    },
    ServerInfo,
    RequestsPut {
        address: [u8; 32],
        envelope: Vec<u8>,
        token: Vec<u8>,
    },
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        use Request::*;
        let w = Writer::new();
        match self {
            SlotPut {
                address,
                write_pub,
                sig,
                envelope,
                token,
            } => w
                .u8(Op::SlotPut as u8)
                .fixed(address)
                .fixed(write_pub)
                .fixed(sig)
                .bytes(envelope)
                .bytes(token),
            SlotGet {
                read_cap,
                write_pub,
                after_seq,
                limit,
            } => w
                .u8(Op::SlotGet as u8)
                .fixed(read_cap)
                .fixed(write_pub)
                .u64(*after_seq)
                .u16(*limit),
            SlotAck {
                read_cap,
                write_pub,
                seq,
            } => w
                .u8(Op::SlotAck as u8)
                .fixed(read_cap)
                .fixed(write_pub)
                .u64(*seq),
            SlotSubscribe {
                address,
                wake_handle,
                proof,
                token,
            } => w
                .u8(Op::SlotSubscribe as u8)
                .fixed(address)
                .fixed(wake_handle)
                .bytes(proof)
                .bytes(token),
            SlotUnsubscribe {
                address,
                wake_handle,
            } => w
                .u8(Op::SlotUnsubscribe as u8)
                .fixed(address)
                .fixed(wake_handle),
            ShelfPut {
                shelf,
                sealed,
                identity_pub,
                sig,
                token,
            } => w
                .u8(Op::ShelfPut as u8)
                .fixed(shelf)
                .bytes(sealed)
                .fixed(identity_pub)
                .fixed(sig)
                .bytes(token),
            ShelfTake { shelf } => w.u8(Op::ShelfTake as u8).fixed(shelf),
            BlobPut { chunk, token } => w.u8(Op::BlobPut as u8).bytes(chunk).bytes(token),
            BlobGet { id } => w.u8(Op::BlobGet as u8).fixed(id),
            NameRegister { card, gate, token } => w
                .u8(Op::NameRegister as u8)
                .bytes(card)
                .bytes(gate)
                .bytes(token),
            NameLookup { localpart } => w.u8(Op::NameLookup as u8).str(localpart),
            NameUpdate { card } => w.u8(Op::NameUpdate as u8).bytes(card),
            BackupPut {
                label,
                index,
                chunk,
                token,
            } => w
                .u8(Op::BackupPut as u8)
                .fixed(label)
                .u32(*index)
                .bytes(chunk)
                .bytes(token),
            BackupGet { label, index } => w.u8(Op::BackupGet as u8).fixed(label).u32(*index),
            WrapPut {
                username,
                salt,
                wrap,
                sig,
            } => w
                .u8(Op::WrapPut as u8)
                .str(username)
                .fixed(salt)
                .bytes(wrap)
                .fixed(sig),
            WrapGet { username } => w.u8(Op::WrapGet as u8).str(username),
            TpmInfo => w.u8(Op::TpmInfo as u8),
            TpmRelay { username, command } => w.u8(Op::TpmRelay as u8).str(username).bytes(command),
            TokenCredential {
                identity_pub,
                sig,
                gate,
                blinded,
            } => w
                .u8(Op::TokenCredential as u8)
                .fixed(identity_pub)
                .fixed(sig)
                .bytes(gate)
                .bytes(blinded),
            TokenIssue {
                credential,
                blinded,
            } => {
                let mut w = w
                    .u8(Op::TokenIssue as u8)
                    .bytes(credential)
                    .u16(blinded.len() as u16);
                for b in blinded {
                    w = w.bytes(b);
                }
                w
            }
            ServerInfo => w.u8(Op::ServerInfo as u8),
            RequestsPut {
                address,
                envelope,
                token,
            } => w
                .u8(Op::RequestsPut as u8)
                .fixed(address)
                .bytes(envelope)
                .bytes(token),
        }
        .finish()
    }

    pub fn decode(b: &[u8]) -> crate::Result<Request> {
        use Request::*;
        let mut r = Reader::new(b);
        let op = Op::from_u8(r.u8()?)?;
        let req = match op {
            Op::SlotPut => SlotPut {
                address: r.fixed()?,
                write_pub: r.fixed()?,
                sig: r.fixed()?,
                envelope: r.bytes()?.to_vec(),
                token: r.bytes()?.to_vec(),
            },
            Op::SlotGet => SlotGet {
                read_cap: r.fixed()?,
                write_pub: r.fixed()?,
                after_seq: r.u64()?,
                limit: r.u16()?,
            },
            Op::SlotAck => SlotAck {
                read_cap: r.fixed()?,
                write_pub: r.fixed()?,
                seq: r.u64()?,
            },
            Op::SlotSubscribe => SlotSubscribe {
                address: r.fixed()?,
                wake_handle: r.fixed()?,
                proof: r.bytes()?.to_vec(),
                token: r.bytes()?.to_vec(),
            },
            Op::SlotUnsubscribe => SlotUnsubscribe {
                address: r.fixed()?,
                wake_handle: r.fixed()?,
            },
            Op::ShelfPut => ShelfPut {
                shelf: r.fixed()?,
                sealed: r.bytes()?.to_vec(),
                identity_pub: r.fixed()?,
                sig: r.fixed()?,
                token: r.bytes()?.to_vec(),
            },
            Op::ShelfTake => ShelfTake { shelf: r.fixed()? },
            Op::BlobPut => BlobPut {
                chunk: r.bytes()?.to_vec(),
                token: r.bytes()?.to_vec(),
            },
            Op::BlobGet => BlobGet { id: r.fixed()? },
            Op::NameRegister => NameRegister {
                card: r.bytes()?.to_vec(),
                gate: r.bytes()?.to_vec(),
                token: r.bytes()?.to_vec(),
            },
            Op::NameLookup => NameLookup {
                localpart: r.str()?.to_string(),
            },
            Op::NameUpdate => NameUpdate {
                card: r.bytes()?.to_vec(),
            },
            Op::BackupPut => BackupPut {
                label: r.fixed()?,
                index: r.u32()?,
                chunk: r.bytes()?.to_vec(),
                token: r.bytes()?.to_vec(),
            },
            Op::BackupGet => BackupGet {
                label: r.fixed()?,
                index: r.u32()?,
            },
            Op::WrapPut => WrapPut {
                username: r.str()?.to_string(),
                salt: r.fixed()?,
                wrap: r.bytes()?.to_vec(),
                sig: r.fixed()?,
            },
            Op::WrapGet => WrapGet {
                username: r.str()?.to_string(),
            },
            Op::TpmInfo => TpmInfo,
            Op::TpmRelay => TpmRelay {
                username: r.str()?.to_string(),
                command: r.bytes()?.to_vec(),
            },
            Op::TokenCredential => TokenCredential {
                identity_pub: r.fixed()?,
                sig: r.fixed()?,
                gate: r.bytes()?.to_vec(),
                blinded: r.bytes()?.to_vec(),
            },
            Op::TokenIssue => {
                let credential = r.bytes()?.to_vec();
                let n = r.u16()?;
                let mut blinded = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    blinded.push(r.bytes()?.to_vec());
                }
                TokenIssue {
                    credential,
                    blinded,
                }
            }
            Op::ServerInfo => ServerInfo,
            Op::RequestsPut => RequestsPut {
                address: r.fixed()?,
                envelope: r.bytes()?.to_vec(),
                token: r.bytes()?.to_vec(),
            },
        };
        r.done()?;
        Ok(req)
    }
}

/// One stored envelope with its sequence number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub seq: u64,
    pub envelope: Vec<u8>,
}

/// Response bodies. An error response is the status byte alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Error(Status),
    SlotPut {
        seq: u64,
    },
    SlotGet {
        items: Vec<Stored>,
        more: bool,
    },
    Empty,
    ShelfTake {
        sealed: Vec<u8>,
    },
    BlobPut {
        id: [u8; 32],
    },
    Bytes(Vec<u8>),
    WrapGet {
        salt: [u8; 16],
        wrap: Vec<u8>,
    },
    TpmInfo {
        ek_pub: Vec<u8>,
        cert_chain: Vec<u8>,
    },
    TokenCredential {
        blind_sig: Vec<u8>,
    },
    TokenIssue {
        blind_sigs: Vec<Vec<u8>>,
    },
}

impl Response {
    /// Encoding depends on the operation it answers, so `op` is required.
    pub fn encode(&self) -> Vec<u8> {
        use Response::*;
        let w = Writer::new();
        match self {
            Error(s) => w.u8(*s as u8),
            SlotPut { seq } => w.u8(0).u64(*seq),
            SlotGet { items, more } => {
                let mut w = w.u8(0).u16(items.len() as u16);
                for it in items {
                    w = w.u64(it.seq).bytes(&it.envelope);
                }
                w.u8(*more as u8)
            }
            Empty => w.u8(0),
            ShelfTake { sealed } => w.u8(0).bytes(sealed),
            BlobPut { id } => w.u8(0).fixed(id),
            Bytes(b) => w.u8(0).bytes(b),
            WrapGet { salt, wrap } => w.u8(0).fixed(salt).bytes(wrap),
            TpmInfo { ek_pub, cert_chain } => w.u8(0).bytes(ek_pub).bytes(cert_chain),
            TokenCredential { blind_sig } => w.u8(0).bytes(blind_sig),
            TokenIssue { blind_sigs } => {
                let mut w = w.u8(0).u16(blind_sigs.len() as u16);
                for b in blind_sigs {
                    w = w.bytes(b);
                }
                w
            }
        }
        .finish()
    }

    pub fn decode(op: Op, b: &[u8]) -> crate::Result<Response> {
        use Response::*;
        let mut r = Reader::new(b);
        let status = Status::from_u8(r.u8()?)?;
        if status != Status::Ok {
            r.done()?;
            return Ok(Error(status));
        }
        let resp = match op {
            Op::SlotPut => SlotPut { seq: r.u64()? },
            Op::SlotGet => {
                let n = r.u16()?;
                let mut items = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    items.push(Stored {
                        seq: r.u64()?,
                        envelope: r.bytes()?.to_vec(),
                    });
                }
                SlotGet {
                    items,
                    more: r.u8()? != 0,
                }
            }
            Op::SlotAck
            | Op::SlotSubscribe
            | Op::SlotUnsubscribe
            | Op::ShelfPut
            | Op::NameRegister
            | Op::NameUpdate
            | Op::BackupPut
            | Op::WrapPut
            | Op::RequestsPut => Empty,
            Op::ShelfTake => ShelfTake {
                sealed: r.bytes()?.to_vec(),
            },
            Op::BlobPut => BlobPut { id: r.fixed()? },
            Op::BlobGet | Op::NameLookup | Op::BackupGet | Op::TpmRelay | Op::ServerInfo => {
                Bytes(r.bytes()?.to_vec())
            }
            Op::WrapGet => WrapGet {
                salt: r.fixed()?,
                wrap: r.bytes()?.to_vec(),
            },
            Op::TpmInfo => TpmInfo {
                ek_pub: r.bytes()?.to_vec(),
                cert_chain: r.bytes()?.to_vec(),
            },
            Op::TokenCredential => TokenCredential {
                blind_sig: r.bytes()?.to_vec(),
            },
            Op::TokenIssue => {
                let n = r.u16()?;
                let mut blind_sigs = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    blind_sigs.push(r.bytes()?.to_vec());
                }
                TokenIssue { blind_sigs }
            }
        };
        r.done()?;
        Ok(resp)
    }
}

/// What a server publishes about itself (`server.info`), SPE, signed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCard {
    pub hostname: String,
    pub kem_pub: Vec<u8>,
    /// SPKI DER of the current token issuing key.
    pub token_key: Vec<u8>,
    /// Bit 0: TPM recovery. Bit 1: OIDC gate on registration. Bit 2: open registration.
    pub flags: u8,
    /// Ed25519 public key the server signs cards and receipts with.
    pub signing_pub: [u8; 32],
}

impl ServerCard {
    pub fn encode(&self) -> Vec<u8> {
        Writer::new()
            .u8(crate::VERSION)
            .str(&self.hostname)
            .fixed(&self.kem_pub)
            .bytes(&self.token_key)
            .u8(self.flags)
            .fixed(&self.signing_pub)
            .finish()
    }
    pub fn decode(b: &[u8]) -> crate::Result<ServerCard> {
        let mut r = Reader::new(b);
        if r.u8()? != crate::VERSION {
            return Err(crate::Error::Malformed);
        }
        let hostname = r.str()?.to_string();
        let kem_pub = r.fixed::<{ crate::kem::PUBLIC_KEY_LEN }>()?.to_vec();
        let token_key = r.bytes()?.to_vec();
        let flags = r.u8()?;
        let signing_pub = r.fixed()?;
        r.done()?;
        Ok(ServerCard {
            hostname,
            kem_pub,
            token_key,
            flags,
            signing_pub,
        })
    }
}

/// Frames on the client–Envoy control channel and the Envoy–server
/// delivery stream. First byte is the frame type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// Client → Envoy: forward `bag` to `server`. If `bind_handle` is set,
    /// the Envoy records handle → this connection before forwarding.
    Bag {
        id: u32,
        server: String,
        bind_handle: Option<[u8; 32]>,
        bag: Vec<u8>,
    },
    /// Envoy → client: the server's sealed response to `id`.
    BagResponse { id: u32, response: Vec<u8> },
    /// Envoy → client (and server → Envoy): an envelope for a handle.
    /// `queue_seq` is per handle, assigned by the Envoy (0 on the server
    /// stream). `slot_seq` is the envelope's sequence number in its slot,
    /// so the client can dedupe against a backfill and track a cursor.
    Deliver {
        wake_handle: [u8; 32],
        queue_seq: u64,
        slot_seq: u64,
        envelope: Vec<u8>,
    },
    /// Client → Envoy: everything up to `queue_seq` for this handle is stored.
    Ack {
        wake_handle: [u8; 32],
        queue_seq: u64,
    },
    /// Client → Envoy: how to wake this connection's device when it is gone.
    /// kind 0 none, 1 APNs, 2 FCM, 3 UnifiedPush.
    Push { kind: u8, token: Vec<u8> },
    /// Client → Envoy: forget this handle.
    Release { wake_handle: [u8; 32] },
    /// Server → Envoy, every 30 s: liveness plus the current requests-read nonce.
    Keepalive { nonce: [u8; 32] },
    /// Client → Envoy with an all-zero nonce: "what is `server`'s current
    /// nonce?"; Envoy → client with the last one seen on that server's stream.
    Nonce { server: String, nonce: [u8; 32] },
}

impl Frame {
    pub fn encode(&self) -> Vec<u8> {
        use Frame::*;
        let w = Writer::new();
        match self {
            Bag {
                id,
                server,
                bind_handle,
                bag,
            } => {
                let w = w.u8(1).u32(*id).str(server);
                let w = match bind_handle {
                    Some(h) => w.u8(1).fixed(h),
                    None => w.u8(0),
                };
                w.bytes(bag)
            }
            BagResponse { id, response } => w.u8(2).u32(*id).bytes(response),
            Deliver {
                wake_handle,
                queue_seq,
                slot_seq,
                envelope,
            } => w.u8(3).fixed(wake_handle).u64(*queue_seq).u64(*slot_seq).bytes(envelope),
            Ack {
                wake_handle,
                queue_seq,
            } => w.u8(4).fixed(wake_handle).u64(*queue_seq),
            Push { kind, token } => w.u8(5).u8(*kind).bytes(token),
            Release { wake_handle } => w.u8(6).fixed(wake_handle),
            Keepalive { nonce } => w.u8(7).fixed(nonce),
            Nonce { server, nonce } => w.u8(8).str(server).fixed(nonce),
        }
        .finish()
    }

    pub fn decode(b: &[u8]) -> crate::Result<Frame> {
        use Frame::*;
        let mut r = Reader::new(b);
        let f = match r.u8()? {
            1 => {
                let id = r.u32()?;
                let server = r.str()?.to_string();
                let bind_handle = match r.u8()? {
                    0 => None,
                    1 => Some(r.fixed()?),
                    _ => return Err(crate::Error::Malformed),
                };
                Bag {
                    id,
                    server,
                    bind_handle,
                    bag: r.bytes()?.to_vec(),
                }
            }
            2 => BagResponse {
                id: r.u32()?,
                response: r.bytes()?.to_vec(),
            },
            3 => Deliver {
                wake_handle: r.fixed()?,
                queue_seq: r.u64()?,
                slot_seq: r.u64()?,
                envelope: r.bytes()?.to_vec(),
            },
            4 => Ack {
                wake_handle: r.fixed()?,
                queue_seq: r.u64()?,
            },
            5 => Push {
                kind: r.u8()?,
                token: r.bytes()?.to_vec(),
            },
            6 => Release {
                wake_handle: r.fixed()?,
            },
            7 => Keepalive { nonce: r.fixed()? },
            8 => Nonce {
                server: r.str()?.to_string(),
                nonce: r.fixed()?,
            },
            _ => return Err(crate::Error::Malformed),
        };
        r.done()?;
        Ok(f)
    }
}
