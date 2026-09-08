//! Canonical application frames inside authenticated direct/group encryption.
const PREFIX: &[u8; 8] = b"SGEV\0\x01\0\0";
const HEADER: usize = 150;
pub const MAX_TEXT: usize = crate::initial::MAX_PLAINTEXT - HEADER;
type Id = [u8; 32];
#[path = "event_content.rs"]
mod content;
pub use content::{Content, Direct, Group};

// Plaintext content deliberately has no Debug implementation.
#[derive(PartialEq, Eq)]
pub struct Text<'a> {
    pub message: Id,
    pub conversation: Id,
    pub sender: Id,
    pub recipient: Id,
    pub timestamp: u64,
    pub body: &'a str,
}
impl<'a> Text<'a> {
    pub fn to_bytes(&self) -> Result<Vec<u8>, &'static str> {
        Direct {
            message: self.message,
            conversation: self.conversation,
            sender: self.sender,
            recipient: self.recipient,
            timestamp: self.timestamp,
            content: Content::Text(self.body),
        }
        .to_bytes()
    }
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Self, &'static str> {
        let event = Direct::from_bytes(bytes)?;
        Ok(Self {
            message: event.message,
            conversation: event.conversation,
            sender: event.sender,
            recipient: event.recipient,
            timestamp: event.timestamp,
            body: event.content.text()?,
        })
    }
}

/// Retained group text; membership and sender-chain context are authenticated
/// by the enclosing group packet. No recipient-specific content is encoded.
#[derive(PartialEq, Eq)]
pub struct GroupText<'a> {
    pub message: Id,
    pub group: Id,
    pub sender: Id,
    pub timestamp: u64,
    pub body: &'a str,
}
impl<'a> GroupText<'a> {
    pub const MAX_BODY: usize = crate::initial::MAX_PLAINTEXT - 118;
    pub fn to_bytes(&self) -> Result<Vec<u8>, &'static str> {
        Group {
            message: self.message,
            group: self.group,
            sender: self.sender,
            timestamp: self.timestamp,
            content: Content::Text(self.body),
        }
        .to_bytes()
    }
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Self, &'static str> {
        let event = Group::from_bytes(bytes)?;
        Ok(Self {
            message: event.message,
            group: event.group,
            sender: event.sender,
            timestamp: event.timestamp,
            body: event.content.text()?,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_events_bind_scope_and_reject_malformed_inner_content_without_text_fallback() {
        let descriptor = [
            crate::file::DESCRIPTOR_PREFIX.as_slice(),
            &[1; 32],
            &5u64.to_be_bytes(),
            &(crate::file::CHUNK_SIZE as u32).to_be_bytes(),
            &[2; 32],
            &[3; 32],
        ]
        .concat();
        let body = crate::file::File {
            source: "chat.example",
            name: "synthetic.bin",
            media_type: "application/octet-stream",
            expires_at: None,
            access: &[4; 32],
            descriptor: &descriptor,
        }
        .to_bytes()
        .unwrap();
        let direct = Direct {
            message: [5; 32],
            conversation: [6; 32],
            sender: [7; 32],
            recipient: [8; 32],
            timestamp: 9,
            content: Content::File(&body),
        }
        .to_bytes()
        .unwrap();
        let group = Group {
            message: [5; 32],
            group: [6; 32],
            sender: [7; 32],
            timestamp: 9,
            content: Content::File(&body),
        }
        .to_bytes()
        .unwrap();
        assert_eq!(&direct[..10], b"SGEV\0\x01\0\0\x03\0");
        assert_eq!(&group[..10], b"SGEV\0\x01\0\0\x04\0");
        let parsed = Direct::from_bytes(&direct).unwrap();
        assert_eq!(parsed.content.bytes(), body);
        assert_eq!(parsed.content.file().unwrap().name, "synthetic.bin");
        assert!(parsed.content.text().is_err());
        assert_eq!(parsed.to_bytes().unwrap(), direct);
        assert_eq!(
            Group::from_bytes(&group).unwrap().to_bytes().unwrap(),
            group
        );
        assert!(Text::from_bytes(&direct).is_err());
        assert!(GroupText::from_bytes(&group).is_err());
        assert!(Direct::from_bytes(&group).is_err());
        assert!(Group::from_bytes(&direct).is_err());
        for n in 0..direct.len() {
            assert!(Direct::from_bytes(&direct[..n]).is_err());
        }
        for n in 0..group.len() {
            assert!(Group::from_bytes(&group[..n]).is_err());
        }
        for (bytes, header) in [(&direct, 150), (&group, 118)] {
            for offset in [8, 9, header - 1, header, header + 54] {
                let mut bad = bytes.clone();
                bad[offset] ^= 128;
                if header == 150 {
                    assert!(Direct::from_bytes(&bad).is_err());
                } else {
                    assert!(Group::from_bytes(&bad).is_err());
                }
            }
        }
        assert!(Direct {
            message: [5; 32],
            conversation: [6; 32],
            sender: [7; 32],
            recipient: [8; 32],
            timestamp: 9,
            content: Content::File(b"ordinary text")
        }
        .to_bytes()
        .is_err());
    }
    #[test]
    fn group_text_rejects_cross_kind_frames_truncation_and_invalid_bounds() {
        let mut event = GroupText {
            message: [1; 32],
            group: [2; 32],
            sender: [3; 32],
            timestamp: 1,
            body: "مرحبا • 👩🏽‍💻\nhello",
        };
        let bytes = event.to_bytes().unwrap();
        assert!(GroupText::from_bytes(&bytes).unwrap() == event);
        assert!(Text::from_bytes(&bytes).is_err());
        for n in 0..bytes.len() {
            assert!(GroupText::from_bytes(&bytes[..n]).is_err());
        }
        for offset in (0..10).chain(114..118) {
            let mut bad = bytes.clone();
            bad[offset] ^= 128;
            assert!(GroupText::from_bytes(&bad).is_err());
        }
        let mut bad = bytes.clone();
        bad[118] = 255;
        assert!(GroupText::from_bytes(&bad).is_err());
        let mut bad = bytes.clone();
        bad.push(0);
        assert!(GroupText::from_bytes(&bad).is_err());
        for timestamp in [0, u64::MAX] {
            event.timestamp = timestamp;
            assert!(event.to_bytes().is_err());
        }
        event.timestamp = 1;
        event.body = "";
        assert!(event.to_bytes().is_err());
        let max = "a".repeat(GroupText::MAX_BODY);
        event.body = &max;
        assert_eq!(
            event.to_bytes().unwrap().len(),
            crate::initial::MAX_PLAINTEXT
        );
        let excessive = "a".repeat(GroupText::MAX_BODY + 1);
        event.body = &excessive;
        assert!(event.to_bytes().is_err());
    }
    #[test]
    fn text_contract_is_strict_bounded_and_preserves_unicode() {
        let mut event = Text {
            message: [1; 32],
            conversation: [2; 32],
            sender: [3; 32],
            recipient: [4; 32],
            timestamp: 1,
            body: "Hello • مرحبا • 👩🏽‍💻\n```rust```",
        };
        let bytes = event.to_bytes().unwrap();
        assert!(Text::from_bytes(&bytes).unwrap() == event);
        for n in 0..bytes.len() {
            assert!(Text::from_bytes(&bytes[..n]).is_err());
        }
        for offset in (0..10).chain(146..150) {
            let mut bad = bytes.clone();
            bad[offset] ^= 128;
            assert!(Text::from_bytes(&bad).is_err());
        }
        let mut bad = bytes.clone();
        bad[HEADER] = 255;
        assert!(Text::from_bytes(&bad).is_err());
        let mut bad = bytes.clone();
        bad.push(0);
        assert!(Text::from_bytes(&bad).is_err());
        event.body = "";
        assert!(event.to_bytes().is_err());
        let max = "a".repeat(MAX_TEXT);
        event.body = &max;
        assert_eq!(
            event.to_bytes().unwrap().len(),
            crate::initial::MAX_PLAINTEXT
        );
        let too_long = "a".repeat(MAX_TEXT + 1);
        event.body = &too_long;
        assert!(event.to_bytes().is_err());
        event.body = "text";
        for timestamp in [0, u64::MAX] {
            event.timestamp = timestamp;
            assert!(event.to_bytes().is_err());
        }
    }
}
