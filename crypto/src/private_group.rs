//! Identity and request binding for the private-group credential service.
use crate::{
    private_credentials::{Attributes, IssuerPublic},
    verify_signature, Error, IdentityKey,
};
use sha2::{Digest, Sha256};
use sigil_protocol::device::SignedBinding;

const PREFIX: &[u8; 8] = b"SGPA\0\0\0\0";

pub fn authority_fingerprint(public: &[u8; 32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"Sigil/group-authority/v0");
    hash.update(32u64.to_be_bytes());
    hash.update(public);
    hash.finalize().into()
}

/// The signing identity is pinned by the authenticated group genesis.
pub struct Authority {
    server: String,
    generation: u64,
    issuer: IssuerPublic,
    public: [u8; 32],
    signature: [u8; 64],
}

impl Authority {
    pub fn sign(
        server: &str,
        generation: u64,
        issuer: IssuerPublic,
        key: &IdentityKey,
    ) -> Result<Self, Error> {
        if !sigil_protocol::valid_server_name(server) || generation == 0 {
            return Err(Error::Encoding);
        }
        let mut value = Self {
            server: server.into(),
            generation,
            issuer,
            public: key.public_key(),
            signature: [0; 64],
        };
        value.signature = key.sign(&value.statement())?;
        Ok(value)
    }

    fn statement(&self) -> Vec<u8> {
        [
            b"Sigil/private-group-authority/v0".as_slice(),
            &(self.server.len() as u16).to_be_bytes(),
            self.server.as_bytes(),
            &self.generation.to_be_bytes(),
            &self.issuer.to_bytes(),
            &self.public,
        ]
        .concat()
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        [
            PREFIX.as_slice(),
            &(self.server.len() as u16).to_be_bytes(),
            self.server.as_bytes(),
            &self.generation.to_be_bytes(),
            &self.issuer.to_bytes(),
            &self.public,
            &self.signature,
        ]
        .concat()
    }

    pub fn from_bytes(
        bytes: &[u8],
        expected_server: &str,
        expected_authority: [u8; 32],
    ) -> Result<Self, Error> {
        if !(179..=431).contains(&bytes.len()) || &bytes[..8] != PREFIX {
            return Err(Error::Encoding);
        }
        let n = u16::from_be_bytes(bytes[8..10].try_into().map_err(|_| Error::Encoding)?) as usize;
        if bytes.len() != 178 + n {
            return Err(Error::Encoding);
        }
        let server = std::str::from_utf8(&bytes[10..10 + n]).map_err(|_| Error::Encoding)?;
        if server != expected_server || !sigil_protocol::valid_server_name(server) {
            return Err(Error::Authentication);
        }
        let fields = &bytes[10 + n..];
        let value = Self {
            server: server.into(),
            generation: u64::from_be_bytes(fields[..8].try_into().map_err(|_| Error::Encoding)?),
            issuer: IssuerPublic::from_bytes(&fields[8..72])?,
            public: fields[72..104].try_into().map_err(|_| Error::Encoding)?,
            signature: fields[104..168].try_into().map_err(|_| Error::Encoding)?,
        };
        if value.generation == 0 || authority_fingerprint(&value.public) != expected_authority {
            return Err(Error::Authentication);
        }
        verify_signature(&value.public, &value.statement(), &value.signature)?;
        Ok(value)
    }

    pub fn server(&self) -> &str {
        &self.server
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn fingerprint(&self) -> [u8; 32] {
        authority_fingerprint(&self.public)
    }
    pub fn issuer(&self) -> &IssuerPublic {
        &self.issuer
    }

    /// Randomized signatures do not change the profile's stable identifier.
    pub fn id(&self) -> [u8; 32] {
        Sha256::digest(self.statement()).into()
    }

    /// Identity possession is checked here; account admission belongs to transport.
    pub fn issuance(&self, binding: &[u8], day: u32) -> Result<Issuance, Error> {
        let signed = SignedBinding::from_bytes(binding).map_err(|_| Error::Encoding)?;
        let statement = signed
            .binding
            .signing_bytes()
            .map_err(|_| Error::Encoding)?;
        verify_signature(&signed.binding.identity, &statement, &signed.signature)?;
        let fingerprint: [u8; 32] = Sha256::digest(&statement).into();
        let mut hash = Sha256::new();
        hash.update(b"Sigil/private-group-uid/v0");
        hash.update(authority_fingerprint(&self.public));
        hash.update(fingerprint);
        let uid: [u8; 16] = hash.finalize()[..16].try_into().map_err(|_| Error::State)?;
        let context = [
            b"Sigil/private-group-issuance/v0".as_slice(),
            &self.id(),
            &fingerprint,
            &uid,
            &day.to_be_bytes(),
        ]
        .concat();
        Ok(Issuance {
            uid,
            fingerprint,
            day,
            context,
        })
    }
}

/// Issuers must retain collision checks for UID-to-full-fingerprint assignments.
pub struct Issuance {
    pub uid: [u8; 16],
    pub fingerprint: [u8; 32],
    pub day: u32,
    context: Vec<u8>,
}

impl Issuance {
    pub fn attributes(&self) -> Result<Attributes, Error> {
        Attributes::for_uid(&self.uid)
    }
    pub fn context(&self) -> &[u8] {
        &self.context
    }
}

#[derive(Clone, Copy)]
pub enum Operation {
    Create = 1,
    Read = 2,
    Advance = 3,
    Join = 4,
    History = 5,
}

/// Hash the complete canonical operation body before preparing its proof.
#[derive(Clone)]
pub struct Request {
    pub operation: Operation,
    pub group: [u8; 32],
    pub predecessor: [u8; 32],
    pub body_hash: [u8; 32],
    pub nonce: [u8; 32],
    pub expires_at: u64,
}

impl Request {
    pub fn context(&self, authority: &Authority, now: u64) -> Result<Vec<u8>, Error> {
        if self.nonce == [0; 32]
            || self.group == [0; 32]
            || now == 0
            || self.expires_at <= now
            || self.expires_at > now.saturating_add(120)
            || self.expires_at > i64::MAX as u64
        {
            return Err(Error::Encoding);
        }
        Ok([
            b"Sigil/private-group-request/v0".as_slice(),
            &authority.id(),
            &[self.operation as u8],
            &self.group,
            &self.predecessor,
            &self.body_hash,
            &self.nonce,
            &self.expires_at.to_be_bytes(),
        ]
        .concat())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        private_credentials::{Credential, GroupKey, Issuer},
        Secret32,
    };

    fn binding(key: &IdentityKey) -> Vec<u8> {
        let binding = sigil_protocol::device::Binding {
            server: "origin.example".into(),
            username: "alice".into(),
            account: [1; 32],
            device: [2; 32],
            identity: key.public_key(),
        };
        let signature = key.sign(&binding.signing_bytes().unwrap()).unwrap();
        SignedBinding { binding, signature }.to_bytes().unwrap()
    }

    #[test]
    fn authority_and_issuance_bind_full_identities_without_granting_account_trust() {
        let key = IdentityKey::generate().unwrap();
        let issuer = Issuer::generate().unwrap();
        let authority = Authority::sign("authority.example", 1, issuer.public(), &key).unwrap();
        let bytes = authority.to_bytes();
        let pin = authority_fingerprint(&key.public_key());
        let parsed = Authority::from_bytes(&bytes, "authority.example", pin).unwrap();
        assert_eq!(authority.id(), parsed.id());
        assert_eq!(
            authority.id(),
            Authority::sign("authority.example", 1, issuer.public(), &key)
                .unwrap()
                .id()
        );
        assert!(Authority::from_bytes(&bytes, "other.example", pin).is_err());
        assert!(Authority::from_bytes(&bytes, "authority.example", [0; 32]).is_err());
        for i in 0..bytes.len() {
            let mut changed = bytes.clone();
            changed[i] ^= 1;
            assert!(Authority::from_bytes(&changed, "authority.example", pin).is_err());
            assert!(Authority::from_bytes(&bytes[..i], "authority.example", pin).is_err());
        }
        let device = IdentityKey::generate().unwrap();
        let signed = binding(&device);
        let issuance = authority.issuance(&signed, 20_000).unwrap();
        let response = issuer
            .issue(
                &issuance.attributes().unwrap(),
                issuance.day,
                issuance.context(),
            )
            .unwrap();
        Credential::accept(
            parsed.issuer(),
            issuance.attributes().unwrap(),
            issuance.day,
            issuance.context(),
            &response,
        )
        .unwrap();
        let retry = authority.issuance(&binding(&device), 20_000).unwrap();
        assert_eq!(retry.uid, issuance.uid);
        assert_eq!(retry.context(), issuance.context());
        let mut forged = signed.clone();
        forged[20] ^= 1;
        assert!(authority.issuance(&forged, 20_000).is_err());
        let other = Authority::sign(
            "authority.example",
            2,
            Issuer::generate().unwrap().public(),
            &key,
        )
        .unwrap()
        .issuance(&signed, 20_000)
        .unwrap();
        assert_eq!(other.uid, issuance.uid);
        assert_ne!(other.context(), issuance.context());
    }

    #[test]
    fn anonymous_request_binds_operation_body_head_nonce_deadline_and_authority_generation() {
        let issuer = Issuer::generate().unwrap();
        let signing = IdentityKey::generate().unwrap();
        let authority = Authority::sign("authority.example", 1, issuer.public(), &signing).unwrap();
        let issuance = authority
            .issuance(&binding(&IdentityKey::generate().unwrap()), 20_000)
            .unwrap();
        let response = issuer
            .issue(
                &issuance.attributes().unwrap(),
                issuance.day,
                issuance.context(),
            )
            .unwrap();
        let credential = Credential::accept(
            authority.issuer(),
            issuance.attributes().unwrap(),
            issuance.day,
            issuance.context(),
            &response,
        )
        .unwrap();
        let group = GroupKey::from_master(Secret32::from_bytes([8; 32])).unwrap();
        let mut request = Request {
            operation: Operation::Read,
            group: [1; 32],
            predecessor: [2; 32],
            body_hash: [3; 32],
            nonce: [4; 32],
            expires_at: 1010,
        };
        let context = request.context(&authority, 1000).unwrap();
        let proof = credential.present(&group, &context).unwrap();
        issuer
            .verify_presentation(group.public(), 20_000, &context, &proof)
            .unwrap();
        request.operation = Operation::Advance;
        assert!(issuer
            .verify_presentation(
                group.public(),
                20_000,
                &request.context(&authority, 1000).unwrap(),
                &proof
            )
            .is_err());
        request.operation = Operation::Read;
        for i in 0..5 {
            let mut changed = request.clone();
            match i {
                0 => changed.group[0] ^= 1,
                1 => changed.predecessor[0] ^= 1,
                2 => changed.body_hash[0] ^= 1,
                3 => changed.nonce[0] ^= 1,
                _ => changed.expires_at += 1,
            }
            assert!(issuer
                .verify_presentation(
                    group.public(),
                    20_000,
                    &changed.context(&authority, 1000).unwrap(),
                    &proof
                )
                .is_err());
        }
        let next = Authority::sign("authority.example", 2, issuer.public(), &signing).unwrap();
        assert_ne!(
            request.context(&authority, 1000).unwrap(),
            request.context(&next, 1000).unwrap()
        );
        assert!(request.context(&authority, 1010).is_err());
        assert!(request.context(&authority, 889).is_err());
        request.nonce = [0; 32];
        assert!(request.context(&authority, 1000).is_err());
    }
}
