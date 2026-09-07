use super::{Id, PREFIX};
/// Content is borrowed from canonical, already bounded bytes. File keys remain
/// sensitive: callers own and zeroize the enclosing plaintext allocation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Content<'a> {
    Text(&'a str),
    File(&'a [u8]),
    Rich(&'a [u8]),
}
impl<'a> Content<'a> {
    pub fn bytes(self) -> &'a [u8] {
        match self {
            Self::Text(text) => text.as_bytes(),
            Self::File(bytes) | Self::Rich(bytes) => bytes,
        }
    }
    pub fn text(self) -> Result<&'a str, &'static str> {
        match self {
            Self::Text(text) => Ok(text),
            _ => Err("not text content"),
        }
    }
    pub fn file(self) -> Result<crate::file::File<'a>, &'static str> {
        match self {
            Self::File(bytes) => crate::file::File::from_bytes(bytes),
            _ => Err("not file content"),
        }
    }
    pub fn rich(self) -> Result<crate::text::Text, &'static str> {
        match self {
            Self::Rich(bytes) => {
                crate::text::Text::from_bytes(bytes).map_err(|_| "invalid SigilText")
            }
            _ => Err("not SigilText content"),
        }
    }
    pub fn card(self) -> Result<crate::text::structured::Card, &'static str> {
        match self {
            Self::Rich(bytes) => crate::text::structured::Card::from_bytes(bytes)
                .map_err(|_| "invalid structured card"),
            _ => Err("not structured content"),
        }
    }
    pub fn action(self) -> Result<crate::text::action::Action, &'static str> {
        match self {
            Self::Rich(bytes) => crate::text::action::Action::from_bytes(bytes)
                .map_err(|_| "invalid structured action"),
            _ => Err("not structured content"),
        }
    }
    fn validate(self) -> Result<(), &'static str> {
        match self {
            Self::Text(text) if !text.is_empty() => Ok(()),
            Self::File(_) => self.file().map(|_| ()),
            Self::Rich(bytes) => crate::text::Document::from_bytes(bytes)
                .map(|_| ())
                .map_err(|_| "invalid SigilText"),
            _ => Err("empty text content"),
        }
    }
}
pub struct Direct<'a> {
    pub message: Id,
    pub conversation: Id,
    pub sender: Id,
    pub recipient: Id,
    pub timestamp: u64,
    pub content: Content<'a>,
}
pub struct Group<'a> {
    pub message: Id,
    pub group: Id,
    pub sender: Id,
    pub timestamp: u64,
    pub content: Content<'a>,
}
struct Frame<'a> {
    message: Id,
    conversation: Id,
    sender: Id,
    recipient: Option<Id>,
    timestamp: u64,
    content: Content<'a>,
}
impl<'a> Frame<'a> {
    fn to_bytes(&self) -> Result<Vec<u8>, &'static str> {
        let header = if self.recipient.is_some() { 150 } else { 118 };
        if self.timestamp == 0
            || self.timestamp > i64::MAX as u64
            || self.content.bytes().len() > crate::initial::MAX_PLAINTEXT - header
        {
            return Err("invalid event bounds");
        }
        self.content.validate()?;
        let kind = match (self.recipient.is_some(), self.content) {
            (true, Content::Text(_)) => 1,
            (false, Content::Text(_)) => 2,
            (true, Content::File(_)) => 3,
            (false, Content::File(_)) => 4,
            (true, Content::Rich(_)) => 5,
            (false, Content::Rich(_)) => 6,
        };
        let mut bytes = Vec::with_capacity(header + self.content.bytes().len());
        bytes.extend_from_slice(PREFIX);
        bytes.extend_from_slice(&[kind, 0]);
        for id in [&self.message, &self.conversation, &self.sender] {
            bytes.extend_from_slice(id);
        }
        if let Some(recipient) = self.recipient {
            bytes.extend_from_slice(&recipient);
        }
        bytes.extend_from_slice(&self.timestamp.to_be_bytes());
        bytes.extend_from_slice(&(self.content.bytes().len() as u32).to_be_bytes());
        bytes.extend_from_slice(self.content.bytes());
        Ok(bytes)
    }
    fn from_bytes(bytes: &'a [u8], direct: bool) -> Result<Self, &'static str> {
        let header = if direct { 150 } else { 118 };
        if !(header + 1..=crate::initial::MAX_PLAINTEXT).contains(&bytes.len())
            || &bytes[..8] != PREFIX
            || bytes[9] != 0
            || (direct && !matches!(bytes[8], 1 | 3 | 5))
            || (!direct && !matches!(bytes[8], 2 | 4 | 6))
        {
            return Err("unsupported event");
        }
        let id = |start| {
            bytes[start..start + 32]
                .try_into()
                .map_err(|_| "invalid event ID")
        };
        let timestamp = u64::from_be_bytes(
            bytes[header - 12..header - 4]
                .try_into()
                .map_err(|_| "invalid event timestamp")?,
        );
        let length = u32::from_be_bytes(
            bytes[header - 4..header]
                .try_into()
                .map_err(|_| "invalid event length")?,
        ) as usize;
        if timestamp == 0 || timestamp > i64::MAX as u64 || length != bytes.len() - header {
            return Err("invalid event bounds");
        }
        let content = if bytes[8] <= 2 {
            Content::Text(
                std::str::from_utf8(&bytes[header..]).map_err(|_| "invalid event encoding")?,
            )
        } else if bytes[8] <= 4 {
            Content::File(&bytes[header..])
        } else {
            Content::Rich(&bytes[header..])
        };
        content.validate()?;
        Ok(Self {
            message: id(10)?,
            conversation: id(42)?,
            sender: id(74)?,
            recipient: if direct { Some(id(106)?) } else { None },
            timestamp,
            content,
        })
    }
}
impl<'a> Direct<'a> {
    pub fn to_bytes(&self) -> Result<Vec<u8>, &'static str> {
        Frame {
            message: self.message,
            conversation: self.conversation,
            sender: self.sender,
            recipient: Some(self.recipient),
            timestamp: self.timestamp,
            content: self.content,
        }
        .to_bytes()
    }
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Self, &'static str> {
        let frame = Frame::from_bytes(bytes, true)?;
        Ok(Self {
            message: frame.message,
            conversation: frame.conversation,
            sender: frame.sender,
            recipient: frame.recipient.ok_or("missing recipient")?,
            timestamp: frame.timestamp,
            content: frame.content,
        })
    }
}
impl<'a> Group<'a> {
    pub fn to_bytes(&self) -> Result<Vec<u8>, &'static str> {
        Frame {
            message: self.message,
            conversation: self.group,
            sender: self.sender,
            recipient: None,
            timestamp: self.timestamp,
            content: self.content,
        }
        .to_bytes()
    }
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Self, &'static str> {
        let frame = Frame::from_bytes(bytes, false)?;
        Ok(Self {
            message: frame.message,
            group: frame.conversation,
            sender: frame.sender,
            timestamp: frame.timestamp,
            content: frame.content,
        })
    }
}

#[cfg(test)]
mod rich_tests {
    use super::*;
    #[test]
    fn canonical_rich_content_has_separate_direct_group_and_legacy_kinds() {
        let text =
            crate::text::parse("redact::secret; bold::visible;", Default::default()).unwrap();
        let body = text.to_bytes().unwrap();
        let event = Direct {
            message: [1; 32],
            conversation: [2; 32],
            sender: [3; 32],
            recipient: [4; 32],
            timestamp: 1,
            content: Content::Rich(&body),
        };
        let bytes = event.to_bytes().unwrap();
        assert_eq!(bytes[8], 5);
        assert!(Direct::from_bytes(&bytes).unwrap().content.rich().unwrap() == text);
        assert!(crate::event::Text::from_bytes(&bytes).is_err());
        assert!(Group::from_bytes(&bytes).is_err());
        let mut bad = bytes.clone();
        bad[8] = 3;
        assert!(Direct::from_bytes(&bad).is_err());
        let group = Group {
            message: [1; 32],
            group: [2; 32],
            sender: [3; 32],
            timestamp: 1,
            content: Content::Rich(&body),
        }
        .to_bytes()
        .unwrap();
        assert_eq!(group[8], 6);
        assert!(Group::from_bytes(&group).unwrap().content.rich().unwrap() == text);
        assert!(Direct {
            content: Content::Rich(b"plain"),
            ..event
        }
        .to_bytes()
        .is_err());
        let plain = Direct {
            content: Content::Text(std::str::from_utf8(&body).unwrap()),
            ..event
        }
        .to_bytes()
        .unwrap();
        assert_eq!(plain[8], 1);
        assert!(Direct::from_bytes(&plain).unwrap().content.rich().is_err());
    }
}
