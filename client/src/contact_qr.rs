use super::*;
use base64ct::{Base64UrlUnpadded, Encoding};

const PREFIX: &str = "sigil:contact:v1:";
const AAD: &[u8] = b"Sigil/contact-invite/v1";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Code {
    binding: String,
    secret: Zeroizing<String>,
    expires: u64,
    signature: String,
}
impl Code {
    fn signing_bytes(&self) -> Result<Vec<u8>, Error> {
        if self.expires == 0 || !sigil_protocol::accounts::valid_credential(&self.secret) {
            return Err(Error::InvalidEvent);
        }
        let bytes = sigil_protocol::device::Statement {
            statement: self.binding.clone(),
        }
        .bytes()
        .map_err(|_| Error::InvalidEvent)?;
        Ok([
            b"Sigil/contact-code/v1\0".as_slice(),
            &self.expires.to_be_bytes(),
            self.secret.as_bytes(),
            &bytes,
        ]
        .concat())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Invite {
    code: Code,
    claimant: Option<String>,
}
impl ClientStore {
    fn contact_invite(&self) -> Result<Option<Invite>, Error> {
        let raw: Option<Vec<u8>> = self.db.query_row("SELECT CASE WHEN length(state)<=4096 THEN state END FROM mobile_contact_invite WHERE id=1", [], |r| r.get(0)).optional()?;
        raw.map(|raw| {
            serde_json::from_slice(&self.key.open(&raw, AAD)?).map_err(|_| Error::InvalidStore)
        })
        .transpose()
    }
    fn save_contact_invite(&self, invite: &Invite) -> Result<(), Error> {
        let raw = Zeroizing::new(serde_json::to_vec(invite).map_err(|_| Error::InvalidStore)?);
        self.db.execute("INSERT INTO mobile_contact_invite VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET state=excluded.state", [self.key.seal(&raw, AAD)?])?;
        Ok(())
    }
    pub(super) fn contact_invite_matches(
        &self,
        incoming: &IncomingRequest,
        now: u64,
    ) -> Result<bool, Error> {
        let Some(tag) = &incoming.invitation else {
            return Ok(false);
        };
        let Some(mut invite) = self.contact_invite()? else {
            return Ok(false);
        };
        if invite.code.expires <= now {
            return Ok(false);
        }
        let own = self.connection_session()?.ok_or(Error::Unprepared)?;
        let request = RequestContact {
            invitation: None,
            server: own
                .address
                .split_once(':')
                .ok_or(Error::InvalidStore)?
                .1
                .into(),
            recipient: own.account_id,
            expires_at: incoming.receipt.expires_at,
            binding: incoming.binding.clone(),
            signature: incoming.signature.clone(),
        };
        let secret = Secret32::from_bytes(id(&invite.code.secret)?);
        if sigil_crypto::link::verify_contact_claim(
            &secret,
            &request.signing_bytes().map_err(|_| Error::InvalidEvent)?,
            &id(tag)?,
        )
        .is_err()
        {
            return Ok(false);
        }
        if let Some(claimant) = &invite.claimant {
            return Ok(*claimant == incoming.signature);
        }
        invite.claimant = Some(incoming.signature.clone());
        self.save_contact_invite(&invite)?;
        Ok(true)
    }
    pub(in crate::mobile) fn mobile_contact_qr(
        &mut self,
        action: &str,
        scanned: Option<&str>,
        target: Option<&str>,
        review: Option<&str>,
    ) -> Result<Value, Error> {
        let now = conversations::now();
        if action == "close" {
            self.db
                .execute("DELETE FROM mobile_contact_invite WHERE id=1", [])?;
            return Ok(json!({"stage":"none"}));
        }
        if action == "status" {
            let invite = self.contact_invite()?;
            return Ok(
                json!({"consumed":invite.as_ref().is_some_and(|v| v.claimant.is_some()),"expired":invite.as_ref().is_none_or(|v| v.code.expires<=now)}),
            );
        }
        if action == "show" {
            self.publish_device_binding_online()?;
            let old = self.contact_invite()?;
            let invite = match old {
                Some(value) if value.code.expires > now && value.claimant.is_none() => value,
                _ => {
                    let mut secret = Zeroizing::new([0; 32]);
                    getrandom::fill(secret.as_mut()).map_err(|_| sigil_crypto::Error::Entropy)?;
                    let mut code = Code {
                        binding: transport::hex(&self.own_device_binding()?),
                        secret: Zeroizing::new(transport::hex(secret.as_ref())),
                        expires: now.saturating_add(600),
                        signature: String::new(),
                    };
                    code.signature = transport::hex(
                        &handshake::identity(&self.db.unchecked_transaction()?, &self.key)?
                            .sign(&code.signing_bytes()?)?,
                    );
                    let invite = Invite {
                        code,
                        claimant: None,
                    };
                    self.save_contact_invite(&invite)?;
                    invite
                }
            };
            let text = format!(
                "{PREFIX}{}",
                Base64UrlUnpadded::encode_string(
                    &serde_json::to_vec(&invite.code).map_err(|_| Error::InvalidStore)?
                )
            );
            let qr =
                qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::M)
                    .map_err(|_| Error::Limit)?;
            return Ok(
                json!({"stage":"show","width":qr.width(),"cells":qr.to_colors().iter().map(|c| if *c == qrcode::Color::Dark {'1'} else {'0'}).collect::<String>(),"expires":invite.code.expires}),
            );
        }
        if action != "scan" {
            return Err(Error::InvalidEvent);
        }
        let text = scanned
            .filter(|s| s.len() <= 4400)
            .and_then(|s| s.strip_prefix(PREFIX))
            .ok_or(Error::InvalidEvent)?;
        let raw =
            Zeroizing::new(Base64UrlUnpadded::decode_vec(text).map_err(|_| Error::InvalidEvent)?);
        let code: Code = serde_json::from_slice(&raw).map_err(|_| Error::InvalidEvent)?;
        if code.expires <= now || code.expires > now.saturating_add(600) {
            return Err(Error::Expired);
        }
        let signed = peers::parse(
            &sigil_protocol::device::Statement {
                statement: code.binding.clone(),
            }
            .bytes()
            .map_err(|_| Error::InvalidEvent)?,
        )?;
        let signature = RequestContact {
            invitation: None,
            server: signed.binding.server.clone(),
            recipient: transport::hex(&signed.binding.account),
            binding: code.binding.clone(),
            expires_at: code.expires,
            signature: code.signature.clone(),
        }
        .signature_bytes()
        .map_err(|_| Error::InvalidEvent)?;
        sigil_crypto::verify_signature(
            &signed.binding.identity,
            &code.signing_bytes()?,
            &signature,
        )?;
        let b = signed.binding;
        let fingerprint = peers::fingerprint(&b)?;
        self.mobile_find(&format!("@{}:{}", b.username, b.server))?;
        let mut contact = self.contact(event::account_reference(&b.server, &b.account))?;
        if contact.blocked || target.is_some_and(|target| target != contact.display()) {
            return Err(Error::Conflict);
        }
        let directory = self.contact_peers(&mut contact, None)?;
        if !directory.bindings.iter().any(|raw| {
            sigil_protocol::device::Statement {
                statement: raw.clone(),
            }
            .bytes()
            .ok()
            .and_then(|bytes| peers::parse(&bytes).ok())
            .is_some_and(|signed| signed.binding == b)
        }) {
            return Err(Error::Conflict);
        }
        let peer = self.peer(peers::reference(&b.server, &b.device))?;
        if peer.fingerprint != fingerprint && peer.changed_fingerprint != Some(fingerprint) {
            return Err(Error::Conflict);
        }
        contact.qr_fingerprint = Some(fingerprint);
        self.save_contact(&contact)?;
        if let Some(expected) = review {
            self.mobile_accept_identity(&contact.display(), id(expected)?)?;
            return Ok(json!({"stage":"done","open":contact.display()}));
        }
        if contact.accepted() {
            if contact.review.is_some() {
                return Err(Error::Conflict);
            }
            self.confirm_peer(peer.id, fingerprint)?;
            return Ok(json!({"stage":"done","open":contact.display()}));
        }
        let mut request = RequestContact {
            server: contact.server.clone(),
            recipient: transport::hex(&contact.account),
            expires_at: code.expires,
            binding: transport::hex(&self.own_device_binding()?),
            signature: String::new(),
            invitation: None,
        };
        request.invitation = Some(transport::hex(&sigil_crypto::link::contact_claim(
            &Secret32::from_bytes(id(&code.secret)?),
            &request.signing_bytes().map_err(|_| Error::InvalidEvent)?,
        )?));
        request.signature = transport::hex(
            &handshake::identity(&self.db.unchecked_transaction()?, &self.key)?
                .sign(&request.signing_bytes().map_err(|_| Error::InvalidEvent)?)?,
        );
        contact.outgoing = Some(request);
        contact.receipt = None;
        contact.work_at = now;
        self.save_contact(&contact)?;
        self.contact_work(contact.clone(), now)?;
        Ok(json!({"stage":"done","open":contact.display()}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn code(store: &ClientStore) -> String {
        format!(
            "{PREFIX}{}",
            Base64UrlUnpadded::encode_string(
                &serde_json::to_vec(&store.contact_invite().unwrap().unwrap().code).unwrap()
            )
        )
    }
    #[test]
    fn in_person_contact_code_authorizes_one_request_without_manual_fingerprint_entry() {
        let (dir, _fixture, mut alice, mut bob, _) = crate::claims::tests::pair();
        alice.publish_device_binding_online().unwrap();
        bob.publish_device_binding_online().unwrap();
        let server = rusqlite::Connection::open(dir.path().join("server.db")).unwrap();
        server.execute("DELETE FROM allowed_senders", []).unwrap();
        bob.mobile_contact_qr("show", None, None, None).unwrap();
        let original = code(&bob);
        let mut altered: Code = serde_json::from_slice(
            &Base64UrlUnpadded::decode_vec(original.strip_prefix(PREFIX).unwrap()).unwrap(),
        )
        .unwrap();
        altered.secret = Zeroizing::new("00".repeat(32));
        let forged = format!(
            "{PREFIX}{}",
            Base64UrlUnpadded::encode_string(&serde_json::to_vec(&altered).unwrap())
        );
        assert!(alice
            .mobile_contact_qr("scan", Some(&forged), None, None)
            .is_err());
        let opened = alice
            .mobile_contact_qr("scan", Some(&original), None, None)
            .unwrap();
        let target = opened["open"].as_str().unwrap();
        assert!(
            !alice
                .peer(alice.mobile_peer(target).unwrap())
                .unwrap()
                .trusted
        );
        bob.mobile_contact_sync(true).unwrap();
        assert!(bob.contact_invite().unwrap().unwrap().claimant.is_some());
        alice.mobile_request(target, "refresh").unwrap();
        let peer = alice.peer(alice.mobile_peer(target).unwrap()).unwrap();
        assert!(peer.trusted && peer.verified);
        let state = bob.mobile_state().unwrap();
        let peer = bob
            .peer(
                bob.mobile_peer(state["chats"][0]["id"].as_str().unwrap())
                    .unwrap(),
            )
            .unwrap();
        assert!(peer.trusted && !peer.verified);
        let mut incoming = bob
            .contact_for(state["chats"][0]["id"].as_str().unwrap())
            .unwrap()
            .incoming
            .unwrap();
        assert!(bob
            .contact_invite_matches(&incoming, conversations::now())
            .unwrap());
        incoming.signature = "00".repeat(64);
        assert!(!bob
            .contact_invite_matches(&incoming, conversations::now())
            .unwrap());
        assert!(!bob
            .contact_invite_matches(&incoming, conversations::now() + 601)
            .unwrap());
        bob.mobile_contact_qr("close", None, None, None).unwrap();
        assert!(bob.contact_invite().unwrap().is_none());
    }
}
