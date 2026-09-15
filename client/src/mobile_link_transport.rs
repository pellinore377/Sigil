use super::*;
impl ClientStore {
    pub(in crate::mobile) fn mobile_link(
        &mut self,
        action: &str,
        scanned: Option<&str>,
        server: Option<&str>,
    ) -> Result<Value, Error> {
        if scanned.is_some_and(|v| v.len() > 4400) {
            return Err(Error::Limit);
        }
        if action == "join" && self.link_flow()?.is_none() {
            let host = super::super::login_server(server.ok_or(Error::InvalidEvent)?)?;
            let (port, roots) = relay::endpoint();
            let methods = network::HttpsClient::login_methods(&host, port, &roots)?;
            self.link_step("join", None)?;
            let mut flow = self.link_flow()?.ok_or(Error::Unprepared)?;
            let offer = self.prepare_device_link_offer(flow.attempt, conversations::now())?;
            flow.relay = Some(relay::Relay::new(methods.server_name, offer.expires_at)?);
            self.save_link_flow(&flow)?;
        } else if action == "sponsor" {
            self.link_step("sponsor", None)?;
        }
        let Some(mut flow) = self.link_flow()? else {
            return Ok(json!({"stage":"none"}));
        };
        if (action == "join" && flow.sponsor) || (action == "sponsor" && !flow.sponsor) {
            return Err(Error::Conflict);
        }
        if action == "close" {
            return self.link_step(action, None);
        }
        if action == "cancel" {
            let network = flow.relay.as_ref().and_then(|v| v.network().ok());
            let value = self.link_step(action, None)?;
            if let (Some(relay), Some(network)) = (flow.relay, network) {
                let _ = relay.exchange(&network, true);
            }
            return Ok(value);
        }
        if action == "status" {
            return self.link_view();
        }
        if action == "scan" {
            if !flow.sponsor || flow.stage != "scan_offer" {
                return Err(Error::Conflict);
            }
            let session = self.connection_session()?.ok_or(Error::Unprepared)?;
            let server = session
                .address
                .rsplit_once(':')
                .ok_or(Error::InvalidStore)?
                .1;
            let (relay, _) = relay::Relay::scan(
                scanned.ok_or(Error::InvalidEvent)?,
                server,
                conversations::now(),
            )?;
            if flow
                .relay
                .as_ref()
                .is_some_and(|old| old.id != relay.id || old.secret != relay.secret)
            {
                return Err(Error::Conflict);
            }
            flow.relay = Some(relay);
            self.save_link_flow(&flow)?;
        } else if action == "confirm" {
            if !flow.sponsor || flow.stage != "confirm_sponsor" {
                return Err(Error::Conflict);
            }
            let expected =
                crate::link::emoji_confirmation(flow.digest.ok_or(Error::Unprepared)?)[0];
            if scanned != Some(expected) {
                return Err(Error::InvalidEvent);
            }
            self.link_step("confirm", None)?;
        } else if !matches!(action, "join" | "sponsor" | "poll" | "retry") {
            return Err(Error::InvalidEvent);
        }
        self.advance_link()?;
        self.link_view()
    }
    fn advance_link(&mut self) -> Result<(), Error> {
        let mut flow = self.link_flow()?.ok_or(Error::Unprepared)?;
        if flow.stage == "done" {
            return Ok(());
        }
        if flow.stage == "authorize" || flow.stage == "cancelling" {
            self.link_step("retry", None)?;
            return Ok(());
        }
        let Some(relay) = flow.relay.as_ref() else {
            if !flow.sponsor && flow.stage == "show_response" {
                match self.link_step("finish", None) {
                    Ok(_) => (),
                    Err(Error::Network(network::Error::Status { code: 401, .. })) => (),
                    Err(e) => return Err(e),
                }
            }
            return Ok(());
        };
        let network = relay.network()?;
        if !flow.sponsor && !relay.registered {
            relay.reserve(&network)?;
            flow.relay.as_mut().unwrap().registered = true;
            self.save_link_flow(&flow)?;
        }
        if flow.sponsor && flow.stage == "scan_offer" {
            let offer = flow.relay.as_ref().unwrap().offer.clone();
            self.link_step("scan", Some(&offer))?;
            flow = self.link_flow()?.ok_or(Error::Unprepared)?;
        }
        if flow.sponsor && flow.stage == "show_proposal" {
            flow.relay.as_mut().unwrap().seal(&flow.qr, true)?;
            self.save_link_flow(&flow)?;
            if let Some(packet) = flow.relay.as_ref().unwrap().exchange(&network, false)? {
                let response = flow.relay.as_ref().unwrap().open(&packet, false)?;
                self.link_step("scan", Some(&response))?;
            }
            return Ok(());
        }
        if !flow.sponsor && flow.stage == "show_offer" {
            if let Some(packet) = flow.relay.as_ref().unwrap().exchange(&network, false)? {
                let proposal = flow.relay.as_ref().unwrap().open(&packet, true)?;
                self.link_step("scan", Some(&proposal))?;
                flow = self.link_flow()?.ok_or(Error::Unprepared)?;
            }
        }
        if !flow.sponsor && flow.stage == "confirm_join" {
            self.link_step("confirm", None)?;
            flow = self.link_flow()?.ok_or(Error::Unprepared)?;
        }
        if !flow.sponsor && flow.stage == "show_response" {
            flow.relay.as_mut().unwrap().seal(&flow.qr, false)?;
            self.save_link_flow(&flow)?;
            // Finishing remains possible after the transient relay expires.
            match self.link_step("finish", None) {
                Ok(_) => return Ok(()),
                Err(Error::Network(network::Error::Status { code: 401, .. })) => (),
                Err(error) => return Err(error),
            }
            flow.relay.as_ref().unwrap().exchange(&network, false)?;
        }
        Ok(())
    }
    fn link_view(&mut self) -> Result<Value, Error> {
        let Some(flow) = self.link_flow()? else {
            return Ok(json!({"stage":"none"}));
        };
        let Some(relay) = flow.relay.as_ref() else {
            let mut value = self.link_step("status", None)?;
            if flow.stage == "done" || (flow.sponsor && flow.stage == "scan_offer") {
                return Ok(value);
            }
            value["stage"] = json!(if !flow.sponsor && flow.stage == "show_response" {
                "wait_approval"
            } else {
                "restart_required"
            });
            return Ok(value);
        };
        let mut value = self.link_step("status", None)?;
        if let Some(digest) = flow.digest {
            let correct = crate::link::emoji_confirmation(digest)[0];
            if flow.sponsor {
                let mut choices = vec![correct];
                for n in 0u32..1024 {
                    let hash: Id = sha2::Sha256::digest(
                        [
                            b"Sigil/link-choices/v1".as_slice(),
                            &digest,
                            &n.to_be_bytes(),
                        ]
                        .concat(),
                    )
                    .into();
                    let emoji = crate::link::emoji_confirmation(hash)[0];
                    if !choices.contains(&emoji) {
                        choices.push(emoji);
                    }
                    if choices.len() == 6 {
                        break;
                    }
                }
                choices.sort_by_key(|emoji| {
                    sha2::Sha256::digest([digest.as_slice(), emoji.as_bytes()].concat()).to_vec()
                });
                value["choices"] = json!(choices);
                value.as_object_mut().unwrap().remove("emoji");
            } else {
                value["emoji"] = json!([correct]);
            }
        }
        value.as_object_mut().unwrap().remove("cells");
        value.as_object_mut().unwrap().remove("width");
        match flow.stage.as_str() {
            "show_offer" if relay.registered => {
                let code = Zeroizing::new(relay.code(flow.qr)?);
                let qr = qrcode::QrCode::with_error_correction_level(
                    code.as_bytes(),
                    qrcode::EcLevel::M,
                )
                .map_err(|_| Error::Limit)?;
                value["width"] = json!(qr.width());
                value["cells"] = json!(qr
                    .to_colors()
                    .into_iter()
                    .map(|c| if c == qrcode::Color::Dark { '1' } else { '0' })
                    .collect::<String>());
            }
            "show_offer" => value["stage"] = json!("prepare_offer"),
            "show_proposal" => value["stage"] = json!("exchanging"),
            "show_response" | "confirm_join" => value["stage"] = json!("wait_approval"),
            _ => (),
        }
        Ok(value)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Endpoint;
    impl Drop for Endpoint {
        fn drop(&mut self) {
            relay::TEST_ENDPOINT.with(|v| *v.borrow_mut() = None)
        }
    }
    fn open(path: &std::path::Path) -> ClientStore {
        ClientStore::open(
            path,
            StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn one_scan_survives_restart_and_only_phone_approval_enrolls_the_browser() {
        let (dir, fixture, invitation, now) = crate::connection::tests::setup();
        relay::TEST_ENDPOINT.with(|v| {
            *v.borrow_mut() = Some((fixture.port(), vec![crate::network::tests::CA.to_vec()]))
        });
        let _endpoint = Endpoint;
        let mut server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
        let mut policy = server.administration_policy().unwrap();
        policy.public_origin = Some(format!("https://chat.example:{}", fixture.port()));
        server.configure_administration(policy).unwrap();
        let phone_path = dir.path().join("phone.db");
        let browser_path = dir.path().join("browser.db");
        let mut phone = open(&phone_path);
        crate::connection::tests::prepare(&mut phone, &fixture, &invitation.secret);
        phone.enroll_online().unwrap();
        let mut browser = open(&browser_path);
        let view = browser
            .mobile_link("join", None, Some("chat.example"))
            .unwrap();
        assert_eq!(view["stage"], "show_offer");
        assert!(view["width"].as_u64().unwrap() <= 177);
        let flow = browser.link_flow().unwrap().unwrap();
        let code = flow.relay.as_ref().unwrap().code(flow.qr.clone()).unwrap();
        phone.mobile_link("sponsor", None, None).unwrap();
        let pending = phone.mobile_link("scan", Some(&code), None).unwrap();
        assert_eq!(pending["stage"], "exchanging");
        assert!(pending.get("cells").is_none());
        let waiting = browser.mobile_link("poll", None, None).unwrap();
        assert_eq!(waiting["stage"], "wait_approval");
        assert!(waiting.get("cells").is_none());
        drop(browser);
        drop(phone);
        let mut browser = open(&browser_path);
        let mut phone = open(&phone_path);
        let approval = phone.mobile_link("poll", None, None).unwrap();
        assert_eq!(approval["stage"], "confirm_sponsor");
        let correct = waiting["emoji"][0].as_str().unwrap();
        assert!(approval["choices"]
            .as_array()
            .unwrap()
            .contains(&json!(correct)));
        assert!(approval.get("emoji").is_none());
        assert!(phone.mobile_link("confirm", Some("wrong"), None).is_err());
        assert_eq!(server.admin_diagnostics(now).unwrap()["devices"], 1);
        assert!(matches!(browser.connection_session(), Err(Error::NotFound)));
        assert!(browser.mobile_link("confirm", None, None).is_err());
        assert_eq!(
            browser.mobile_link("poll", None, None).unwrap()["stage"],
            "wait_approval"
        );
        assert_eq!(
            phone.mobile_link("confirm", Some(correct), None).unwrap()["stage"],
            "done"
        );
        assert_eq!(
            browser.mobile_link("poll", None, None).unwrap()["stage"],
            "done"
        );
        assert_eq!(server.admin_diagnostics(now).unwrap()["devices"], 2);
        assert_eq!(
            phone.connection_session().unwrap().unwrap().account_id,
            browser.connection_session().unwrap().unwrap().account_id
        );
        assert_eq!(
            browser.mobile_link("poll", None, None).unwrap()["stage"],
            "done"
        );
    }
}
