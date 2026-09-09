//! Shared linking signature validation and ephemeral encrypted QR frames.
use crate::{storage::StorageKey, verify_signature, Error, IdentityKey, Secret32};
use sha2::{Digest, Sha256};
use sigil_protocol::{
    device::{Binding, SignedBinding},
    link::{Proof, Transcript},
};
use zeroize::Zeroizing;

pub fn fingerprint(binding: &Binding) -> Result<[u8; 32], Error> {
    Ok(Sha256::digest(binding.signing_bytes().map_err(|_| Error::Encoding)?).into())
}
pub fn confirmation(transcript: &Transcript) -> Result<[u8; 32], Error> {
    Ok(Sha256::digest(
        [
            b"Sigil/device-link-confirmation/v0".as_slice(),
            &transcript.to_bytes().map_err(|_| Error::Encoding)?,
        ]
        .concat(),
    )
    .into())
}
pub fn signing_bytes(transcript: &Transcript, sponsor: bool) -> Result<Vec<u8>, Error> {
    Ok([
        b"Sigil/device-link-consent/v0".as_slice(),
        &[u8::from(sponsor)],
        &transcript.to_bytes().map_err(|_| Error::Encoding)?,
    ]
    .concat())
}
pub fn context(
    transcript: &Transcript,
    sponsor: &[u8],
    joining: &[u8],
) -> Result<(SignedBinding, SignedBinding), Error> {
    transcript.to_bytes().map_err(|_| Error::Encoding)?;
    let a = SignedBinding::from_bytes(sponsor).map_err(|_| Error::Encoding)?;
    let b = SignedBinding::from_bytes(joining).map_err(|_| Error::Encoding)?;
    for signed in [&a, &b] {
        verify_signature(
            &signed.binding.identity,
            &signed
                .binding
                .signing_bytes()
                .map_err(|_| Error::Encoding)?,
            &signed.signature,
        )?;
    }
    if fingerprint(&a.binding)? != transcript.sponsor
        || fingerprint(&b.binding)? != transcript.joining
        || a.binding.server != b.binding.server
        || a.binding.account != b.binding.account
        || a.binding.username != b.binding.username
        || a.binding.device == b.binding.device
        || a.binding.identity == b.binding.identity
        || transcript.provisioning_key == a.binding.identity
        || transcript.provisioning_key == b.binding.identity
    {
        return Err(Error::Authentication);
    }
    Ok((a, b))
}
pub fn verify(proof: &Proof, trusted_sponsor: [u8; 32], now: u64) -> Result<[u8; 32], Error> {
    if trusted_sponsor != proof.transcript.sponsor
        || now < proof.transcript.created_at
        || now >= proof.transcript.expires_at
    {
        return Err(Error::Authentication);
    }
    context(
        &proof.transcript,
        &proof.sponsor.to_bytes().map_err(|_| Error::Encoding)?,
        &proof.joining.to_bytes().map_err(|_| Error::Encoding)?,
    )?;
    verify_signature(
        &proof.sponsor.binding.identity,
        &signing_bytes(&proof.transcript, true)?,
        &proof.sponsor_signature,
    )?;
    verify_signature(
        &proof.joining.binding.identity,
        &signing_bytes(&proof.transcript, false)?,
        &proof.joining_signature,
    )?;
    confirmation(&proof.transcript)
}
fn channel(own: &IdentityKey, peer: &[u8; 32], context: &[u8; 32]) -> Result<StorageKey, Error> {
    let shared = own.exchange(peer)?;
    let mut key = Zeroizing::new([0; 32]);
    hkdf::Hkdf::<Sha256>::new(Some(context), shared.0.as_ref())
        .expand(b"Sigil/link-provisioning/v0", key.as_mut())
        .map_err(|_| Error::Limit)?;
    StorageKey::new(Secret32::from_bytes(*key))
}
pub fn seal(
    own: &IdentityKey,
    peer: &[u8; 32],
    context: &[u8; 32],
    role: u8,
    bytes: &[u8],
) -> Result<Vec<u8>, Error> {
    if bytes.len() > 2048 || role > 1 {
        return Err(Error::Limit);
    }
    channel(own, peer, context)?.seal(
        bytes,
        &[b"Sigil/link-frame/v0".as_slice(), context, &[role]].concat(),
    )
}
pub fn open(
    own: &IdentityKey,
    peer: &[u8; 32],
    context: &[u8; 32],
    role: u8,
    bytes: &[u8],
) -> Result<Zeroizing<Vec<u8>>, Error> {
    if bytes.len() > 2084 || role > 1 {
        return Err(Error::Limit);
    }
    channel(own, peer, context)?.open(
        bytes,
        &[b"Sigil/link-frame/v0".as_slice(), context, &[role]].concat(),
    )
}

fn contact_mac(secret: &Secret32, request: &[u8]) -> Result<hmac::Hmac<sha2::Sha256>, Error> {
    use hmac::Mac;
    if request.len() > 2048 {
        return Err(Error::Limit);
    }
    let mut mac = <hmac::Hmac<sha2::Sha256> as hmac::KeyInit>::new_from_slice(secret.0.as_ref())
        .map_err(|_| Error::InvalidKey)?;
    mac.update(b"Sigil/contact-code-claim/v1\0");
    mac.update(request);
    Ok(mac)
}
pub fn contact_claim(secret: &Secret32, request: &[u8]) -> Result<[u8; 32], Error> {
    use hmac::Mac;
    Ok(contact_mac(secret, request)?.finalize().into_bytes().into())
}
pub fn verify_contact_claim(
    secret: &Secret32,
    request: &[u8],
    tag: &[u8; 32],
) -> Result<(), Error> {
    use hmac::Mac;
    contact_mac(secret, request)?
        .verify_slice(tag)
        .map_err(|_| Error::Authentication)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contact_claim_binds_the_request_and_requires_the_scanned_secret() {
        let secret = Secret32::from_bytes([19; 32]);
        let tag = contact_claim(&secret, b"synthetic request").unwrap();
        verify_contact_claim(&secret, b"synthetic request", &tag).unwrap();
        assert!(verify_contact_claim(&secret, b"another request", &tag).is_err());
        assert!(
            verify_contact_claim(&Secret32::from_bytes([20; 32]), b"synthetic request", &tag)
                .is_err()
        );
        assert!(contact_claim(&secret, &[0; 2049]).is_err());
    }
    #[test]
    fn provisioning_frames_bind_keys_context_role_and_every_ciphertext_byte() {
        let a = IdentityKey::generate().unwrap();
        let b = IdentityKey::generate().unwrap();
        let c = IdentityKey::generate().unwrap();
        let context = [7; 32];
        let packet = seal(&a, &b.public_key(), &context, 0, b"synthetic provisioning").unwrap();
        assert_eq!(
            open(&b, &a.public_key(), &context, 0, &packet)
                .unwrap()
                .as_slice(),
            b"synthetic provisioning"
        );
        assert!(open(&b, &a.public_key(), &context, 1, &packet).is_err());
        assert!(open(&b, &a.public_key(), &[8; 32], 0, &packet).is_err());
        assert!(open(&c, &a.public_key(), &context, 0, &packet).is_err());
        assert!(seal(&a, &[0; 32], &context, 0, b"x").is_err());
        for index in 0..packet.len() {
            let mut bad = packet.clone();
            bad[index] ^= 1;
            assert!(open(&b, &a.public_key(), &context, 0, &bad).is_err());
        }
        assert!(seal(&a, &b.public_key(), &context, 0, &vec![0; 2049]).is_err());
    }
}
