//! Fixed RFC 9421/9530 request profile; TLS and durable replay admission remain
//! mandatory. Verification establishes a homeserver, never an E2EE participant.
use axum::http::{header, HeaderMap, HeaderValue};
use base64ct::{Base64, Encoding};
use ring::signature::{self, KeyPair};
use sha2::{Digest, Sha256};
use sigil_protocol::federation::{Discovery, Key};
use zeroize::Zeroizing;
const COMPONENTS:&str="(\"@method\" \"@path\" \"content-type\" \"content-digest\" \"sigil-origin\" \"sigil-destination\")";
const TAG: &str = "sigil-federation-v0";
pub const LIFETIME: u64 = 120;
pub const SKEW: u64 = 30;
const MAX_TIME: u64 = 999_999_999_999_999;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Expired,
    Signature,
    Key,
}
pub fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|byte| {
            [
                HEX[(byte >> 4) as usize] as char,
                HEX[(byte & 15) as usize] as char,
            ]
        })
        .collect()
}
pub fn bytes32(value: &str) -> Result<[u8; 32], Error> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::Invalid);
    }
    let mut bytes = [0; 32];
    for (i, out) in bytes.iter_mut().enumerate() {
        *out = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16).map_err(|_| Error::Invalid)?;
    }
    Ok(bytes)
}
pub fn key_id(public: &[u8]) -> String {
    hex(&Sha256::digest(
        [b"Sigil/federation-server-key/v0".as_slice(), public].concat(),
    ))
}
pub fn validate_key(key: &Key) -> Result<[u8; 32], Error> {
    let public = bytes32(&key.public_key)?;
    if key.id != key_id(&public)
        || key.generation == 0
        || key.generation > i64::MAX as u64
        || key.not_before == 0
        || key.not_before > MAX_TIME
    {
        return Err(Error::Key);
    }
    Ok(public)
}
pub struct SigningKey {
    seed: Zeroizing<Vec<u8>>,
    pair: signature::Ed25519KeyPair,
    descriptor: Key,
}
impl SigningKey {
    pub fn descriptor(&self) -> &Key {
        &self.descriptor
    }
    pub fn generate(generation: u64, now: u64) -> Result<Self, Error> {
        let mut seed = Zeroizing::new(vec![0; 32]);
        getrandom::fill(&mut seed).map_err(|_| Error::Key)?;
        Self::from_seed(seed, generation, now)
    }
    pub fn from_seed(seed: Zeroizing<Vec<u8>>, generation: u64, now: u64) -> Result<Self, Error> {
        let pair = signature::Ed25519KeyPair::from_seed_unchecked(&seed).map_err(|_| Error::Key)?;
        let descriptor = Key {
            id: key_id(pair.public_key().as_ref()),
            public_key: hex(pair.public_key().as_ref()),
            generation,
            not_before: now,
        };
        validate_key(&descriptor)?;
        Ok(Self {
            seed,
            pair,
            descriptor,
        })
    }
    pub(crate) fn seed(&self) -> &[u8] {
        &self.seed
    }
    pub fn transition(&self, server: &str, next: &Key) -> Result<String, Error> {
        Ok(Base64::encode_string(
            self.pair
                .sign(&transition_base(server, &self.descriptor, next)?)
                .as_ref(),
        ))
    }
}
fn transition_base(server: &str, previous: &Key, next: &Key) -> Result<Vec<u8>, Error> {
    validate_key(previous)?;
    validate_key(next)?;
    if !sigil_protocol::valid_server_name(server)
        || previous.generation.checked_add(1) != Some(next.generation)
        || next.not_before <= previous.not_before
        || previous.id == next.id
    {
        return Err(Error::Key);
    }
    let mut bytes = b"Sigil/federation-key-transition/v0".to_vec();
    bytes.extend_from_slice(&(server.len() as u16).to_be_bytes());
    bytes.extend_from_slice(server.as_bytes());
    for key in [previous, next] {
        bytes.extend_from_slice(&bytes32(&key.public_key)?);
        bytes.extend_from_slice(&key.generation.to_be_bytes());
        bytes.extend_from_slice(&key.not_before.to_be_bytes());
    }
    Ok(bytes)
}
pub fn validate_discovery(value: &Discovery, server: &str, now: u64) -> Result<(), Error> {
    if value.version != 0
        || value.server != server
        || !sigil_protocol::valid_server_name(server)
        || value.current.not_before > now.saturating_add(SKEW)
    {
        return Err(Error::Key);
    }
    validate_key(&value.current)?;
    match &value.rotation {
        Some(rotation) => {
            let public = validate_key(&rotation.previous)?;
            let base = transition_base(server, &rotation.previous, &value.current)?;
            signature::UnparsedPublicKey::new(&signature::ED25519, public)
                .verify(&base, &signature_bytes(&rotation.signature)?)
                .map_err(|_| Error::Signature)?;
        }
        None if value.current.generation == 1 => {}
        None => return Err(Error::Key),
    }
    Ok(())
}
pub fn follows(pinned: &Key, value: &Discovery, server: &str, now: u64) -> Result<(), Error> {
    validate_discovery(value, server, now)?;
    if &value.current == pinned {
        return Ok(());
    }
    if value
        .rotation
        .as_ref()
        .is_some_and(|rotation| &rotation.previous == pinned)
    {
        return Ok(());
    }
    Err(Error::Key)
}
#[derive(Clone)]
pub struct Metadata {
    pub created: u64,
    pub expires: u64,
    pub nonce: String,
    pub key_id: String,
}
pub struct Verified {
    metadata: Metadata,
    origin: String,
    destination: String,
}
impl Verified {
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }
    pub fn origin(&self) -> &str {
        &self.origin
    }
    pub fn destination(&self) -> &str {
        &self.destination
    }
}
impl Metadata {
    fn validate(&self) -> Result<(), Error> {
        bytes32(&self.nonce)?;
        bytes32(&self.key_id)?;
        if self.created == 0
            || self.created > MAX_TIME
            || self.expires > MAX_TIME
            || self.expires <= self.created
            || self.expires - self.created > LIFETIME
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    fn parameters(&self) -> Result<String, Error> {
        self.validate()?;
        Ok(format!("{COMPONENTS};created={};expires={};nonce=\"{}\";keyid=\"{}\";alg=\"ed25519\";tag=\"{TAG}\"",self.created,self.expires,self.nonce,self.key_id))
    }
    fn parse(value: &str) -> Result<Self, Error> {
        if value.len() > 1024 {
            return Err(Error::Invalid);
        }
        let tail = value
            .strip_prefix("sigil=")
            .and_then(|s| s.strip_prefix(COMPONENTS))
            .and_then(|s| s.strip_prefix(';'))
            .ok_or(Error::Invalid)?;
        let parts: Vec<&str> = tail.split(';').collect();
        if parts.len() != 6 {
            return Err(Error::Invalid);
        }
        let integer = |part: &str, prefix: &str| -> Result<u64, Error> {
            let text = part.strip_prefix(prefix).ok_or(Error::Invalid)?;
            let value = text.parse::<u64>().map_err(|_| Error::Invalid)?;
            if value.to_string() != text {
                return Err(Error::Invalid);
            }
            Ok(value)
        };
        let quoted = |part: &str, prefix: &str| -> Result<String, Error> {
            Ok(part
                .strip_prefix(prefix)
                .and_then(|s| s.strip_suffix('"'))
                .ok_or(Error::Invalid)?
                .to_owned())
        };
        let meta = Self {
            created: integer(parts[0], "created=")?,
            expires: integer(parts[1], "expires=")?,
            nonce: quoted(parts[2], "nonce=\"")?,
            key_id: quoted(parts[3], "keyid=\"")?,
        };
        if value != format!("sigil={}", meta.parameters()?) {
            return Err(Error::Invalid);
        }
        Ok(meta)
    }
}
pub struct Request<'a> {
    pub origin: &'a str,
    pub destination: &'a str,
    pub path: &'a str,
    pub body: &'a [u8],
}
impl Request<'_> {
    fn validate(&self) -> Result<(), Error> {
        if !sigil_protocol::valid_server_name(self.origin)
            || !sigil_protocol::valid_server_name(self.destination)
            || self.origin == self.destination
            || self.path.len() > 100
            || !self.path.starts_with("/federation/v0/")
            || !self
                .path
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'/' | b'-'))
            || self.body.len()
                > if self.path == sigil_protocol::federation::LOOKUP_PATH {
                    sigil_protocol::federation::MAX_LOOKUP_BODY
                } else {
                    sigil_protocol::federation::MAX_BODY
                }
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub fn digest(&self) -> String {
        format!(
            "sha-256=:{}:",
            Base64::encode_string(&Sha256::digest(self.body))
        )
    }
    fn base(&self, metadata: &Metadata) -> Result<String, Error> {
        self.validate()?;
        Ok(format!("\"@method\": POST\n\"@path\": {}\n\"content-type\": application/json\n\"content-digest\": {}\n\"sigil-origin\": {}\n\"sigil-destination\": {}\n\"@signature-params\": {}",self.path,self.digest(),self.origin,self.destination,metadata.parameters()?))
    }
}
fn field<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, Error> {
    let values = headers.get_all(name);
    if values.iter().count() != 1 {
        return Err(Error::Invalid);
    }
    values
        .iter()
        .next()
        .ok_or(Error::Invalid)?
        .to_str()
        .map_err(|_| Error::Invalid)
}
pub fn inspect(headers: &HeaderMap) -> Result<(&str, &str, Metadata), Error> {
    let origin = field(headers, "sigil-origin")?;
    let destination = field(headers, "sigil-destination")?;
    if !sigil_protocol::valid_server_name(origin) || !sigil_protocol::valid_server_name(destination)
    {
        return Err(Error::Invalid);
    }
    Ok((
        origin,
        destination,
        Metadata::parse(field(headers, "signature-input")?)?,
    ))
}
fn signature_bytes(value: &str) -> Result<Vec<u8>, Error> {
    if value.len() != 88 {
        return Err(Error::Signature);
    }
    let bytes = Base64::decode_vec(value).map_err(|_| Error::Signature)?;
    if bytes.len() != 64 || Base64::encode_string(&bytes) != value {
        return Err(Error::Signature);
    }
    Ok(bytes)
}
pub fn sign(
    key: &SigningKey,
    request: &Request<'_>,
    now: u64,
    nonce: [u8; 32],
) -> Result<HeaderMap, Error> {
    if now < key.descriptor.not_before {
        return Err(Error::Expired);
    }
    let metadata = Metadata {
        created: now,
        expires: now.checked_add(LIFETIME).ok_or(Error::Expired)?,
        nonce: hex(&nonce),
        key_id: key.descriptor.id.clone(),
    };
    let signature = key.pair.sign(request.base(&metadata)?.as_bytes());
    let mut headers = HeaderMap::new();
    for (name, value) in [
        ("content-type", "application/json".to_owned()),
        ("content-digest", request.digest()),
        ("sigil-origin", request.origin.to_owned()),
        ("sigil-destination", request.destination.to_owned()),
        (
            "signature-input",
            format!("sigil={}", metadata.parameters()?),
        ),
        (
            "signature",
            format!("sigil=:{}:", Base64::encode_string(signature.as_ref())),
        ),
    ] {
        headers.insert(
            axum::http::HeaderName::from_bytes(name.as_bytes()).map_err(|_| Error::Invalid)?,
            HeaderValue::from_str(&value).map_err(|_| Error::Invalid)?,
        );
    }
    Ok(headers)
}
/// Caller must enforce the actual HTTP method, exact target path and configured
/// destination, and consume this nonce atomically with the admitted operation.
pub fn verify(
    key: &Key,
    request: &Request<'_>,
    headers: &HeaderMap,
    now: u64,
) -> Result<Verified, Error> {
    let (origin, destination, metadata) = inspect(headers)?;
    if origin != request.origin
        || destination != request.destination
        || field(headers, "content-type")? != "application/json"
        || field(headers, "content-digest")? != request.digest()
        || headers.contains_key(header::CONTENT_ENCODING)
    {
        return Err(Error::Invalid);
    }
    if metadata.created > now.saturating_add(SKEW)
        || metadata.expires <= now
        || metadata.created < key.not_before
    {
        return Err(Error::Expired);
    }
    let public = validate_key(key)?;
    if metadata.key_id != key.id {
        return Err(Error::Key);
    }
    let encoded = field(headers, "signature")?
        .strip_prefix("sigil=:")
        .and_then(|s| s.strip_suffix(':'))
        .ok_or(Error::Signature)?;
    signature::UnparsedPublicKey::new(&signature::ED25519, public)
        .verify(
            request.base(&metadata)?.as_bytes(),
            &signature_bytes(encoded)?,
        )
        .map_err(|_| Error::Signature)?;
    Ok(Verified {
        metadata,
        origin: origin.into(),
        destination: destination.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key() -> SigningKey {
        SigningKey::from_seed(Zeroizing::new(vec![1; 32]), 1, 1000).unwrap()
    }
    #[test]
    fn fixed_profile_binds_target_body_servers_parameters_and_time() {
        let key = key();
        let request = Request {
            origin: "a.example",
            destination: "b.example",
            path: "/federation/v0/ping",
            body: b"{}",
        };
        let signed = sign(&key, &request, 1000, [2; 32]).unwrap();
        let verified = verify(key.descriptor(), &request, &signed, 1000).unwrap();
        assert_eq!(verified.origin(), "a.example");
        assert_eq!(verified.destination(), "b.example");
        assert_eq!(verified.metadata().expires, 1120);
        for (name, value) in [
            ("sigil-origin", "c.example"),
            ("sigil-destination", "c.example"),
            ("content-type", "text/plain"),
            ("content-digest", "sha-256=:AAAA:"),
        ] {
            let mut changed = signed.clone();
            changed.insert(name, HeaderValue::from_static(value));
            assert!(verify(key.descriptor(), &request, &changed, 1000).is_err());
        }
        let mut duplicate = signed.clone();
        duplicate.append("signature-input", signed["signature-input"].clone());
        assert!(verify(key.descriptor(), &request, &duplicate, 1000).is_err());
        let mut compressed = signed.clone();
        compressed.insert("content-encoding", HeaderValue::from_static("gzip"));
        assert!(verify(key.descriptor(), &request, &compressed, 1000).is_err());
        for changed in [
            Request {
                body: b"[]",
                ..request
            },
            Request {
                path: "/federation/v0/deliver",
                ..request
            },
            Request {
                path: "/federation/v0/ping?x=y",
                ..request
            },
        ] {
            assert!(verify(key.descriptor(), &changed, &signed, 1000).is_err());
        }
        assert!(verify(key.descriptor(), &request, &signed, 1120).is_err());
        assert!(verify(key.descriptor(), &request, &signed, 969).is_err());
        assert!(verify(key.descriptor(), &request, &signed, 970).is_ok());
        let wrong = SigningKey::from_seed(Zeroizing::new(vec![3; 32]), 1, 1000).unwrap();
        assert!(verify(wrong.descriptor(), &request, &signed, 1000).is_err());
        let original = signed["signature-input"].to_str().unwrap();
        for value in [
            original.replace("alg=\"ed25519\"", "alg=\"rsa-v1_5-sha256\""),
            original.replace("created=1000", "created=01000"),
            original.replace("expires=1120", "expires=9999"),
            original.replace("sigil-federation-v0", "sigil-federation-v1"),
        ] {
            let mut changed = signed.clone();
            changed.insert("signature-input", HeaderValue::from_str(&value).unwrap());
            assert!(verify(key.descriptor(), &request, &changed, 1000).is_err());
        }
    }
    #[test]
    fn key_transitions_preserve_pins_and_reject_rollback_or_unproven_replacement() {
        let first = key();
        let second = SigningKey::generate(2, 1100).unwrap();
        let value = Discovery {
            version: 0,
            server: "a.example".into(),
            current: second.descriptor().clone(),
            rotation: Some(sigil_protocol::federation::Rotation {
                previous: first.descriptor().clone(),
                signature: first.transition("a.example", second.descriptor()).unwrap(),
            }),
        };
        follows(first.descriptor(), &value, "a.example", 1100).unwrap();
        follows(second.descriptor(), &value, "a.example", 1100).unwrap();
        assert!(follows(first.descriptor(), &value, "b.example", 1100).is_err());
        let mut corrupt = value.clone();
        corrupt.current.not_before += 1;
        assert!(follows(first.descriptor(), &corrupt, "a.example", 1101).is_err());
        let unrelated = SigningKey::generate(1, 1100).unwrap();
        assert!(follows(unrelated.descriptor(), &value, "a.example", 1100).is_err());
        let old = Discovery {
            version: 0,
            server: "a.example".into(),
            current: first.descriptor().clone(),
            rotation: None,
        };
        assert!(follows(second.descriptor(), &old, "a.example", 1100).is_err());
        assert!(first.transition("a.example", first.descriptor()).is_err());
    }
    #[test]
    fn published_rfc_9421_ed25519_vector_matches_without_a_prehash() {
        // RFC 9421 B.1.4 and B.2.6: published synthetic interoperability data.
        let seed =
            base64ct::Base64UrlUnpadded::decode_vec("n4Ni-HpISpVObnQMW0wOhCKROaIKqKtW_2ZYb2p9KcU")
                .unwrap();
        let key = SigningKey::from_seed(Zeroizing::new(seed), 1, 1).unwrap();
        let expected_public =
            base64ct::Base64UrlUnpadded::decode_vec("JrQLj5P_89iXES9-vFgrIy29clF9CC_oPPsw3c5D0bs")
                .unwrap();
        assert_eq!(key.pair.public_key().as_ref(), expected_public);
        let base="\"date\": Tue, 20 Apr 2021 02:07:55 GMT\n\"@method\": POST\n\"@path\": /foo\n\"@authority\": example.com\n\"content-type\": application/json\n\"content-length\": 18\n\"@signature-params\": (\"date\" \"@method\" \"@path\" \"@authority\" \"content-type\" \"content-length\");created=1618884473;keyid=\"test-key-ed25519\"";
        let expected="wqcAqbmYJ2ji2glfAMaRy4gruYYnx2nEFN2HN6jrnDnQCK1u02Gb04v9EDgwUPiu4A0w6vuQv5lIp5WPpBKRCw==";
        assert_eq!(
            Base64::encode_string(key.pair.sign(base.as_bytes()).as_ref()),
            expected
        );
        signature::UnparsedPublicKey::new(&signature::ED25519, expected_public)
            .verify(base.as_bytes(), &signature_bytes(expected).unwrap())
            .unwrap();
    }
}
