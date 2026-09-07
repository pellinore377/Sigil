//! Secret distribution wire controls and non-secret retained receipts. The
//! receipt authenticates local bookkeeping; it is not an application message.
use super::*;
use sigil_crypto::{sender_keys as sk, storage::StorageKey};
use std::borrow::Cow;
use zeroize::Zeroizing;
const WIRE: &[u8; 8] = b"SGKC\0\x01\0\0";
const MARKER: &[u8; 8] = b"SGKJ\0\x01\0\0";

#[derive(Clone, Copy)]
pub struct DistributionReceipt {
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
    bytes.starts_with(&WIRE[..4])
}
pub(super) fn parse_wire(bytes: &[u8]) -> Result<(Id, Id, sk::Distribution), Error> {
    if bytes.len() != 280 || &bytes[..8] != WIRE {
        return Err(Error::InvalidEvent);
    }
    let message: Id = bytes[8..40].try_into().map_err(|_| Error::InvalidEvent)?;
    let recipient: Id = bytes[40..72].try_into().map_err(|_| Error::InvalidEvent)?;
    let distribution = sk::Distribution::from_bytes(&bytes[72..])?;
    if message_id(&distribution.context(), &recipient) != message {
        return Err(Error::InvalidEvent);
    }
    Ok((message, recipient, distribution))
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
    pub(super) fn to_bytes(self) -> Vec<u8> {
        [
            MARKER.as_slice(),
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
    if bytes.len() != 240 || &bytes[..8] != MARKER {
        return Err(Error::InvalidEvent);
    }
    let receipt = DistributionReceipt {
        message: bytes[8..40].try_into().map_err(|_| Error::InvalidEvent)?,
        recipient: bytes[40..72].try_into().map_err(|_| Error::InvalidEvent)?,
        tag: bytes[72..104].try_into().map_err(|_| Error::InvalidEvent)?,
        context: context_from_bytes(&bytes[104..])?,
    };
    if message_id(&receipt.context, &receipt.recipient) != receipt.message {
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
    if !is_wire_control(plaintext) {
        return Ok(Cow::Borrowed(plaintext));
    }
    let (message, recipient, distribution) = parse_wire(plaintext)?;
    let tag = key.commitment(plaintext, b"Sigil/private-control-receipt/v0")?;
    Ok(Cow::Owned(
        DistributionReceipt {
            message,
            recipient,
            context: distribution.context(),
            tag,
        }
        .to_bytes(),
    ))
}
