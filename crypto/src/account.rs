//! Account identity: one XEdDSA key endorses each device binding. Recovery keeps it,
//! sealed under the recovery secret; passkeys wrap that secret.
use crate::{storage::StorageKey, verify_signature, Error, IdentityKey, Secret32};
use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

const ENDORSEMENT: &[u8] = b"Sigil/account-endorsement/v1\0";
const BUNDLE: &[u8] = b"Sigil/account-bundle/v1\0";
const WRAP: &[u8] = b"Sigil/recovery-wrap/v1\0";
const BUNDLE_MAGIC: &[u8; 8] = b"SGAB\0\x01\0\0";

/// Binds the account key to one exact device binding fingerprint.
pub fn endorsement_bytes(account_key: &[u8; 32], device: &[u8; 32]) -> Vec<u8> {
    [ENDORSEMENT, account_key, device].concat()
}
pub fn endorse(key: &IdentityKey, device: &[u8; 32]) -> Result<[u8; 64], Error> {
    key.sign(&endorsement_bytes(&key.public_key(), device))
}
pub fn verify_endorsement(
    account_key: &[u8; 32],
    device: &[u8; 32],
    signature: &[u8],
) -> Result<(), Error> {
    verify_signature(account_key, &endorsement_bytes(account_key, device), signature)
}

/// Stable, user-comparable account fingerprint.
pub fn account_fingerprint(account_key: &[u8; 32]) -> [u8; 32] {
    Sha256::digest([b"Sigil/account-fingerprint/v1\0".as_slice(), account_key].concat()).into()
}

fn derived(secret: &[u8], domain: &[u8], scope: &[u8; 32]) -> Result<StorageKey, Error> {
    let mut key = Zeroizing::new([0; 32]);
    Hkdf::<Sha256>::new(Some(scope), secret)
        .expand(domain, key.as_mut())
        .map_err(|_| Error::State)?;
    StorageKey::new(Secret32(key))
}

/// Server-stored account key, readable only with the recovery secret.
pub fn seal_bundle(secret: &Secret32, scope: &[u8; 32], key: &IdentityKey) -> Result<Vec<u8>, Error> {
    let storage = derived(secret.0.as_ref(), BUNDLE, scope)?;
    key.seal_checkpoint(&storage, &[BUNDLE_MAGIC.as_slice(), scope, &key.public_key()].concat())
}
pub fn open_bundle(
    secret: &Secret32,
    scope: &[u8; 32],
    public: &[u8; 32],
    sealed: &[u8],
) -> Result<IdentityKey, Error> {
    let storage = derived(secret.0.as_ref(), BUNDLE, scope)?;
    let key = IdentityKey::open_checkpoint(
        &storage,
        sealed,
        &[BUNDLE_MAGIC.as_slice(), scope, public].concat(),
    )?;
    if key.public_key() != *public {
        return Err(Error::Authentication);
    }
    Ok(key)
}

/// Wraps the recovery secret under a passkey PRF output. `credential` binds the passkey.
pub fn wrap_secret(
    prf: &[u8; 32],
    scope: &[u8; 32],
    credential: &[u8],
    secret: &Secret32,
) -> Result<Vec<u8>, Error> {
    derived(prf, WRAP, scope)?.seal(secret.0.as_ref(), &[scope.as_slice(), credential].concat())
}
pub fn unwrap_secret(
    prf: &[u8; 32],
    scope: &[u8; 32],
    credential: &[u8],
    wrapped: &[u8],
) -> Result<Secret32, Error> {
    let bytes = derived(prf, WRAP, scope)?.open(wrapped, &[scope.as_slice(), credential].concat())?;
    Ok(Secret32::from_bytes(
        bytes.as_slice().try_into().map_err(|_| Error::Encoding)?,
    ))
}

impl Secret32 {
    pub fn expose(&self) -> &[u8; 32] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endorsements_bind_key_and_device() {
        let key = IdentityKey::generate().unwrap();
        let other = IdentityKey::generate().unwrap();
        let signature = endorse(&key, &[7; 32]).unwrap();
        assert!(verify_endorsement(&key.public_key(), &[7; 32], &signature).is_ok());
        assert!(verify_endorsement(&key.public_key(), &[8; 32], &signature).is_err());
        assert!(verify_endorsement(&other.public_key(), &[7; 32], &signature).is_err());
    }

    #[test]
    fn bundle_opens_only_with_its_secret_scope_and_key() {
        let key = IdentityKey::generate().unwrap();
        let secret = Secret32::generate().unwrap();
        let sealed = seal_bundle(&secret, &[1; 32], &key).unwrap();
        let opened = open_bundle(&secret, &[1; 32], &key.public_key(), &sealed).unwrap();
        assert_eq!(opened.public_key(), key.public_key());
        assert!(open_bundle(&Secret32::generate().unwrap(), &[1; 32], &key.public_key(), &sealed).is_err());
        assert!(open_bundle(&secret, &[2; 32], &key.public_key(), &sealed).is_err());
        let other = IdentityKey::generate().unwrap().public_key();
        assert!(open_bundle(&secret, &[1; 32], &other, &sealed).is_err());
    }

    #[test]
    fn passkey_wrap_is_bound_to_output_scope_and_credential() {
        let secret = Secret32::generate().unwrap();
        let wrapped = wrap_secret(&[3; 32], &[1; 32], b"cred", &secret).unwrap();
        assert_eq!(
            unwrap_secret(&[3; 32], &[1; 32], b"cred", &wrapped).unwrap().expose(),
            secret.expose()
        );
        assert!(unwrap_secret(&[4; 32], &[1; 32], b"cred", &wrapped).is_err());
        assert!(unwrap_secret(&[3; 32], &[2; 32], b"cred", &wrapped).is_err());
        assert!(unwrap_secret(&[3; 32], &[1; 32], b"other", &wrapped).is_err());
    }
}
