use super::*;
use sigil_calls::{Answer, Relay, RelayRequest, SignedConnect, SignedRoster};
use sigil_protocol::federation::Service;
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallAvailability {
    pub revision: u64,
    pub enabled: bool,
    pub max_participants: u8,
}
impl HttpsClient {
    pub fn call_availability(&self) -> Result<CallAvailability, Error> {
        let value: CallAvailability = self.json(
            self.request(Method::GET, "/client/v0/calls", None::<&()>)?,
            200,
            SMALL,
        )?;
        if value.max_participants != 8 || value.revision > i64::MAX as u64 {
            return Err(Error::InvalidResponse);
        }
        Ok(value)
    }
    pub fn publish_call(&self, roster: &SignedRoster) -> Result<(), Error> {
        roster.verify().map_err(|_| Error::Configuration)?;
        if roster.roster.server != self.server {
            return Err(Error::Configuration);
        }
        let reply: SignedRoster = self.json(
            self.request(Method::PUT, "/client/v0/calls", Some(roster))?,
            200,
            16384,
        )?;
        reply.verify().map_err(|_| Error::InvalidResponse)?;
        if *roster != reply {
            return Err(Error::InvalidResponse);
        }
        Ok(())
    }
    fn call_request<T: DeserializeOwned>(
        &self,
        server: &str,
        path: &str,
        body: &impl Serialize,
        relay: bool,
    ) -> Result<T, Error> {
        let bytes = Zeroizing::new(serde_json::to_vec(body).map_err(|_| Error::Configuration)?);
        if bytes.len() > if relay { 2048 } else { 100352 } {
            return Err(Error::Limit);
        }
        if server != self.server {
            let request = String::from_utf8(bytes.to_vec()).map_err(|_| Error::Configuration)?;
            let service = if relay {
                Service::CallRelay { request }
            } else {
                Service::CallConnect { request }
            };
            let reply = self.federated_service(server, service)?;
            if reply.len() > 98304 {
                return Err(Error::Limit);
            }
            return serde_json::from_str(&reply).map_err(|_| Error::InvalidResponse);
        }
        let request = Request::builder()
            .method(Method::POST)
            .uri(format!("{}{path}", self.origin))
            .header(header::ACCEPT, "application/json")
            .header(header::ACCEPT_ENCODING, "identity")
            .header(header::CONTENT_TYPE, "application/json")
            .body(bytes.as_slice())
            .map_err(|_| Error::Configuration)?;
        self.json(self.send(request)?, 200, 98304)
    }
    pub fn connect_call(
        &self,
        roster: &SignedRoster,
        proof: &SignedConnect,
        now: u64,
    ) -> Result<Answer, Error> {
        roster.verify().map_err(|_| Error::Configuration)?;
        proof
            .verify(&roster.roster, now)
            .map_err(|_| Error::Configuration)?;
        let answer: Answer =
            self.call_request(&roster.roster.server, "/calls/v0/connect", proof, false)?;
        if answer.sequence != proof.request.sequence
            || answer.roster != proof.request.roster
            || answer.participant != proof.request.participant
            || answer.sdp.len() > 65536
            || answer.streams.len() != proof.request.layout.downloads.len()
        {
            return Err(Error::InvalidResponse);
        }
        let mut ssrcs = std::collections::BTreeSet::new();
        let mut mids = std::collections::BTreeSet::new();
        for stream in &answer.streams {
            if stream.ssrc == 0
                || !ssrcs.insert(stream.ssrc)
                || !mids.insert(&stream.track.mid)
                || !proof.request.layout.downloads.contains(&stream.track)
            {
                return Err(Error::InvalidResponse);
            }
        }
        Ok(answer)
    }
    pub fn call_relay(
        &self,
        roster: &SignedRoster,
        proof: &RelayRequest,
        now: u64,
    ) -> Result<Option<Relay>, Error> {
        roster.verify().map_err(|_| Error::Configuration)?;
        proof
            .verify(&roster.roster, now)
            .map_err(|_| Error::Configuration)?;
        let reply: Option<Relay> =
            self.call_request(&roster.roster.server, "/calls/v0/relay", proof, true)?;
        if let Some(value) = &reply {
            if value.urls.is_empty()
                || value.urls.len() > 4
                || value
                    .urls
                    .iter()
                    .any(|v| sigil_calls::validate_turn_url(v).is_err())
                || value.expires <= now
                || value.expires > now.saturating_add(660)
                || value.expires > roster.roster.expires
                || value.username.len() > 128
                || value.credential.len() > 256
            {
                return Err(Error::InvalidResponse);
            }
        }
        Ok(reply)
    }
}
