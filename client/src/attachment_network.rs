use super::*;
use sigil_crypto::attachment::{Shape, CHUNK_OVERHEAD};
use sigil_protocol::attachments::{Begin, Parts, Publish, State, Status, MAX_PARTS_PAGE};
fn path(file: &[u8; 32]) -> String {
    format!("/client/v0/attachments/{}", crate::transport::hex(file))
}
fn validate(status: &Status, shape: Shape) -> Result<(), Error> {
    let chunks = shape.chunks().map_err(|_| Error::Configuration)?;
    if status.plaintext_bytes != shape.length
        || status.chunks != chunks
        || status.received_chunks > chunks
        || !valid_time(status.upload_deadline)
        || status.expires_at.is_some_and(|v| !valid_time(v))
        || status
            .root
            .as_ref()
            .is_some_and(|v| !accounts::valid_credential(v))
        || (status.state == State::Published
            && (status.root.is_none() || status.received_chunks != chunks))
        || (status.state == State::Uploading
            && (status.root.is_some() || status.restored_checkpoint))
    {
        return Err(Error::InvalidResponse);
    }
    Ok(())
}
impl HttpsClient {
    pub fn begin_attachment(&self, file: [u8; 32], request: &Begin) -> Result<Status, Error> {
        let shape = Shape {
            file,
            length: request.plaintext_bytes,
        };
        shape.chunks().map_err(|_| Error::Configuration)?;
        if !accounts::valid_credential(&request.access_token)
            || request.expires_at.is_some_and(|v| !valid_time(v))
        {
            return Err(Error::Configuration);
        }
        let status: Status = self.json(
            self.request(Method::PUT, &path(&file), Some(request))?,
            200,
            SMALL,
        )?;
        validate(&status, shape)?;
        if status.expires_at != request.expires_at {
            return Err(Error::InvalidResponse);
        }
        Ok(status)
    }
    pub fn attachment_status(&self, shape: Shape) -> Result<Status, Error> {
        shape.chunks().map_err(|_| Error::Configuration)?;
        let status: Status = self.json(
            self.request(Method::GET, &path(&shape.file), None::<&()>)?,
            200,
            SMALL,
        )?;
        validate(&status, shape)?;
        Ok(status)
    }
    pub fn put_attachment_chunk(
        &self,
        shape: Shape,
        index: u32,
        ciphertext: &[u8],
    ) -> Result<(), Error> {
        if ciphertext.len()
            != shape
                .chunk_length(index)
                .map_err(|_| Error::Configuration)?
                + CHUNK_OVERHEAD
        {
            return Err(Error::Configuration);
        }
        self.empty(self.request_bytes(
            Method::PUT,
            &format!("{}/chunks/{index}", path(&shape.file)),
            ciphertext,
            Some("application/octet-stream"),
            None,
        )?)
    }
    /// Returns bounded ciphertext only; callers must authenticate before processing.
    pub fn attachment_chunk(
        &self,
        shape: Shape,
        index: u32,
        access: &str,
    ) -> Result<Zeroizing<Vec<u8>>, Error> {
        let length = shape
            .chunk_length(index)
            .map_err(|_| Error::Configuration)?
            + CHUNK_OVERHEAD;
        let bytes = self.response_bytes(
            self.request_bytes(
                Method::GET,
                &format!("{}/chunks/{index}", path(&shape.file)),
                &[],
                None,
                Some(access),
            )?,
            200,
            length,
            "application/octet-stream",
        )?;
        if bytes.len() != length {
            return Err(Error::InvalidResponse);
        }
        Ok(bytes)
    }
    pub fn publish_attachment(&self, shape: Shape, request: &Publish) -> Result<Status, Error> {
        shape.chunks().map_err(|_| Error::Configuration)?;
        if !accounts::valid_credential(&request.root) {
            return Err(Error::Configuration);
        }
        let status: Status = self.json(
            self.request(
                Method::POST,
                &format!("{}/publish", path(&shape.file)),
                Some(request),
            )?,
            200,
            SMALL,
        )?;
        validate(&status, shape)?;
        if status.state != State::Published
            || status.restored_checkpoint
            || status.root.as_ref() != Some(&request.root)
        {
            return Err(Error::InvalidResponse);
        }
        Ok(status)
    }
    pub fn attachment_parts(&self, shape: Shape, after: Option<u32>) -> Result<Parts, Error> {
        let chunks = shape.chunks().map_err(|_| Error::Configuration)?;
        let path = match after {
            None => format!("{}/parts", path(&shape.file)),
            Some(after) => format!("{}/parts/{after}", path(&shape.file)),
        };
        let parts: Parts = self.json(self.request(Method::GET, &path, None::<&()>)?, 200, SMALL)?;
        if parts.chunks.len() > MAX_PARTS_PAGE
            || parts.chunks.windows(2).any(|v| v[0].index >= v[1].index)
            || parts.chunks.iter().any(|v| {
                v.index >= chunks
                    || after.is_some_and(|a| v.index <= a)
                    || !accounts::valid_credential(&v.hash)
            })
            || parts.next_after
                != if parts.chunks.len() == MAX_PARTS_PAGE {
                    parts.chunks.last().map(|v| v.index)
                } else {
                    None
                }
        {
            return Err(Error::InvalidResponse);
        }
        Ok(parts)
    }
    pub fn remove_attachment(&self, file: [u8; 32]) -> Result<(), Error> {
        self.empty(self.request(Method::DELETE, &path(&file), None::<&()>)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::tests::{Fixture, CA};
    use axum::{body::Body as ServerBody, http::Response as ServerResponse, routing::get, Router};
    use sigil_crypto::attachment::{CiphertextList, FileKey, CHUNK_SIZE};

    #[test]
    fn real_https_transfers_full_binary_chunks_and_preserves_access_boundaries() {
        let (_dir, _fixture, alice, bob, _now) = crate::claims::tests::pair();
        let sender = alice.connected_client().unwrap();
        let recipient = bob.connected_client().unwrap();
        let key = FileKey::generate(CHUNK_SIZE as u64 + 5).unwrap();
        let shape = key.shape();
        let begin = Begin {
            plaintext_bytes: shape.length,
            access_token: sigil_server::auth::random_secret().unwrap(),
            expires_at: None,
        };
        let status = sender.begin_attachment(shape.file, &begin).unwrap();
        assert_eq!(status.state, State::Uploading);
        let first = key.seal_chunk(0, &vec![1; CHUNK_SIZE]).unwrap();
        let final_part = key.seal_chunk(1, b"final").unwrap();
        sender.put_attachment_chunk(shape, 1, &final_part).unwrap();
        sender.put_attachment_chunk(shape, 0, &first).unwrap();
        sender.put_attachment_chunk(shape, 0, &first).unwrap();
        assert_eq!(
            sender.attachment_parts(shape, None).unwrap().chunks.len(),
            2
        );
        assert!(recipient
            .attachment_chunk(shape, 0, &begin.access_token)
            .is_err());
        let mut list = CiphertextList::new(shape).unwrap();
        list.push(0, &first).unwrap();
        list.push(1, &final_part).unwrap();
        let request = Publish {
            root: crate::transport::hex(&list.finish().unwrap()),
            acknowledge_restored_checkpoint: false,
        };
        sender.publish_attachment(shape, &request).unwrap();
        assert_eq!(
            sender.attachment_status(shape).unwrap().state,
            State::Published
        );
        assert_eq!(
            recipient
                .attachment_chunk(shape, 0, &begin.access_token)
                .unwrap()
                .as_slice(),
            first
        );
        assert_eq!(
            key.open_chunk(
                1,
                &recipient
                    .attachment_chunk(shape, 1, &begin.access_token)
                    .unwrap()
            )
            .unwrap()
            .as_slice(),
            b"final"
        );
        assert!(matches!(
            recipient.attachment_chunk(shape, 0, &"00".repeat(32)),
            Err(Error::Status { code: 404, .. })
        ));
        assert!(matches!(
            recipient.remove_attachment(shape.file),
            Err(Error::Status { code: 404, .. })
        ));
        sender.remove_attachment(shape.file).unwrap();
        assert!(matches!(
            recipient.attachment_chunk(shape, 0, &begin.access_token),
            Err(Error::Status { code: 404, .. })
        ));
    }

    #[test]
    fn binary_responses_reject_wrong_types_compression_truncation_and_oversize() {
        let shape = Shape {
            file: [1; 32],
            length: 0,
        };
        for (kind, encoding, length) in [
            ("application/json", "identity", 84),
            ("application/octet-stream", "gzip", 84),
            ("application/octet-stream", "identity", 83),
            ("application/octet-stream", "identity", 85),
        ] {
            let app = Router::new().route(
                "/client/v0/attachments/{id}/chunks/{index}",
                get(move || async move {
                    ServerResponse::builder()
                        .status(200)
                        .header("content-type", kind)
                        .header("content-encoding", encoding)
                        .body(ServerBody::from(vec![0; length]))
                        .unwrap()
                }),
            );
            let fixture = Fixture::new(app);
            let client = HttpsClient::new(
                "chat.example",
                fixture.port(),
                &"ab".repeat(32),
                &[CA.to_vec()],
            )
            .unwrap();
            assert!(client.attachment_chunk(shape, 0, &"cd".repeat(32)).is_err());
        }
        let app = Router::new().route(
            "/client/v0/attachments/{id}/chunks/{index}",
            get(|| async {
                ServerResponse::builder()
                    .status(429)
                    .header("retry-after", "12")
                    .body(ServerBody::empty())
                    .unwrap()
            }),
        );
        let fixture = Fixture::new(app);
        let client = HttpsClient::new(
            "chat.example",
            fixture.port(),
            &"ab".repeat(32),
            &[CA.to_vec()],
        )
        .unwrap();
        assert_eq!(
            client.attachment_chunk(shape, 0, &"cd".repeat(32)).err(),
            Some(Error::Status {
                code: 429,
                retry_after_seconds: Some(12)
            })
        );
    }

    #[test]
    fn status_cannot_change_shape_or_claim_incomplete_publication() {
        let shape = Shape {
            file: [1; 32],
            length: 5,
        };
        let status = Status {
            state: State::Uploading,
            plaintext_bytes: 5,
            chunks: 1,
            received_chunks: 0,
            upload_deadline: 100,
            expires_at: None,
            root: None,
            restored_checkpoint: false,
        };
        assert!(validate(&status, shape).is_ok());
        let mut bad = status.clone();
        bad.plaintext_bytes = 6;
        assert!(validate(&bad, shape).is_err());
        let mut bad = status.clone();
        bad.chunks = 2;
        assert!(validate(&bad, shape).is_err());
        let mut bad = status.clone();
        bad.received_chunks = 2;
        assert!(validate(&bad, shape).is_err());
        let mut bad = status.clone();
        bad.state = State::Published;
        bad.root = Some("ab".repeat(32));
        assert!(validate(&bad, shape).is_err());
        let mut bad = status.clone();
        bad.restored_checkpoint = true;
        assert!(validate(&bad, shape).is_err());
        let mut bad = status;
        bad.root = Some("ab".repeat(32));
        assert!(validate(&bad, shape).is_err());
    }
}
