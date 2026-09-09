use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub revision: u64,
    pub display_name: String,
}

pub fn valid_name(name: &str) -> bool {
    name == name.trim()
        && name.len() <= 512
        && name.chars().count() <= 128
        && !name.chars().any(|c| {
            c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
}
