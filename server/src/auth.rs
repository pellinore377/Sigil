use crate::store::private_file;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
};
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub struct AdminToken([u8; 32]);

impl AdminToken {
    pub(crate) fn fingerprint(&self) -> [u8; 32] {
        self.0
    }
    pub fn load_or_create(path: &Path) -> io::Result<Self> {
        if !path.exists() {
            Self::create(path)?;
        }
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 || metadata.len() != 64
        {
            return Err(io::Error::other(
                "invalid admin token file; expected private 64-byte file",
            ));
        }
        let value = fs::read_to_string(path)?;
        if !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(io::Error::other("invalid admin token encoding"));
        }
        Ok(Self(Sha256::digest(value.as_bytes()).into()))
    }

    fn create(path: &Path) -> io::Result<()> {
        let value = random_secret()?;
        let mut file = private_file(path)?;
        file.write_all(value.as_bytes())?;
        file.sync_all()
    }

    pub fn rotate(path: &Path) -> io::Result<()> {
        let temporary = path.with_extension("new");
        Self::create(&temporary)?;
        fs::rename(&temporary, path)?;
        if let Some(parent) = path.parent() {
            fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    }

    pub fn accepts(&self, bearer: &str) -> bool {
        if bearer.len() != 64 {
            return false;
        }
        bool::from(self.0.ct_eq(&Sha256::digest(bearer.as_bytes())[..]))
    }
}

pub fn random_secret() -> io::Result<String> {
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(io::Error::other)?;
    Ok(random.iter().map(|b| format!("{b:02x}")).collect())
}

pub(crate) fn digest(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}
