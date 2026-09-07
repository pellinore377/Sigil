//! Experimental Sender Keys profile; see docs/SenderKeys.md. Authenticated
//! distribution, membership ordering and durable commits belong to the client.
//! Symmetric advancement does not provide post-compromise healing.
use crate::{
    checkpoint::{Reader, Writer},
    derive_chain,
    identity::validate_public,
    random_bytes,
    skipped::Skipped,
    storage::StorageKey,
    verify_signature, Error, IdentityKey, MessageKey, Secret32, MAX_PLAINTEXT,
};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;
type Id = [u8; 32];
const PACKET: &[u8; 8] = b"SGKM\0\x01\0\0";
const DISTRIBUTION: &[u8; 8] = b"SGKD\0\x01\0\0";
const HEADER: usize = 188;
pub const MAX_PACKET: usize = HEADER + MAX_PLAINTEXT + 16 + 64;
pub const MAX_SKIPPED_KEYS: usize = crate::skipped::MAX;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Context {
    pub group: Id,
    pub state: Id,
    pub epoch: u64,
    pub sender: Id,
    pub chain: Id,
}
impl Context {
    fn bytes(&self) -> Vec<u8> {
        [
            &self.group[..],
            &self.state,
            &self.epoch.to_be_bytes(),
            &self.sender,
            &self.chain,
        ]
        .concat()
    }
    fn read(reader: &mut Reader<'_>) -> Result<Self, Error> {
        Ok(Self {
            group: reader.take()?,
            state: reader.take()?,
            epoch: reader.u64()?,
            sender: reader.take()?,
            chain: reader.take()?,
        })
    }
    fn checkpoint_aad(&self, role: &[u8], binding: &[u8]) -> Vec<u8> {
        [
            b"Sigil/sender-key-checkpoint/v0".as_slice(),
            role,
            &self.bytes(),
            binding,
        ]
        .concat()
    }
}

pub struct Distribution {
    context: Context,
    chain: Secret32,
    public: Id,
}
impl Distribution {
    pub fn context(&self) -> Context {
        self.context
    }
    /// Sensitive plaintext: only authenticated pairwise encryption may carry it.
    pub fn to_bytes(&self) -> Result<Zeroizing<Vec<u8>>, Error> {
        let mut out = Writer::new();
        out.put(DISTRIBUTION)?;
        out.put(&self.context.bytes())?;
        out.put(self.chain.0.as_ref())?;
        out.put(&self.public)?;
        Ok(out.finish())
    }
    /// Parsing proves neither origin nor membership; the caller must validate
    /// those against the authenticated pairwise sender and committed group state.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() != 208 {
            return Err(Error::Encoding);
        }
        let mut reader = Reader::new(bytes)?;
        if &reader.take::<8>()? != DISTRIBUTION {
            return Err(Error::Encoding);
        }
        let context = Context::read(&mut reader)?;
        let chain = reader.secret()?;
        let public = reader.take()?;
        validate_public(&public)?;
        reader.finish()?;
        Ok(Self {
            context,
            chain,
            public,
        })
    }
}

pub struct Packet {
    context: Context,
    counter: u64,
    message: Id,
    ciphertext: Vec<u8>,
    signature: [u8; 64],
}
impl Packet {
    pub fn context(&self) -> Context {
        self.context
    }
    pub fn counter(&self) -> u64 {
        self.counter
    }
    pub fn message(&self) -> Id {
        self.message
    }
    fn header(&self) -> Vec<u8> {
        self.header_with_length(self.ciphertext.len())
    }
    fn header_with_length(&self, length: usize) -> Vec<u8> {
        [
            PACKET.as_slice(),
            &self.context.bytes(),
            &self.counter.to_be_bytes(),
            &self.message,
            &(length as u32).to_be_bytes(),
        ]
        .concat()
    }
    fn statement(&self) -> Vec<u8> {
        let mut hash = Sha256::new();
        hash.update(self.header());
        hash.update(&self.ciphertext);
        [
            b"Sigil/sender-key-signature/v0".as_slice(),
            hash.finalize().as_slice(),
        ]
        .concat()
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        [
            self.header().as_slice(),
            self.ciphertext.as_slice(),
            &self.signature,
        ]
        .concat()
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if !(HEADER + 16 + 64..=MAX_PACKET).contains(&bytes.len()) {
            return Err(Error::Limit);
        }
        let mut reader = Reader::archive(bytes)?;
        if &reader.take::<8>()? != PACKET {
            return Err(Error::Encoding);
        }
        let context = Context::read(&mut reader)?;
        let counter = reader.u64()?;
        let message = reader.take()?;
        let ciphertext = reader.blob(MAX_PLAINTEXT + 16)?.to_vec();
        if ciphertext.len() < 16 {
            return Err(Error::Encoding);
        }
        let signature = reader.take()?;
        reader.finish()?;
        Ok(Self {
            context,
            counter,
            message,
            ciphertext,
            signature,
        })
    }
}

pub struct Sender {
    context: Context,
    chain: Secret32,
    signing: IdentityKey,
    counter: u64,
}
impl Sender {
    pub fn new(
        group: Id,
        state: Id,
        epoch: u64,
        sender: Id,
    ) -> Result<(Self, Distribution), Error> {
        let context = Context {
            group,
            state,
            epoch,
            sender,
            chain: *random_bytes()?,
        };
        let chain = Secret32::generate()?;
        let signing = IdentityKey::generate()?;
        let distribution = Distribution {
            context,
            chain: Secret32::from_bytes(*chain.0),
            public: signing.public_key(),
        };
        Ok((
            Self {
                context,
                chain,
                signing,
                counter: 0,
            },
            distribution,
        ))
    }
    pub fn context(&self) -> Context {
        self.context
    }
    pub fn counter(&self) -> u64 {
        self.counter
    }
    /// The caller must durably freeze encrypted distribution before the first
    /// send, then commit each checkpoint and packet in one transaction.
    pub fn seal(&mut self, message: Id, plaintext: &[u8]) -> Result<Packet, Error> {
        if plaintext.len() > MAX_PLAINTEXT {
            return Err(Error::Limit);
        }
        let next_counter = self.counter.checked_add(1).ok_or(Error::Limit)?;
        let (next_chain, message_key) = derive_chain(&self.chain)?;
        let mut packet = Packet {
            context: self.context,
            counter: self.counter,
            message,
            ciphertext: Vec::new(),
            signature: [0; 64],
        };
        packet.ciphertext =
            message_key.seal(plaintext, &packet.header_with_length(plaintext.len() + 16))?;
        packet.signature = self.signing.sign(&packet.statement())?;
        self.chain = next_chain;
        self.counter = next_counter;
        Ok(packet)
    }
    pub fn seal_checkpoint(&self, key: &StorageKey, binding: &[u8]) -> Result<Vec<u8>, Error> {
        let aad = self.context.checkpoint_aad(b"sender", binding);
        let mut out = Writer::new();
        out.put(b"SGKS\0\x01\0\0")?;
        out.put(&self.context.bytes())?;
        out.put(self.chain.0.as_ref())?;
        out.u64(self.counter)?;
        out.blob(&self.signing.seal_checkpoint(key, &aad)?)?;
        key.seal(&out.finish(), &aad)
    }
    pub fn open_checkpoint(
        key: &StorageKey,
        sealed: &[u8],
        expected: Context,
        binding: &[u8],
    ) -> Result<Self, Error> {
        let aad = expected.checkpoint_aad(b"sender", binding);
        let bytes = key.open(sealed, &aad)?;
        let mut reader = Reader::new(&bytes)?;
        if reader.take::<8>()? != *b"SGKS\0\x01\0\0" || Context::read(&mut reader)? != expected {
            return Err(Error::Encoding);
        }
        let chain = reader.secret()?;
        let counter = reader.u64()?;
        let signing = IdentityKey::open_checkpoint(key, reader.blob(76)?, &aad)?;
        reader.finish()?;
        Ok(Self {
            context: expected,
            chain,
            counter,
            signing,
        })
    }
}

pub struct Receiver {
    context: Context,
    chain: Secret32,
    public: Id,
    counter: u64,
    skipped: Skipped<u64>,
}
impl Receiver {
    /// The caller has authenticated the distribution's exact origin and group
    /// membership. It must not overwrite an already advanced receiver with it.
    pub fn from_authenticated_distribution(
        distribution: Distribution,
        expected: Context,
    ) -> Result<Self, Error> {
        if distribution.context != expected {
            return Err(Error::Authentication);
        }
        Ok(Self {
            context: expected,
            chain: distribution.chain,
            public: distribution.public,
            counter: 0,
            skipped: Skipped::new(),
        })
    }
    pub fn context(&self) -> Context {
        self.context
    }
    pub fn counter(&self) -> u64 {
        self.counter
    }
    pub fn open(&mut self, packet: &Packet) -> Result<Vec<u8>, Error> {
        if packet.context != self.context {
            return Err(Error::Authentication);
        }
        verify_signature(&self.public, &packet.statement(), &packet.signature)?;
        let mut skipped = self.skipped.candidate();
        if packet.counter < self.counter {
            let key = skipped.remove(&packet.counter).ok_or(Error::Replay)?;
            let plaintext = key.open(&packet.ciphertext, &packet.header())?;
            self.skipped = skipped;
            return Ok(plaintext);
        }
        let gap = packet.counter - self.counter;
        if gap > MAX_SKIPPED_KEYS as u64 {
            return Err(Error::Limit);
        }
        let next_counter = packet.counter.checked_add(1).ok_or(Error::Limit)?;
        let mut chain = Secret32::from_bytes(*self.chain.0);
        for index in self.counter..packet.counter {
            let (next, key) = derive_chain(&chain)?;
            skipped.insert(index, key);
            chain = next;
        }
        let (chain, key) = derive_chain(&chain)?;
        let plaintext = key.open(&packet.ciphertext, &packet.header())?;
        self.chain = chain;
        self.counter = next_counter;
        self.skipped = skipped;
        Ok(plaintext)
    }
    pub fn seal_checkpoint(&self, key: &StorageKey, binding: &[u8]) -> Result<Vec<u8>, Error> {
        let mut out = Writer::new();
        out.put(b"SGKR\0\x01\0\0")?;
        out.put(&self.context.bytes())?;
        out.put(self.chain.0.as_ref())?;
        out.put(&self.public)?;
        out.u64(self.counter)?;
        out.u32(self.skipped.len() as u32)?;
        for (counter, message_key) in self.skipped.iter() {
            out.u64(*counter)?;
            out.put(message_key.0 .0.as_ref())?;
        }
        key.seal(
            &out.finish(),
            &self.context.checkpoint_aad(b"receiver", binding),
        )
    }
    pub fn open_checkpoint(
        key: &StorageKey,
        sealed: &[u8],
        expected: Context,
        binding: &[u8],
    ) -> Result<Self, Error> {
        let bytes = key.open(sealed, &expected.checkpoint_aad(b"receiver", binding))?;
        let mut reader = Reader::new(&bytes)?;
        if reader.take::<8>()? != *b"SGKR\0\x01\0\0" || Context::read(&mut reader)? != expected {
            return Err(Error::Encoding);
        }
        let chain = reader.secret()?;
        let public = reader.take()?;
        validate_public(&public)?;
        let counter = reader.u64()?;
        let count = reader.u32()? as usize;
        if count > MAX_SKIPPED_KEYS {
            return Err(Error::Limit);
        }
        let mut skipped = Skipped::new();
        for _ in 0..count {
            let index = reader.u64()?;
            let key = MessageKey(reader.secret()?);
            if index >= counter || skipped.insert(index, key).is_some() {
                return Err(Error::Encoding);
            }
        }
        reader.finish()?;
        Ok(Self {
            context: expected,
            chain,
            public,
            counter,
            skipped,
        })
    }
}

#[cfg(test)]
#[path = "sender_key_tests.rs"]
mod tests;
