use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub revision: u64,
    pub display_name: String,
}

pub const MAX_PHOTO: usize = 128 * 1024;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Photo {
    pub revision: u64,
    pub hash: Option<String>,
    pub bytes: u32,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContactProfile {
    pub profile: Profile,
    pub photo: Photo,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetPhoto {
    pub revision: u64,
    pub photo: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShareProfile {
    pub server: String,
    pub account: String,
    pub allowed: bool,
}
pub fn photo_bytes(encoded: &str) -> Result<Vec<u8>, &'static str> {
    if encoded.len() > MAX_PHOTO * 2
        || !encoded.len().is_multiple_of(2)
        || !encoded
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("invalid profile photo");
    }
    let bytes = (0..encoded.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&encoded[i..i + 2], 16).map_err(|_| "invalid photo byte"))
        .collect::<Result<Vec<_>, _>>()?;
    if !bytes.is_empty()
        && (bytes.len() < 4 || !bytes.starts_with(&[0xff, 0xd8]) || !bytes.ends_with(&[0xff, 0xd9]))
    {
        return Err("profile photos must be JPEG");
    }
    Ok(bytes)
}

pub fn valid_name(name: &str) -> bool {
    name == name.trim()
        && name.len() <= 512
        && name.chars().count() <= 128
        && !name.chars().any(|c| {
            c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
}
