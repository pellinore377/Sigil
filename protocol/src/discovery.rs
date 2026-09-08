use serde::{Deserialize, Serialize};
pub const PATH: &str = "/.well-known/sigil";
pub const MAX_BODY: usize = 4096;
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Discovery {
    pub server_name: String,
    pub api_origin: String,
}
impl Discovery {
    pub fn valid_for(&self, server: &str) -> bool {
        self.server_name == server
            && crate::valid_server_name(server)
            && valid_origin(&self.api_origin)
    }
}
pub fn valid_origin(origin: &str) -> bool {
    let Some(authority) = origin.strip_prefix("https://") else {
        return false;
    };
    let (host, port) = authority
        .split_once(':')
        .map_or((authority, None), |(h, p)| (h, Some(p)));
    crate::valid_server_name(host)
        && port.is_none_or(|p| p.parse::<u16>().is_ok_and(|n| n != 0 && n.to_string() == p))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_canonical_https_origins_are_delegated() {
        for origin in [
            "http://example.test",
            "https://user@example.test",
            "https://example.test/",
            "https://example.test?q=1",
            "https://127.0.0.1",
            "https://example.test:0",
            "https://example.test:0443",
            "https://example.test#x",
        ] {
            assert!(!valid_origin(origin), "{origin}");
        }
        let d = Discovery {
            server_name: "example.test".into(),
            api_origin: "https://sigil.example.test".into(),
        };
        assert!(d.valid_for("example.test"));
        assert!(!d.valid_for("other.test"));
    }
}
