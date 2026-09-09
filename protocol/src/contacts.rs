use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateContactInvite {
    /// Generate and persist 32 random bytes as lowercase hex before submission.
    pub secret: String,
    pub expires_at: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RedeemContactInvite {
    pub secret: String,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ContactInvite {
    pub id: String,
    pub device_id: String,
    pub expires_at: u64,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ContactPeer {
    pub device_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RequestContact {
    pub server: String,
    pub recipient: String,
    pub expires_at: u64,
    pub binding: String,
    pub signature: String,
}
impl RequestContact {
    pub fn valid(&self) -> bool {
        self.signing_bytes().is_ok() && self.signature_bytes().is_ok()
    }
    pub fn signing_bytes(&self) -> Result<Vec<u8>, &'static str> {
        if !crate::valid_server_name(&self.server)
            || !crate::accounts::valid_credential(&self.recipient)
            || self.expires_at == 0
            || self.expires_at > i64::MAX as u64
        {
            return Err("invalid contact request");
        }
        let binding = crate::device::Statement {
            statement: self.binding.clone(),
        }
        .bytes()?;
        crate::device::SignedBinding::from_bytes(&binding)?;
        Ok([
            b"Sigil/contact-request/v1\0".as_slice(),
            &(self.server.len() as u16).to_be_bytes(),
            self.server.as_bytes(),
            self.recipient.as_bytes(),
            &self.expires_at.to_be_bytes(),
            &(binding.len() as u16).to_be_bytes(),
            &binding,
        ]
        .concat())
    }
    pub fn signature_bytes(&self) -> Result<[u8; 64], &'static str> {
        if self.signature.len() != 128
            || !self
                .signature
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("invalid contact signature");
        }
        let mut bytes = [0; 64];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&self.signature[index * 2..index * 2 + 2], 16)
                .map_err(|_| "invalid contact signature")?;
        }
        Ok(bytes)
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RequestState {
    Pending,
    Accepted,
    Declined,
    Blocked,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RequestReceipt {
    pub id: String,
    pub state: RequestState,
    pub expires_at: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IncomingRequest {
    pub receipt: RequestReceipt,
    pub origin: String,
    pub account: String,
    pub device: String,
    pub binding: String,
    pub signature: String,
    pub created_at: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RequestPage {
    pub requests: Vec<IncomingRequest>,
    pub next: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveRequest {
    pub state: RequestState,
    pub signature: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestPolicy {
    pub enabled: bool,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BlockContact {
    pub server: String,
    pub account: String,
    pub blocked: bool,
}
