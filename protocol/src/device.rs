//! Canonical public device statement. Parsing is not signature verification.
use serde::{Deserialize, Serialize};
const DOMAIN: &[u8] = b"Sigil/device-binding/v0\0";
pub const MAX_BYTES: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub server: String,
    pub username: String,
    pub account: [u8; 32],
    pub device: [u8; 32],
    pub identity: [u8; 32],
}
impl Binding {
    /// Sign these exact domain-separated bytes. Fingerprints hash these bytes,
    /// excluding randomized signatures so the same binding has a stable identity.
    pub fn signing_bytes(&self) -> Result<Vec<u8>, &'static str> {
        if !crate::valid_server_name(&self.server)
            || !crate::accounts::valid_username(&self.username)
        {
            return Err("invalid device address");
        }
        let mut bytes = Vec::with_capacity(MAX_BYTES);
        bytes.extend_from_slice(DOMAIN);
        bytes.extend_from_slice(&(self.server.len() as u16).to_be_bytes());
        bytes.extend_from_slice(self.server.as_bytes());
        bytes.push(self.username.len() as u8);
        bytes.extend_from_slice(self.username.as_bytes());
        bytes.extend_from_slice(&self.account);
        bytes.extend_from_slice(&self.device);
        bytes.extend_from_slice(&self.identity);
        Ok(bytes)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedBinding {
    pub binding: Binding,
    pub signature: [u8; 64],
}
impl SignedBinding {
    pub fn to_bytes(&self) -> Result<Vec<u8>, &'static str> {
        let mut bytes = self.binding.signing_bytes()?;
        bytes.extend_from_slice(&self.signature);
        Ok(bytes)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() > MAX_BYTES {
            return Err("oversized device statement");
        }
        fn take<'a>(bytes: &mut &'a [u8], n: usize) -> Result<&'a [u8], &'static str> {
            let (value, rest) = bytes
                .split_at_checked(n)
                .ok_or("truncated device statement")?;
            *bytes = rest;
            Ok(value)
        }
        fn array<const N: usize>(bytes: &mut &[u8]) -> Result<[u8; N], &'static str> {
            take(bytes, N)?
                .try_into()
                .map_err(|_| "invalid device field")
        }
        let mut rest = bytes;
        if take(&mut rest, DOMAIN.len())? != DOMAIN {
            return Err("unsupported device statement");
        }
        let length = u16::from_be_bytes(array(&mut rest)?) as usize;
        let server = std::str::from_utf8(take(&mut rest, length)?)
            .map_err(|_| "invalid server")?
            .to_owned();
        let [length] = array(&mut rest)?;
        let username = std::str::from_utf8(take(&mut rest, length as usize)?)
            .map_err(|_| "invalid username")?
            .to_owned();
        let binding = Binding {
            server,
            username,
            account: array(&mut rest)?,
            device: array(&mut rest)?,
            identity: array(&mut rest)?,
        };
        let signature = array(&mut rest)?;
        if !rest.is_empty() {
            return Err("trailing device data");
        }
        binding.signing_bytes()?;
        Ok(Self { binding, signature })
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Statement {
    /// Lowercase hex of a canonical SignedBinding. Contains only public data.
    pub statement: String,
}
impl Statement {
    pub fn bytes(&self) -> Result<Vec<u8>, &'static str> {
        if self.statement.len() > MAX_BYTES * 2
            || !self.statement.len().is_multiple_of(2)
            || !self
                .statement
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("invalid device encoding");
        }
        self.statement
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                u8::from_str_radix(std::str::from_utf8(pair).map_err(|_| "invalid hex")?, 16)
                    .map_err(|_| "invalid hex")
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_statement_has_strict_lengths_and_names() {
        let mut value = SignedBinding {
            binding: Binding {
                server: "chat.example".into(),
                username: "synthetic".into(),
                account: [1; 32],
                device: [2; 32],
                identity: [3; 32],
            },
            signature: [4; 64],
        };
        let bytes = value.to_bytes().unwrap();
        assert_eq!(SignedBinding::from_bytes(&bytes).unwrap(), value);
        for n in 0..bytes.len() {
            assert!(SignedBinding::from_bytes(&bytes[..n]).is_err());
        }
        assert!(SignedBinding::from_bytes(&[bytes.as_slice(), &[0]].concat()).is_err());
        for name in [
            "https://chat.example",
            "CHAT.example",
            "localhost",
            "127.0.0.1",
        ] {
            value.binding.server = name.into();
            assert!(value.to_bytes().is_err());
        }
        value.binding.server = "chat.example".into();
        value.binding.username = "Alice".into();
        assert!(value.to_bytes().is_err());
        for text in ["g0", "FF", "a"] {
            assert!(Statement {
                statement: text.into()
            }
            .bytes()
            .is_err());
        }
    }
}
