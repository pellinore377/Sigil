use super::*;
use rusqlite::OptionalExtension;
use serde::Serialize;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Flow {
    attempt: Id,
    sponsor: bool,
    stage: String,
    qr: String,
    response: String,
    digest: Option<Id>,
}
impl ClientStore {
    fn link_flow(&mut self) -> Result<Option<Flow>, Error> {
        let bytes: Option<Vec<u8>> = self
            .db
            .query_row("SELECT state FROM mobile_link WHERE id=1", [], |row| {
                row.get(0)
            })
            .optional()?;
        bytes
            .map(|bytes| {
                if bytes.len() > 16000 {
                    return Err(Error::InvalidStore);
                }
                let binding = self.link_flow_binding()?;
                let plain = self.key.open(&bytes, &binding)?;
                serde_json::from_slice(&plain).map_err(|_| Error::InvalidStore)
            })
            .transpose()
    }
    fn save_link_flow(&mut self, flow: &Flow) -> Result<(), Error> {
        let bytes = Zeroizing::new(serde_json::to_vec(flow).map_err(|_| Error::InvalidStore)?);
        let binding = self.link_flow_binding()?;
        self.db.execute("INSERT INTO mobile_link VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET state=excluded.state", [self.key.seal(&bytes, &binding)?])?;
        Ok(())
    }
    fn link_flow_binding(&mut self) -> Result<Vec<u8>, Error> {
        Ok([b"Sigil/mobile-link/v1".as_slice(), &self.identity()?].concat())
    }
    pub(super) fn mobile_link(
        &mut self,
        action: &str,
        scanned: Option<&str>,
    ) -> Result<Value, Error> {
        if scanned.is_some_and(|qr| qr.len() > 4400) {
            return Err(Error::Limit);
        }
        if !matches!(
            action,
            "join"
                | "sponsor"
                | "status"
                | "scan"
                | "confirm"
                | "finish"
                | "retry"
                | "cancel"
                | "close"
        ) {
            return Err(Error::InvalidEvent);
        }
        let mut flow = self.link_flow()?;
        if matches!(action, "join" | "sponsor") && flow.is_none() {
            let sponsor = action == "sponsor";
            if sponsor {
                self.connected_client()?;
            } else if self
                .db
                .query_row("SELECT EXISTS(SELECT 1 FROM connection)", [], |row| {
                    row.get::<_, bool>(0)
                })?
            {
                return Err(Error::Conflict);
            }
            let mut attempt = [0; 32];
            self.identity()?;
            getrandom::fill(&mut attempt).map_err(|_| sigil_crypto::Error::Entropy)?;
            let initial = Flow {
                attempt,
                sponsor,
                stage: if sponsor {
                    "scan_offer"
                } else {
                    "prepare_offer"
                }
                .into(),
                qr: String::new(),
                response: String::new(),
                digest: None,
            };
            self.save_link_flow(&initial)?;
            flow = Some(initial);
        }
        let Some(mut flow) = flow else {
            return Ok(json!({"stage":"none"}));
        };
        if matches!(action, "join" | "sponsor") && flow.sponsor != (action == "sponsor") {
            return Err(Error::Conflict);
        }
        let now = conversations::now();
        let preparing = flow.stage == "prepare_offer" && matches!(action, "join" | "retry");
        if preparing {
            let offer = self.prepare_device_link_offer(flow.attempt, now)?;
            flow.qr = crate::link::offer_qr(&offer)?;
            flow.stage = "show_offer".into();
            self.save_link_flow(&flow)?;
        }
        match action {
            "join" | "sponsor" | "status" => (),
            "retry" if preparing => (),
            "scan" => {
                let qr = scanned.ok_or(Error::InvalidEvent)?;
                match flow.stage.as_str() {
                    "scan_offer" => {
                        let (proposal, digest) =
                            self.prepare_sponsored_link(flow.attempt, qr, now)?;
                        flow.qr = proposal;
                        flow.digest = Some(digest);
                        flow.stage = "show_proposal".into();
                    }
                    "show_offer" => {
                        flow.digest = Some(self.accept_link_proposal(flow.attempt, qr, now)?);
                        flow.qr.clear();
                        flow.stage = "confirm_join".into();
                    }
                    "show_proposal" => {
                        if !qr.starts_with("sigil:link:v1:response:") {
                            return Err(Error::InvalidEvent);
                        }
                        flow.response = qr.to_owned();
                        flow.qr.clear();
                        flow.stage = "confirm_sponsor".into();
                    }
                    _ => return Err(Error::Conflict),
                }
                self.save_link_flow(&flow)?;
            }
            "confirm" => {
                let digest = flow.digest.ok_or(Error::Unprepared)?;
                match flow.stage.as_str() {
                    "confirm_join" => {
                        flow.qr = self.confirm_link_proposal(flow.attempt, digest, now)?;
                        flow.stage = "show_response".into();
                    }
                    "confirm_sponsor" => {
                        self.confirm_sponsored_link(flow.attempt, &flow.response, digest, now)?;
                        flow.stage = "authorize".into();
                    }
                    _ => return Err(Error::Conflict),
                }
                self.save_link_flow(&flow)?;
            }
            "finish" if flow.stage == "show_response" => {
                self.finish_device_link_online(flow.attempt, 443, &[])?;
                flow.stage = "done".into();
                flow.qr.clear();
                self.save_link_flow(&flow)?;
            }
            "retry" if flow.stage == "authorize" || flow.stage == "cancelling" => (),
            "cancel" if flow.stage != "done" => {
                if !flow.sponsor {
                    let tx = self
                        .db
                        .transaction_with_behavior(TransactionBehavior::Immediate)?;
                    crate::link::discard_unapproved_offer(&tx, &self.key, flow.attempt)?;
                    tx.execute("DELETE FROM mobile_link WHERE id=1", [])?;
                    tx.commit()?;
                    return Ok(json!({"stage":"none"}));
                }
                flow.stage = "cancelling".into();
                self.save_link_flow(&flow)?;
            }
            "close" if flow.stage == "done" => {
                self.db.execute("DELETE FROM mobile_link WHERE id=1", [])?;
                return Ok(json!({"stage":"none"}));
            }
            _ => return Err(Error::InvalidEvent),
        }
        if flow.stage == "authorize" && matches!(action, "confirm" | "retry") {
            self.authorize_sponsored_link_online(flow.attempt)?;
            flow.stage = "done".into();
            flow.response.clear();
            self.save_link_flow(&flow)?;
        }
        if flow.stage == "cancelling" && matches!(action, "cancel" | "retry") {
            let cancelled = self.cancel_sponsored_link_online(flow.attempt);
            match cancelled {
                Ok(()) | Err(Error::Unprepared) => (),
                Err(error) => return Err(error),
            }
            self.db.execute("DELETE FROM mobile_link WHERE id=1", [])?;
            return Ok(json!({"stage":"none"}));
        }
        let mut value = json!({"stage":flow.stage,"sponsor":flow.sponsor,"emoji":flow.digest.map(crate::link::emoji_confirmation)});
        value["can_cancel"] = json!(flow.sponsor || !self.joining_link_approved(flow.attempt)?);
        if flow.sponsor {
            value["account"] = json!(self.connection_session()?.ok_or(Error::Unprepared)?.address);
        } else if flow.digest.is_some() {
            value["account"] = json!(self.joining_link_address(flow.attempt)?);
        }
        if !flow.qr.is_empty() {
            let qr =
                qrcode::QrCode::with_error_correction_level(flow.qr.as_bytes(), qrcode::EcLevel::M)
                    .map_err(|_| Error::Limit)?;
            value["width"] = json!(qr.width());
            value["cells"] = json!(qr
                .to_colors()
                .into_iter()
                .map(|c| if c == qrcode::Color::Dark { '1' } else { '0' })
                .collect::<String>());
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::tests::{prepare, setup};
    fn open(path: &std::path::Path) -> ClientStore {
        ClientStore::open(
            path,
            StorageKey::new(Secret32::from_bytes([9; 32])).unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn approval_is_explicit_and_restart_preserves_the_same_exchange() {
        let (dir, fixture, invitation, _) = setup();
        let sp = dir.path().join("sponsor.db");
        let jp = dir.path().join("joining.db");
        let mut sponsor = open(&sp);
        prepare(&mut sponsor, &fixture, &invitation.secret);
        sponsor.enroll_online().unwrap();
        let mut joining = open(&jp);
        assert_eq!(
            joining.mobile_link("join", None).unwrap()["stage"],
            "show_offer"
        );
        let offer = joining.link_flow().unwrap().unwrap().qr;
        drop(joining);
        let mut joining = open(&jp);
        assert_eq!(joining.link_flow().unwrap().unwrap().qr, offer);
        assert_eq!(
            sponsor.mobile_link("sponsor", None).unwrap()["stage"],
            "scan_offer"
        );
        assert!(sponsor.mobile_link("confirm", None).is_err());
        sponsor.db.execute_batch("CREATE TRIGGER fail_ui BEFORE UPDATE ON mobile_link BEGIN SELECT RAISE(ABORT,'disk failure'); END;").unwrap();
        assert!(sponsor.mobile_link("scan", Some(&offer)).is_err());
        sponsor.db.execute_batch("DROP TRIGGER fail_ui;").unwrap();
        drop(sponsor);
        let mut sponsor = open(&sp);
        let proposal_view = sponsor.mobile_link("scan", Some(&offer)).unwrap();
        let proposal = sponsor.link_flow().unwrap().unwrap().qr;
        assert!(proposal_view["width"].as_u64().unwrap() <= 177);
        let confirmation = joining.mobile_link("scan", Some(&proposal)).unwrap();
        assert_eq!(confirmation["emoji"], proposal_view["emoji"]);
        assert_eq!(confirmation["stage"], "confirm_join");
        assert_eq!(confirmation["account"], proposal_view["account"]);
        assert!(joining.mobile_link("finish", None).is_err());
        joining.mobile_link("confirm", None).unwrap();
        let pending = joining.link_flow().unwrap().unwrap();
        assert_eq!(
            joining.mobile_link("status", None).unwrap()["can_cancel"],
            false
        );
        assert!(matches!(
            joining.mobile_link("cancel", None),
            Err(Error::Conflict)
        ));
        assert_eq!(
            joining.mobile_link("status", None).unwrap()["stage"],
            "show_response"
        );
        let response = pending.qr;
        assert_eq!(
            sponsor.mobile_link("scan", Some(&response)).unwrap()["stage"],
            "confirm_sponsor"
        );
        let mut server = sigil_server::store::Store::open(&dir.path().join("server.db")).unwrap();
        let credential = crate::connection::tests::credential(&sponsor);
        assert_eq!(
            server
                .list_devices(&credential, None, conversations::now())
                .unwrap()
                .devices
                .len(),
            1
        );
        drop(sponsor);
        let mut sponsor = open(&sp);
        assert_eq!(
            sponsor.mobile_link("status", None).unwrap()["stage"],
            "confirm_sponsor"
        );
        sponsor.db.execute_batch("CREATE TABLE test_ui_writes(n INTEGER); INSERT INTO test_ui_writes VALUES(0);
            CREATE TRIGGER fail_done BEFORE UPDATE ON mobile_link WHEN (SELECT n FROM test_ui_writes)>0 BEGIN SELECT RAISE(ABORT,'disk failure'); END;
            CREATE TRIGGER count_ui AFTER UPDATE ON mobile_link BEGIN UPDATE test_ui_writes SET n=n+1; END;").unwrap();
        assert!(sponsor.mobile_link("confirm", None).is_err());
        assert_eq!(
            sponsor.mobile_link("status", None).unwrap()["stage"],
            "authorize"
        );
        assert_eq!(
            server
                .list_devices(&credential, None, conversations::now())
                .unwrap()
                .devices
                .len(),
            2
        );
        sponsor
            .db
            .execute_batch(
                "DROP TRIGGER fail_done; DROP TRIGGER count_ui; DROP TABLE test_ui_writes;",
            )
            .unwrap();
        drop(sponsor);
        let mut sponsor = open(&sp);
        assert_eq!(sponsor.mobile_link("retry", None).unwrap()["stage"], "done");
        joining
            .finish_device_link_online(
                pending.attempt,
                fixture.port(),
                &[crate::network::tests::CA.to_vec()],
            )
            .unwrap();
        // The platform's completion retries the already persisted receipt.
        assert_eq!(
            joining.mobile_link("finish", None).unwrap()["stage"],
            "done"
        );
        assert_eq!(
            sponsor.connection_session().unwrap().unwrap().account_id,
            joining.connection_session().unwrap().unwrap().account_id
        );
        assert_eq!(joining.mobile_link("close", None).unwrap()["stage"], "none");
        assert!(joining.mobile_link("join", None).is_err());
    }
    #[test]
    fn cancellation_is_durable_and_local_frames_are_identity_bound() {
        let (dir, fixture, invitation, _) = setup();
        let mut sponsor = open(&dir.path().join("sponsor.db"));
        prepare(&mut sponsor, &fixture, &invitation.secret);
        sponsor.enroll_online().unwrap();
        let mut joining = open(&dir.path().join("joining.db"));
        joining.mobile_link("join", None).unwrap();
        let offer = joining.link_flow().unwrap().unwrap().qr;
        sponsor.mobile_link("sponsor", None).unwrap();
        sponsor.mobile_link("scan", Some(&offer)).unwrap();
        let attempt = sponsor.link_flow().unwrap().unwrap().attempt;
        let raw: Vec<u8> = sponsor
            .db
            .query_row("SELECT state FROM mobile_link", [], |r| r.get(0))
            .unwrap();
        assert!(!raw.windows(offer.len()).any(|w| w == offer.as_bytes()));
        joining
            .db
            .execute("UPDATE mobile_link SET state=?1", [raw])
            .unwrap();
        assert!(joining.mobile_link("status", None).is_err());
        assert_eq!(
            sponsor.mobile_link("cancel", None).unwrap()["stage"],
            "none"
        );
        assert!(matches!(
            sponsor.prepare_sponsored_link(attempt, &offer, conversations::now()),
            Err(Error::Cancelled)
        ));
        sponsor.mobile_link("sponsor", None).unwrap();
        assert_eq!(
            sponsor.mobile_link("cancel", None).unwrap()["stage"],
            "none"
        );
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    #[test]
    fn cancelling_an_unapproved_link_allows_ordinary_sign_in() {
        let (dir, fixture, invitation, _) = crate::connection::tests::setup();
        let mut joining = ClientStore::open(
            &dir.path().join("joining.db"),
            StorageKey::new(Secret32::from_bytes([19; 32])).unwrap(),
        )
        .unwrap();
        joining.mobile_link("join", None).unwrap();
        let offer = joining.mobile_link("status", None).unwrap();
        joining.db.execute_batch("CREATE TRIGGER fail_cancel BEFORE DELETE ON identity BEGIN SELECT RAISE(ABORT,'disk failure'); END;").unwrap();
        assert!(joining.mobile_link("cancel", None).is_err());
        assert_eq!(joining.mobile_link("status", None).unwrap(), offer);
        joining
            .db
            .execute_batch("DROP TRIGGER fail_cancel;")
            .unwrap();
        joining.mobile_link("cancel", None).unwrap();
        crate::connection::tests::prepare(&mut joining, &fixture, &invitation.secret);
        joining.enroll_online().unwrap();
    }
}
