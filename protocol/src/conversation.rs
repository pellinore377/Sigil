//! Conversation operations carried only inside authenticated application events.
use serde::{Deserialize, Serialize};
pub type Id = [u8; 32];
pub const MAX_BYTES: usize = crate::event::MAX_TEXT;
pub const MAX_SNAPSHOT: usize = crate::event::GroupText::MAX_BODY + 282;
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub author: Id,
    pub message: Id,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub enum Body {
    Text(String),
    File(Vec<u8>),
    Rich(Vec<u8>),
}
impl Body {
    pub fn editable(&self) -> bool {
        match self {
            Self::Rich(v) => matches!(
                crate::text::Document::from_bytes(v),
                Ok(crate::text::Document::Text(_))
            ),
            _ => true,
        }
    }
    pub fn content(&self) -> crate::event::Content<'_> {
        match self {
            Self::Text(v) => crate::event::Content::Text(v),
            Self::File(v) => crate::event::Content::File(v),
            Self::Rich(v) => crate::event::Content::Rich(v),
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Text(v) if !v.is_empty() && v.len() <= crate::event::GroupText::MAX_BODY => {
                Ok(())
            }
            Self::File(v) => crate::file::File::from_bytes(v).map(|_| ()),
            Self::Rich(v) => crate::text::Document::from_bytes(v)
                .map(|_| ())
                .map_err(|_| "invalid rich body"),
            _ => Err("invalid body"),
        }
    }
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub enum Action {
    Post {
        body: Body,
        reply: Option<Reference>,
        thread: Option<Reference>,
        expires_at: Option<u64>,
        view_once: bool,
    },
    Edit {
        target: Reference,
        body: Body,
    },
    Delete {
        target: Reference,
    },
    Reaction {
        target: Reference,
        emoji: String,
        active: bool,
    },
    Pin {
        target: Reference,
        active: bool,
    },
    Note {
        target: Reference,
        active: bool,
    },
    Receipt {
        target: Reference,
        read: bool,
    },
    Typing {
        active: bool,
        until: u64,
    },
    Presence {
        online: bool,
        until: u64,
    },
    Private {
        conversation: Id,
        value: Private,
    },
    SyncPart {
        transfer: Id,
        digest: Id,
        index: u32,
        total: u32,
        payload: String,
    },
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub enum Private {
    Draft {
        text: String,
        observed: Vec<Version>,
    },
    ConversationPin(bool),
    Unread(bool),
    Snooze(Option<u64>),
    Hidden(bool),
    Collection {
        id: Id,
        name: String,
        present: bool,
    },
    CollectionMember {
        id: Id,
        present: bool,
    },
    CollectionsEnabled(bool),
    ReadReceipts(bool),
    TypingIndicators(bool),
    PresenceSharing(bool),
    RecoveryRetention(Option<u32>),
    Consumed(Reference),
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct Version {
    pub device: Id,
    pub counter: u64,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub id: Id,
    pub version: Version,
    pub action: Action,
}
impl Operation {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version.counter == 0 || self.version.counter > i64::MAX as u64 {
            return Err("invalid operation counter");
        }
        match &self.action {
            Action::SyncPart {
                index,
                total,
                payload,
                ..
            } => {
                if *total == 0
                    || *total > 32
                    || *index >= *total
                    || payload.is_empty()
                    || payload.len() > 32768
                    || payload.len() % 2 != 0
                    || !payload
                        .bytes()
                        .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
                {
                    return Err("invalid sync fragment");
                }
            }
            Action::Post {
                body,
                reply,
                thread,
                expires_at,
                ..
            } => {
                body.validate()?;
                if expires_at.is_some_and(|v| v == 0 || v > i64::MAX as u64)
                    || reply.as_ref().is_some_and(|v| v.message == self.id)
                    || thread.as_ref().is_some_and(|v| v.message == self.id)
                {
                    return Err("invalid post metadata");
                }
            }
            Action::Edit { body, .. } => {
                body.validate()?;
                if !body.editable() {
                    return Err("structured content requires structured actions");
                }
            }
            Action::Reaction { emoji, .. }
                if emoji.is_empty() || emoji.len() > 64 || emoji.chars().any(char::is_control) =>
            {
                return Err("invalid reaction")
            }
            Action::Typing { until, .. } | Action::Presence { until, .. }
                if *until == 0 || *until > i64::MAX as u64 =>
            {
                return Err("invalid ephemeral deadline")
            }
            Action::Private { value, .. } => match value {
                Private::Draft { text, observed } => {
                    if text.len() > 32768
                        || observed.len() > 64
                        || observed
                            .iter()
                            .any(|v| v.counter == 0 || v.counter > i64::MAX as u64)
                        || observed.windows(2).any(|v| v[0].device >= v[1].device)
                        || observed.iter().any(|v| {
                            v.device == self.version.device && v.counter >= self.version.counter
                        })
                    {
                        return Err("invalid draft context");
                    }
                }
                Private::Collection { name, .. } if name.is_empty() || name.len() > 256 => {
                    return Err("invalid collection")
                }
                Private::Snooze(Some(v)) if *v == 0 || *v > i64::MAX as u64 => {
                    return Err("invalid snooze")
                }
                Private::RecoveryRetention(Some(0)) => return Err("invalid recovery retention"),
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>, &'static str> {
        self.validate()?;
        let mut bytes = b"SGCO\0\x01".to_vec();
        bytes.extend(serde_json::to_vec(self).map_err(|_| "invalid operation")?);
        if bytes.len() > MAX_BYTES {
            return Err("operation too large");
        }
        Ok(bytes)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() > MAX_BYTES || !bytes.starts_with(b"SGCO\0\x01") {
            return Err("invalid operation frame");
        }
        let result: Self = serde_json::from_slice(&bytes[6..]).map_err(|_| "invalid operation")?;
        if result.to_bytes()? != bytes {
            return Err("noncanonical operation");
        }
        Ok(result)
    }
    pub fn ephemeral(&self) -> bool {
        matches!(
            self.action,
            Action::Typing { .. }
                | Action::Presence { .. }
                | Action::Post {
                    expires_at: Some(_),
                    ..
                }
                | Action::Post {
                    view_once: true,
                    ..
                }
        )
    }
}

pub struct Snapshot {
    pub conversation: Id,
    pub author: Id,
    pub timestamp: u64,
    pub operation: Operation,
}
impl Snapshot {
    pub fn to_bytes(&self) -> Result<Vec<u8>, &'static str> {
        if self.timestamp == 0
            || self.timestamp > i64::MAX as u64
            || self.operation.ephemeral()
            || matches!(self.operation.action, Action::SyncPart { .. })
        {
            return Err("unarchivable conversation operation");
        }
        self.operation.validate()?;
        if let Action::Post {
            body,
            reply,
            thread,
            ..
        } = &self.operation.action
        {
            let mut raw = b"SGCS\0\x01\0\x01".to_vec();
            raw.extend(self.conversation);
            raw.extend(self.author);
            raw.extend(self.timestamp.to_be_bytes());
            raw.extend(self.operation.id);
            raw.extend(self.operation.version.device);
            raw.extend(self.operation.version.counter.to_be_bytes());
            raw.push(u8::from(reply.is_some()) | u8::from(thread.is_some()) << 1);
            for reference in [reply, thread].into_iter().flatten() {
                raw.extend(reference.author);
                raw.extend(reference.message);
            }
            match body {
                Body::Text(v) => {
                    raw.push(0);
                    raw.extend(v.as_bytes());
                }
                Body::File(v) => {
                    raw.push(1);
                    raw.extend(v);
                }
                Body::Rich(v) => {
                    raw.push(2);
                    raw.extend(v);
                }
            }
            return Ok(raw);
        }
        let mut raw = b"SGCS\0\x01\0\0".to_vec();
        raw.extend(self.conversation);
        raw.extend(self.author);
        raw.extend(self.timestamp.to_be_bytes());
        raw.extend(self.operation.to_bytes()?);
        Ok(raw)
    }
    pub fn from_bytes(raw: &[u8]) -> Result<Self, &'static str> {
        if raw.len() < 81 || raw.len() > MAX_SNAPSHOT || !raw.starts_with(b"SGCS\0\x01\0") {
            return Err("invalid conversation snapshot");
        }
        let operation = match raw[7] {
            0 => Operation::from_bytes(&raw[80..])?,
            1 => {
                if raw.len() < 154 || raw[152] > 3 {
                    return Err("invalid post snapshot");
                }
                let flags = raw[152];
                let mut at = 153;
                let mut reference = |bit| -> Result<Option<Reference>, &'static str> {
                    if flags & bit == 0 {
                        return Ok(None);
                    }
                    let bytes = raw.get(at..at + 64).ok_or("short reference")?;
                    at += 64;
                    Ok(Some(Reference {
                        author: bytes[..32].try_into().map_err(|_| "invalid author")?,
                        message: bytes[32..].try_into().map_err(|_| "invalid message")?,
                    }))
                };
                let reply = reference(1)?;
                let thread = reference(2)?;
                let kind = *raw.get(at).ok_or("missing body")?;
                let bytes = &raw[at + 1..];
                let body = match kind {
                    0 => Body::Text(
                        std::str::from_utf8(bytes)
                            .map_err(|_| "invalid text")?
                            .into(),
                    ),
                    1 => Body::File(bytes.into()),
                    2 => Body::Rich(bytes.into()),
                    _ => return Err("invalid body kind"),
                };
                Operation {
                    id: raw[80..112].try_into().map_err(|_| "invalid id")?,
                    version: Version {
                        device: raw[112..144].try_into().map_err(|_| "invalid device")?,
                        counter: u64::from_be_bytes(
                            raw[144..152].try_into().map_err(|_| "invalid counter")?,
                        ),
                    },
                    action: Action::Post {
                        body,
                        reply,
                        thread,
                        expires_at: None,
                        view_once: false,
                    },
                }
            }
            _ => return Err("invalid snapshot kind"),
        };
        let result = Self {
            conversation: raw[8..40].try_into().map_err(|_| "invalid scope")?,
            author: raw[40..72].try_into().map_err(|_| "invalid author")?,
            timestamp: u64::from_be_bytes(raw[72..80].try_into().map_err(|_| "invalid timestamp")?),
            operation,
        };
        if result.to_bytes() != Ok(raw.to_vec()) {
            return Err("invalid conversation snapshot");
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn operation() -> Operation {
        Operation {
            id: [1; 32],
            version: Version {
                device: [2; 32],
                counter: 1,
            },
            action: Action::Post {
                body: Body::Text("🖋️".into()),
                reply: None,
                thread: None,
                expires_at: None,
                view_once: false,
            },
        }
    }
    #[test]
    fn canonical_conversation_operations_reject_ambiguity_and_bad_bounds() {
        let mut op = operation();
        let raw = op.to_bytes().unwrap();
        assert!(Operation::from_bytes(&raw).unwrap() == op);
        for len in 0..raw.len() {
            assert!(Operation::from_bytes(&raw[..len]).is_err());
        }
        let mut extra = raw.clone();
        extra.push(b' ');
        assert!(Operation::from_bytes(&extra).is_err());
        let mut unknown = raw.clone();
        unknown.splice(7..7, b"\"unknown\":0,".iter().copied());
        assert!(Operation::from_bytes(&unknown).is_err());
        op.version.counter = 0;
        assert!(op.to_bytes().is_err());
        op.version.counter = 1;
        op.action = Action::SyncPart {
            transfer: [3; 32],
            digest: [4; 32],
            index: 1,
            total: 1,
            payload: "ff".into(),
        };
        assert!(op.to_bytes().is_err());
        op.action = Action::Private {
            conversation: [5; 32],
            value: Private::Draft {
                text: "draft".into(),
                observed: vec![op.version.clone()],
            },
        };
        assert!(op.to_bytes().is_err());
        op.action = Action::Post {
            body: Body::Text("x".repeat(MAX_BYTES)),
            reply: None,
            thread: None,
            expires_at: None,
            view_once: false,
        };
        assert!(op.to_bytes().is_err());
    }
    #[test]
    fn conversation_snapshots_preserve_maximum_legacy_posts_without_json_expansion() {
        let mut op = operation();
        op.action = Action::Post {
            body: Body::Text("\0".repeat(crate::event::GroupText::MAX_BODY)),
            reply: Some(Reference {
                author: [3; 32],
                message: [4; 32],
            }),
            thread: Some(Reference {
                author: [5; 32],
                message: [6; 32],
            }),
            expires_at: None,
            view_once: false,
        };
        assert!(op.to_bytes().is_err());
        let mut snapshot = Snapshot {
            conversation: [7; 32],
            author: [8; 32],
            timestamp: 1,
            operation: op,
        };
        let raw = snapshot.to_bytes().unwrap();
        assert_eq!(raw.len(), MAX_SNAPSHOT);
        assert!(Snapshot::from_bytes(&raw).unwrap().operation == snapshot.operation);
        let mut extra = raw.clone();
        extra.push(0);
        assert!(Snapshot::from_bytes(&extra).is_err());
        for len in 0..282 {
            assert!(Snapshot::from_bytes(&raw[..len]).is_err());
        }
        snapshot.operation.action = Action::Post {
            body: Body::Text("once".into()),
            reply: None,
            thread: None,
            expires_at: None,
            view_once: true,
        };
        assert!(snapshot.to_bytes().is_err());
    }
}
