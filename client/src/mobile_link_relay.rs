use super::*;
use sigil_protocol::link::{RelayCreate, RelayExchange};
#[cfg(test)]
thread_local! {pub(super) static TEST_ENDPOINT:std::cell::RefCell<Option<(u16,Vec<Vec<u8>>)>>=const{std::cell::RefCell::new(None)};}
pub(super) fn endpoint() -> (u16, Vec<Vec<u8>>) {
    #[cfg(test)]
    if let Some(value) = TEST_ENDPOINT.with(|v| v.borrow().clone()) {
        return value;
    }
    (443, Vec::new())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Relay {
    pub server: String,
    pub id: String,
    pub secret: Zeroizing<Id>,
    pub token: Zeroizing<String>,
    pub phone: Zeroizing<String>,
    pub expires: u64,
    pub outgoing: Option<String>,
    pub offer: String,
    pub registered: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Code {
    server: String,
    id: String,
    secret: Zeroizing<Id>,
    token: Zeroizing<String>,
    offer: String,
    expires: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Join {
    server: String,
    id: String,
    secret: Zeroizing<Id>,
    token: Zeroizing<String>,
    expires: u64,
}
fn random() -> Result<Id, Error> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| sigil_crypto::Error::Entropy)?;
    Ok(bytes)
}
impl Relay {
    pub fn new(server: String, expires: u64) -> Result<Self, Error> {
        Ok(Self {
            server,
            id: transport::hex(&random()?),
            secret: Zeroizing::new(random()?),
            token: Zeroizing::new(transport::hex(&random()?)),
            phone: Zeroizing::new(transport::hex(&random()?)),
            expires,
            outgoing: None,
            offer: String::new(),
            registered: false,
        })
    }
    pub fn code(&self, offer: String) -> Result<String, Error> {
        let code = Code {
            server: self.server.clone(),
            id: self.id.clone(),
            secret: self.secret.clone(),
            token: self.phone.clone(),
            offer,
            expires: self.expires,
        };
        Ok(format!(
            "sigil:link:v1:relay:{}",
            serde_json::to_string(&code).map_err(|_| Error::InvalidStore)?
        ))
    }
    /// Shown by an existing device so a new one needs no server address.
    pub fn join_code(&self) -> Result<String, Error> {
        let code = Join {
            server: self.server.clone(),
            id: self.id.clone(),
            secret: self.secret.clone(),
            token: self.phone.clone(),
            expires: self.expires,
        };
        Ok(format!(
            "sigil:link:v1:join:{}",
            serde_json::to_string(&code).map_err(|_| Error::InvalidStore)?
        ))
    }
    pub fn scan_join(text: &str, now: u64) -> Result<Self, Error> {
        let raw = text
            .strip_prefix("sigil:link:v1:join:")
            .filter(|t| t.len() <= 1000)
            .ok_or(Error::InvalidEvent)?;
        let code: Join = serde_json::from_str(raw).map_err(|_| Error::InvalidEvent)?;
        if !sigil_protocol::valid_server_name(&code.server)
            || !sigil_protocol::accounts::valid_credential(&code.id)
            || !sigil_protocol::accounts::valid_credential(&code.token)
            || *code.secret == [0; 32]
        {
            return Err(Error::InvalidEvent);
        }
        if code.expires <= now || code.expires > now.saturating_add(600) {
            return Err(Error::Expired);
        }
        Ok(Self {
            server: code.server,
            id: code.id,
            secret: code.secret,
            token: code.token,
            phone: Zeroizing::new(String::new()),
            expires: code.expires,
            outgoing: None,
            offer: String::new(),
            registered: true,
        })
    }
    pub fn scan(text: &str, server: &str, now: u64) -> Result<(Self, String), Error> {
        if text.len() > 4400 {
            return Err(Error::Limit);
        }
        let raw = text
            .strip_prefix("sigil:link:v1:relay:")
            .ok_or(Error::InvalidEvent)?;
        let code: Code = serde_json::from_str(raw).map_err(|_| Error::InvalidEvent)?;
        if code.server != server
            || !sigil_protocol::accounts::valid_credential(&code.id)
            || !sigil_protocol::accounts::valid_credential(&code.token)
            || *code.secret == [0; 32]
        {
            return Err(Error::InvalidEvent);
        }
        if code.expires <= now || code.expires > now.saturating_add(600) {
            return Err(Error::Expired);
        }
        let offer = code.offer;
        Ok((
            Self {
                server: code.server,
                id: code.id,
                secret: code.secret,
                token: code.token,
                phone: Zeroizing::new(String::new()),
                expires: code.expires,
                outgoing: None,
                offer: offer.clone(),
                registered: true,
            },
            offer,
        ))
    }
    pub fn network(&self) -> Result<network::HttpsClient, Error> {
        let (port, roots) = endpoint();
        Ok(network::HttpsClient::discover(
            &self.server,
            port,
            &"0".repeat(64),
            &roots,
        )?)
    }
    pub fn reserve(&self, network: &network::HttpsClient) -> Result<(), Error> {
        network.reserve_link_relay(&RelayCreate {
            id: self.id.clone(),
            owner: self.token.to_string(),
            phone: self.phone.to_string(),
            expires: self.expires,
        })?;
        Ok(())
    }
    fn aad(&self, phone: bool) -> Vec<u8> {
        [
            b"Sigil/device-link-relay/v1/".as_slice(),
            self.server.as_bytes(),
            b"/",
            self.id.as_bytes(),
            if phone { b"/proposal" } else { b"/response" },
        ]
        .concat()
    }
    pub fn seal(&mut self, text: &str, phone: bool) -> Result<(), Error> {
        if self.outgoing.is_none() {
            let key = StorageKey::new(Secret32::from_bytes(*self.secret))?;
            self.outgoing = Some(transport::hex(
                &key.seal(text.as_bytes(), &self.aad(phone))?,
            ));
        }
        Ok(())
    }
    pub fn open(&self, packet: &str, phone: bool) -> Result<Zeroizing<String>, Error> {
        if packet.len() > 10000 || !packet.len().is_multiple_of(2) {
            return Err(Error::Limit);
        }
        let mut bytes = Vec::with_capacity(packet.len() / 2);
        for pair in packet.as_bytes().as_chunks::<2>().0 {
            bytes.push(
                u8::from_str_radix(
                    std::str::from_utf8(pair).map_err(|_| Error::InvalidEvent)?,
                    16,
                )
                .map_err(|_| Error::InvalidEvent)?,
            )
        }
        let key = StorageKey::new(Secret32::from_bytes(*self.secret))?;
        let plain = key.open(&bytes, &self.aad(phone))?;
        Ok(Zeroizing::new(
            std::str::from_utf8(&plain)
                .map_err(|_| Error::InvalidEvent)?
                .to_owned(),
        ))
    }
    pub fn exchange(
        &self,
        network: &network::HttpsClient,
        cancel: bool,
    ) -> Result<Option<String>, Error> {
        Ok(network
            .exchange_link_relay(&RelayExchange {
                id: self.id.clone(),
                token: self.token.to_string(),
                packet: if cancel { None } else { self.outgoing.clone() },
                cancel,
            })?
            .packet)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn qr_secret_authenticates_session_server_and_direction() {
        let owner = Relay::new("chat.example".into(), 700).unwrap();
        let code = owner.code("synthetic offer".into()).unwrap();
        assert!(Relay::scan(&code, "other.example", 100).is_err());
        assert!(Relay::scan(&code, "chat.example", 700).is_err());
        let (mut phone, offer) = Relay::scan(&code, "chat.example", 100).unwrap();
        assert_eq!(offer, "synthetic offer");
        phone.seal("synthetic proposal", true).unwrap();
        let packet = phone.outgoing.as_ref().unwrap();
        assert_eq!(&*owner.open(packet, true).unwrap(), "synthetic proposal");
        assert!(owner.open(packet, false).is_err());
        let mut forged = Relay::new("chat.example".into(), 700).unwrap();
        forged.id = owner.id.clone();
        assert!(forged.open(packet, true).is_err());
        let mut changed: Relay =
            serde_json::from_str(&serde_json::to_string(&owner).unwrap()).unwrap();
        changed.server = "other.example".into();
        assert!(changed.open(packet, true).is_err());
        changed.server = owner.server.clone();
        changed.id = "ff".repeat(32);
        assert!(changed.open(packet, true).is_err());
        let mut bytes = packet.clone().into_bytes();
        let last = bytes.last_mut().unwrap();
        *last = if *last == b'0' { b'1' } else { b'0' };
        assert!(owner
            .open(std::str::from_utf8(&bytes).unwrap(), true)
            .is_err());
    }
}
