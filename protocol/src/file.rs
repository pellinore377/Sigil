//! Sensitive file content. Framing validation does not establish sender authority.
use serde::{Deserialize, Serialize};
pub const CHUNK_SIZE: usize = 1024 * 1024;
pub const MAX_FILE_BYTES: u64 = 1024 * 1024 * 1024 * 1024;
pub const CHUNK_OVERHEAD: usize = 84;
pub const DESCRIPTOR_SIZE: usize = 116;
pub const DESCRIPTOR_PREFIX: &[u8; 8] = b"SGAD\0\x01\0\0";
const PREFIX: &[u8; 8] = b"SGFC\0\x01\0\0";
const HEADER: usize = 170;
pub const MAX_CONTENT: usize = HEADER + 253 + 255 + 127;
type Id = [u8; 32];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub name: String,
    pub media_type: String,
}
impl Metadata {
    pub fn validate(&self) -> Result<(), &'static str> {
        metadata(&self.name, &self.media_type)
    }
}
fn metadata(name: &str, media_type: &str) -> Result<(), &'static str> {
    if name.is_empty()
        || name.len() > 255
        || matches!(name, "." | "..")
        || name
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\'))
        || media_type.len() > 127
        || !media_type.split_once('/').is_some_and(|(a, b)| {
            !a.is_empty()
                && !b.is_empty()
                && [a, b].iter().all(|s| {
                    s.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"!#$&^_.+-".contains(&c))
                })
        })
    {
        return Err("invalid file metadata");
    }
    Ok(())
}
/// Borrowed key material; intentionally has no Debug or standalone serialization.
pub struct KeyDescriptor<'a> {
    pub bytes: &'a [u8; DESCRIPTOR_SIZE],
    pub file: Id,
    pub length: u64,
    pub key: &'a Id,
    pub root: &'a Id,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DescriptorError {
    Encoding,
    Limit,
}
impl<'a> KeyDescriptor<'a> {
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Self, DescriptorError> {
        if bytes.len() != DESCRIPTOR_SIZE
            || &bytes[..8] != DESCRIPTOR_PREFIX
            || bytes[48..52] != (CHUNK_SIZE as u32).to_be_bytes()
        {
            return Err(DescriptorError::Encoding);
        }
        let length = u64::from_be_bytes(
            bytes[40..48]
                .try_into()
                .map_err(|_| DescriptorError::Encoding)?,
        );
        if length > MAX_FILE_BYTES {
            return Err(DescriptorError::Limit);
        }
        Ok(Self {
            bytes: bytes.try_into().map_err(|_| DescriptorError::Encoding)?,
            file: bytes[8..40]
                .try_into()
                .map_err(|_| DescriptorError::Encoding)?,
            length,
            key: bytes[52..84]
                .try_into()
                .map_err(|_| DescriptorError::Encoding)?,
            root: bytes[84..116]
                .try_into()
                .map_err(|_| DescriptorError::Encoding)?,
        })
    }
}
/// Canonical ordinary-retention content carried only inside authenticated
/// encrypted events/recovery. MIME and name remain untrusted rendering metadata.
pub struct File<'a> {
    pub source: &'a str,
    pub name: &'a str,
    pub media_type: &'a str,
    pub expires_at: Option<u64>,
    pub access: &'a Id,
    pub descriptor: &'a [u8],
}
impl<'a> File<'a> {
    pub fn validate(&self) -> Result<(), &'static str> {
        metadata(self.name, self.media_type)?;
        if !crate::valid_server_name(self.source)
            || self
                .expires_at
                .is_some_and(|v| v == 0 || v > i64::MAX as u64)
        {
            return Err("invalid file source or expiry");
        }
        KeyDescriptor::from_bytes(self.descriptor).map_err(|_| "invalid file descriptor")?;
        Ok(())
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>, &'static str> {
        self.validate()?;
        let mut bytes = Vec::with_capacity(
            HEADER + self.source.len() + self.name.len() + self.media_type.len(),
        );
        bytes.extend_from_slice(PREFIX);
        for value in [self.source, self.name, self.media_type] {
            bytes.extend_from_slice(&(value.len() as u16).to_be_bytes());
        }
        bytes.extend_from_slice(&self.expires_at.unwrap_or(0).to_be_bytes());
        bytes.extend_from_slice(self.access);
        bytes.extend_from_slice(self.descriptor);
        for value in [self.source, self.name, self.media_type] {
            bytes.extend_from_slice(value.as_bytes());
        }
        Ok(bytes)
    }
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Self, &'static str> {
        if !(HEADER..=MAX_CONTENT).contains(&bytes.len()) || &bytes[..8] != PREFIX {
            return Err("unsupported file content");
        }
        let length = |offset| u16::from_be_bytes([bytes[offset], bytes[offset + 1]]) as usize;
        let (source, name, mime) = (length(8), length(10), length(12));
        if HEADER + source + name + mime != bytes.len() {
            return Err("invalid file lengths");
        }
        let expires = u64::from_be_bytes(
            bytes[14..22]
                .try_into()
                .map_err(|_| "invalid file expiry")?,
        );
        let text = |start, end| {
            std::str::from_utf8(&bytes[start..end]).map_err(|_| "invalid file encoding")
        };
        let value = Self {
            source: text(HEADER, HEADER + source)?,
            name: text(HEADER + source, HEADER + source + name)?,
            media_type: text(HEADER + source + name, bytes.len())?,
            expires_at: (expires != 0).then_some(expires),
            access: bytes[22..54]
                .try_into()
                .map_err(|_| "invalid file access")?,
            descriptor: &bytes[54..HEADER],
        };
        value.validate()?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn descriptor() -> Vec<u8> {
        [
            DESCRIPTOR_PREFIX.as_slice(),
            &[1; 32],
            &5u64.to_be_bytes(),
            &(CHUNK_SIZE as u32).to_be_bytes(),
            &[2; 32],
            &[3; 32],
        ]
        .concat()
    }
    #[test]
    fn sensitive_file_content_is_canonical_bounded_and_strict() {
        let descriptor = descriptor();
        let access = [4; 32];
        let mut file = File {
            source: "chat.example",
            name: "synthetic 🗒.txt",
            media_type: "text/plain",
            expires_at: Some(123),
            access: &access,
            descriptor: &descriptor,
        };
        let bytes = file.to_bytes().unwrap();
        let parsed = File::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.to_bytes().unwrap(), bytes);
        assert_eq!(parsed.name, file.name);
        for length in 0..bytes.len() {
            assert!(File::from_bytes(&bytes[..length]).is_err());
        }
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(File::from_bytes(&extra).is_err());
        for offset in 0..14 {
            let mut bad = bytes.clone();
            bad[offset] ^= 128;
            assert!(File::from_bytes(&bad).is_err());
        }
        for source in ["https://chat.example", "Chat.example", "a/b", "a:443"] {
            file.source = source;
            assert!(file.to_bytes().is_err());
        }
        file.source = "chat.example";
        for name in ["", "..", "a/b", "a\\b", "a\0b"] {
            file.name = name;
            assert!(file.to_bytes().is_err());
        }
        let max = "a".repeat(255);
        file.name = &max;
        file.expires_at = None;
        assert!(file.to_bytes().is_ok());
        let too_long = "a".repeat(256);
        file.name = &too_long;
        assert!(file.to_bytes().is_err());
        file.name = "synthetic";
        for mime in [
            "text",
            "/plain",
            "text/",
            "text/plain; charset=utf8",
            "text/a/b",
        ] {
            file.media_type = mime;
            assert!(file.to_bytes().is_err());
        }
    }
    #[test]
    fn descriptor_rejects_unknown_profiles_bad_shape_and_extra_bytes() {
        let bytes = descriptor();
        let parsed = KeyDescriptor::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.file, [1; 32]);
        assert_eq!(parsed.length, 5);
        assert_eq!(parsed.key, &[2; 32]);
        assert_eq!(parsed.root, &[3; 32]);
        for offset in (0..8).chain(48..52) {
            let mut bad = bytes.clone();
            bad[offset] ^= 128;
            assert!(KeyDescriptor::from_bytes(&bad).is_err());
        }
        let mut bad = bytes.clone();
        bad[40..48].copy_from_slice(&(MAX_FILE_BYTES + 1).to_be_bytes());
        assert!(KeyDescriptor::from_bytes(&bad).is_err());
        for length in 0..bytes.len() {
            assert!(KeyDescriptor::from_bytes(&bytes[..length]).is_err());
        }
        let mut bad = bytes;
        bad.push(0);
        assert!(KeyDescriptor::from_bytes(&bad).is_err());
    }
}
