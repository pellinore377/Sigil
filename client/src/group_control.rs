//! Secret distribution wire controls and non-secret retained receipts. The
//! receipt authenticates local bookkeeping; it is not an application message.
use super::*;
use sigil_crypto::{sender_keys as sk, storage::StorageKey};
use std::borrow::Cow;
use zeroize::Zeroizing;
const WIRE: &[u8; 8] = b"SGKC\0\x01\0\0";
const MARKER: &[u8; 8] = b"SGKJ\0\x01\0\0";
const REPAIR: &[u8; 8] = b"SGKC\0\x02\0\0";
const REQUEST: &[u8; 8] = b"SGKQ\0\x01\0\0";

#[derive(Clone, Copy)]
pub struct DistributionReceipt {
    pub(super) kind: u8,
    pub message: Id,
    pub context: sk::Context,
    pub(super) recipient: Id,
    pub(super) tag: Id,
}
pub(super) fn context_bytes(context: &sk::Context) -> Vec<u8> {
    [
        context.group.as_slice(),
        &context.state,
        &context.epoch.to_be_bytes(),
        &context.sender,
        &context.chain,
    ]
    .concat()
}
pub(super) fn context_from_bytes(bytes: &[u8]) -> Result<sk::Context, Error> {
    if bytes.len() != 136 {
        return Err(Error::InvalidEvent);
    }
    let id = |n| <Id>::try_from(&bytes[n..n + 32]).map_err(|_| Error::InvalidEvent);
    Ok(sk::Context {
        group: id(0)?,
        state: id(32)?,
        epoch: u64::from_be_bytes(bytes[64..72].try_into().map_err(|_| Error::InvalidEvent)?),
        sender: id(72)?,
        chain: id(104)?,
    })
}
pub(super) fn message_id(context: &sk::Context, recipient: &Id) -> Id {
    digest(
        b"Sigil/group-key-delivery/v0",
        &[&context_bytes(context), recipient],
    )
}
pub(crate) fn is_wire_control(bytes: &[u8]) -> bool {
    is_distribution_wire(bytes) || super::invitation::is_wire(bytes)
}
pub(crate) fn is_distribution_wire(bytes: &[u8]) -> bool {
    bytes.starts_with(&WIRE[..4]) || bytes.starts_with(&REQUEST[..4]) || history::is_wire(bytes)
}
pub(crate) fn distribution_group(bytes: &[u8]) -> Result<Id, Error> {
    if history::is_wire(bytes) {
        return history::group(bytes);
    }
    if bytes.starts_with(&REQUEST[..4]) {
        return Ok(parse_request(bytes)?.context.group);
    }
    Ok(parse_wire(bytes)?.2.context().group)
}
pub(super) fn parse_wire(bytes: &[u8]) -> Result<(Id, Id, sk::Distribution), Error> {
    if !((bytes.len() == 280 && &bytes[..8] == WIRE)
        || (bytes.len() == 320 && &bytes[..8] == REPAIR))
    {
        return Err(Error::InvalidEvent);
    }
    let message: Id = bytes[8..40].try_into().map_err(|_| Error::InvalidEvent)?;
    let recipient: Id = bytes[40..72].try_into().map_err(|_| Error::InvalidEvent)?;
    let repair = &bytes[..8] == REPAIR;
    let distribution = sk::Distribution::from_bytes(&bytes[if repair { 104 } else { 72 }..])?;
    let expected = if repair {
        repair_id(&distribution.context(), &recipient, &bytes[72..104])
    } else {
        message_id(&distribution.context(), &recipient)
    };
    if distribution.is_current() != repair || expected != message {
        return Err(Error::InvalidEvent);
    }
    Ok((message, recipient, distribution))
}
pub(super) fn repair_id(context: &sk::Context, recipient: &Id, request: &[u8]) -> Id {
    digest(
        b"Sigil/group-key-repair/v0",
        &[&context_bytes(context), recipient, request],
    )
}
pub(super) fn repair_wire(
    distribution: &sk::Distribution,
    recipient: &Id,
    request: &Id,
) -> Result<Zeroizing<Vec<u8>>, Error> {
    if !distribution.is_current() {
        return Err(Error::InvalidEvent);
    }
    let mut out = Zeroizing::new(REPAIR.to_vec());
    out.extend_from_slice(&repair_id(&distribution.context(), recipient, request));
    out.extend_from_slice(recipient);
    out.extend_from_slice(request);
    out.extend_from_slice(&distribution.to_bytes()?);
    Ok(out)
}
pub(super) fn request_wire(context: &sk::Context, recipient: &Id, nonce: &Id) -> Vec<u8> {
    let message = digest(
        b"Sigil/group-key-request/v0",
        &[&context_bytes(context), recipient, nonce],
    );
    [
        REQUEST.as_slice(),
        &message,
        recipient,
        &context_bytes(context),
        nonce,
    ]
    .concat()
}
pub(super) fn parse_request(bytes: &[u8]) -> Result<DistributionReceipt, Error> {
    if bytes.len() != 240 || &bytes[..8] != REQUEST {
        return Err(Error::InvalidEvent);
    }
    let context = context_from_bytes(&bytes[72..208])?;
    let recipient = bytes[40..72].try_into().map_err(|_| Error::InvalidEvent)?;
    let message = bytes[8..40].try_into().map_err(|_| Error::InvalidEvent)?;
    if context.chain != [0; 32]
        || request_wire(
            &context,
            &recipient,
            &bytes[208..].try_into().map_err(|_| Error::InvalidEvent)?,
        ) != bytes
    {
        return Err(Error::InvalidEvent);
    }
    Ok(DistributionReceipt {
        kind: 2,
        message,
        context,
        recipient,
        tag: [0; 32],
    })
}
pub(super) fn wire(
    distribution: &sk::Distribution,
    recipient: &Id,
) -> Result<Zeroizing<Vec<u8>>, Error> {
    let mut bytes = Zeroizing::new(Vec::with_capacity(280));
    bytes.extend_from_slice(WIRE);
    bytes.extend_from_slice(&message_id(&distribution.context(), recipient));
    bytes.extend_from_slice(recipient);
    bytes.extend_from_slice(&distribution.to_bytes()?);
    Ok(bytes)
}
impl DistributionReceipt {
    pub fn history_share(&self) -> Option<Id> {
        (self.kind == 3).then_some(self.context.chain)
    }
    pub(super) fn to_bytes(self) -> Vec<u8> {
        let mut marker = *MARKER;
        marker[5] += self.kind;
        [
            marker.as_slice(),
            &self.message,
            &self.recipient,
            &self.tag,
            &context_bytes(&self.context),
        ]
        .concat()
    }
}
pub(crate) fn distribution_receipt(bytes: &[u8]) -> Result<Option<DistributionReceipt>, Error> {
    if !bytes.starts_with(&MARKER[..4]) {
        return Ok(None);
    }
    if bytes.len() != 240
        || bytes[..5] != MARKER[..5]
        || !(1..=4).contains(&bytes[5])
        || bytes[6..8] != [0, 0]
    {
        return Err(Error::InvalidEvent);
    }
    let receipt = DistributionReceipt {
        kind: bytes[5] - 1,
        message: bytes[8..40].try_into().map_err(|_| Error::InvalidEvent)?,
        recipient: bytes[40..72].try_into().map_err(|_| Error::InvalidEvent)?,
        tag: bytes[72..104].try_into().map_err(|_| Error::InvalidEvent)?,
        context: context_from_bytes(&bytes[104..])?,
    };
    if receipt.kind == 0 && message_id(&receipt.context, &receipt.recipient) != receipt.message {
        return Err(Error::InvalidEvent);
    }
    Ok(Some(receipt))
}
/// Called before caching any pairwise plaintext. Never retain a live sender
/// chain in ordinary inbox/outbox history, even temporarily inside the write.
pub(crate) fn retained_payload<'a>(
    key: &StorageKey,
    plaintext: &'a [u8],
) -> Result<Cow<'a, [u8]>, Error> {
    if history::is_wire(plaintext) {
        return Ok(Cow::Owned(history::marker(key, plaintext)?.to_bytes()));
    }
    if let Some(marker) = super::invitation::retained(key, plaintext)? {
        return Ok(Cow::Owned(marker));
    }
    if !is_distribution_wire(plaintext) {
        return Ok(Cow::Borrowed(plaintext));
    }
    if plaintext.starts_with(&REQUEST[..4]) {
        let mut receipt = parse_request(plaintext)?;
        receipt.tag = key.commitment(plaintext, b"Sigil/private-control-receipt/v0")?;
        return Ok(Cow::Owned(receipt.to_bytes()));
    }
    let (message, recipient, distribution) = parse_wire(plaintext)?;
    let tag = key.commitment(plaintext, b"Sigil/private-control-receipt/v0")?;
    Ok(Cow::Owned(
        DistributionReceipt {
            kind: u8::from(distribution.is_current()),
            message,
            recipient,
            context: distribution.context(),
            tag,
        }
        .to_bytes(),
    ))
}
