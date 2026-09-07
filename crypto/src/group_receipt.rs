//! Authority ordering evidence is distinct from member authorization.
use crate::{verify_signature, Error, IdentityKey};
type Id = [u8; 32];
const PREFIX: &[u8; 8] = b"SGGR\0\x01\0\0";

pub use crate::private_group::authority_fingerprint;

pub struct Receipt {
    pub group: Id,
    pub predecessor: Id,
    pub head: Id,
    pub revision: u64,
    public: Id,
    signature: [u8; 64],
}
impl Receipt {
    /// The authority must perform authorization and durable compare-and-swap
    /// before issuing a receipt. This primitive alone performs neither.
    pub fn sign(
        group: Id,
        predecessor: Id,
        head: Id,
        revision: u64,
        key: &IdentityKey,
    ) -> Result<Self, Error> {
        if revision == 0 || predecessor == head {
            return Err(Error::Encoding);
        }
        let mut receipt = Self {
            group,
            predecessor,
            head,
            revision,
            public: key.public_key(),
            signature: [0; 64],
        };
        receipt.signature = key.sign(&receipt.statement())?;
        Ok(receipt)
    }
    fn statement(&self) -> Vec<u8> {
        [
            b"Sigil/group-ordering/v0".as_slice(),
            &self.group,
            &self.predecessor,
            &self.head,
            &self.revision.to_be_bytes(),
            &self.public,
        ]
        .concat()
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        [
            PREFIX.as_slice(),
            &self.group,
            &self.predecessor,
            &self.head,
            &self.revision.to_be_bytes(),
            &self.public,
            &self.signature,
        ]
        .concat()
    }
    pub fn from_bytes(bytes: &[u8], expected_authority: Id) -> Result<Self, Error> {
        if bytes.len() != 208 || &bytes[..8] != PREFIX {
            return Err(Error::Encoding);
        }
        let id = |n| <Id>::try_from(&bytes[n..n + 32]).map_err(|_| Error::Encoding);
        let receipt = Self {
            group: id(8)?,
            predecessor: id(40)?,
            head: id(72)?,
            revision: u64::from_be_bytes(bytes[104..112].try_into().map_err(|_| Error::Encoding)?),
            public: id(112)?,
            signature: bytes[144..208].try_into().map_err(|_| Error::Encoding)?,
        };
        if receipt.revision == 0
            || receipt.predecessor == receipt.head
            || authority_fingerprint(&receipt.public) != expected_authority
        {
            return Err(Error::Authentication);
        }
        verify_signature(&receipt.public, &receipt.statement(), &receipt.signature)?;
        Ok(receipt)
    }
}
