use super::*;
use crate::storage::StorageKey;
use zeroize::Zeroizing;

const PREFIX: &[u8; 8] = b"SGRS\0\x02\0\0";
impl Session {
    /// Encrypted checkpoint only. Persist atomically with inbox/outbox changes.
    pub fn seal_checkpoint(&self, key: &StorageKey, binding: &[u8]) -> Result<Vec<u8>, Error> {
        key.seal(&self.checkpoint_bytes(), binding)
    }

    pub(crate) fn checkpoint_bytes(&self) -> Zeroizing<Vec<u8>> {
        // Avoid reallocating a buffer containing live secret state.
        let mut bytes = Zeroizing::new(Vec::with_capacity(9000));
        bytes.extend_from_slice(PREFIX);
        bytes.extend_from_slice(&self.context);
        bytes.extend_from_slice(&Zeroizing::new(self.step.local.0.to_bytes())[..]);
        bytes.extend_from_slice(self.step.root.0.as_ref());
        for value in [
            self.step.remote.as_ref().map(|v| v.as_slice()),
            self.step.send.as_ref().map(|v| v.0.as_ref()),
            self.step.receive.as_ref().map(|v| v.0.as_ref()),
        ] {
            bytes.push(u8::from(value.is_some()));
            if let Some(value) = value {
                bytes.extend_from_slice(value);
            }
        }
        for count in [
            self.step.sent,
            self.step.received,
            self.step.previous,
            self.skipped.len() as u32,
        ] {
            bytes.extend_from_slice(&count.to_be_bytes());
        }
        for ((dh, number), key) in self.skipped.iter() {
            bytes.extend_from_slice(dh);
            bytes.extend_from_slice(&number.to_be_bytes());
            bytes.extend_from_slice(key.0 .0.as_ref());
        }
        bytes
    }

    pub fn open_checkpoint(key: &StorageKey, sealed: &[u8], binding: &[u8]) -> Result<Self, Error> {
        let bytes = key.open(sealed, binding)?;
        Self::from_checkpoint_bytes(&bytes)
    }

    pub(crate) fn from_checkpoint_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let mut input = bytes;
        fn take<const N: usize>(input: &mut &[u8]) -> Result<[u8; N], Error> {
            if input.len() < N {
                return Err(Error::Encoding);
            }
            let (head, tail) = input.split_at(N);
            *input = tail;
            head.try_into().map_err(|_| Error::Encoding)
        }
        fn optional(input: &mut &[u8]) -> Result<Option<Zeroizing<[u8; 32]>>, Error> {
            match take::<1>(input)?[0] {
                0 => Ok(None),
                1 => Ok(Some(Zeroizing::new(take(input)?))),
                _ => Err(Error::Encoding),
            }
        }
        let prefix = take::<8>(&mut input)?;
        if &prefix != PREFIX && &prefix != b"SGRS\0\x01\0\0" {
            return Err(Error::Encoding);
        }
        let context = take(&mut input)?;
        let local = DhKey(x25519_dalek::StaticSecret::from(*Zeroizing::new(
            take::<32>(&mut input)?,
        )));
        let root = Secret32(Zeroizing::new(take(&mut input)?));
        let remote = optional(&mut input)?.map(|v| *v);
        if let Some(remote) = &remote {
            validate_public(remote)?;
        }
        let send = optional(&mut input)?.map(Secret32);
        let receive = optional(&mut input)?.map(Secret32);
        let sent = u32::from_be_bytes(take(&mut input)?);
        let received = u32::from_be_bytes(take(&mut input)?);
        let previous = u32::from_be_bytes(take(&mut input)?);
        let count = u32::from_be_bytes(take(&mut input)?) as usize;
        if count > MAX_SKIPPED_KEYS {
            return Err(Error::Limit);
        }
        if (remote.is_none()
            && (send.is_some()
                || receive.is_some()
                || sent != 0
                || received != 0
                || previous != 0
                || count != 0))
            || (remote.is_some() && send.is_none())
            || (receive.is_none() && received != 0)
        {
            return Err(Error::State);
        }
        let mut skipped = Skipped::new();
        for _ in 0..count {
            let dh = take(&mut input)?;
            validate_public(&dh)?;
            let number = u32::from_be_bytes(take(&mut input)?);
            let key = MessageKey(Secret32(Zeroizing::new(take(&mut input)?)));
            if skipped.insert((dh, number), key).is_some() {
                return Err(Error::Encoding);
            }
        }
        if !input.is_empty() {
            return Err(Error::Encoding);
        }
        Ok(Self {
            step: Step {
                local,
                remote,
                root,
                send,
                receive,
                sent,
                received,
                previous,
            },
            skipped,
            context,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_skipped_key_checkpoint_remains_usable_across_dh_turns() {
        let (mut alice, mut bob) = super::super::tests::pair();
        let mut packets = Vec::new();
        for _ in 0..=MAX_SKIPPED_KEYS {
            packets.push(alice.send(b"synthetic").unwrap());
        }
        bob.receive(packets.last().unwrap()).unwrap();
        let key = StorageKey::new(Secret32::from_bytes([9; 32])).unwrap();
        let sealed = bob.seal_checkpoint(&key, b"session/bob/1").unwrap();
        assert!(Session::open_checkpoint(&key, &sealed, b"session/alice/1").is_err());
        let mut bob = Session::open_checkpoint(&key, &sealed, b"session/bob/1").unwrap();
        assert_eq!(bob.skipped.len(), MAX_SKIPPED_KEYS);
        alice.receive(&bob.send(b"reply").unwrap()).unwrap();
        for packet in &packets[..MAX_SKIPPED_KEYS] {
            assert_eq!(bob.receive(packet).unwrap(), b"synthetic");
        }
        assert!(bob.skipped.is_empty());
        assert_eq!(
            bob.receive(&alice.send(b"next DH chain").unwrap()).unwrap(),
            b"next DH chain"
        );
    }
    #[test]
    fn malformed_checkpoint_payloads_fail_closed() {
        let (alice, _) = super::super::tests::pair();
        let key = StorageKey::new(Secret32::from_bytes([9; 32])).unwrap();
        let sealed = alice.seal_checkpoint(&key, b"row").unwrap();
        let bytes = key.open(&sealed, b"row").unwrap();
        for length in [0, 7, 40, 104, bytes.len() - 1] {
            assert!(Session::open_checkpoint(
                &key,
                &key.seal(&bytes[..length], b"row").unwrap(),
                b"row"
            )
            .is_err());
        }
        for index in [0, 4, 104] {
            let mut bad = Zeroizing::new(bytes.to_vec());
            bad[index] = 255;
            assert!(
                Session::open_checkpoint(&key, &key.seal(&bad, b"row").unwrap(), b"row").is_err()
            );
        }
        let mut bad = Zeroizing::new(bytes.to_vec());
        bad.push(0);
        assert!(Session::open_checkpoint(&key, &key.seal(&bad, b"row").unwrap(), b"row").is_err());
        let mut bad = Zeroizing::new(bytes.to_vec());
        let n = bad.len();
        bad[n - 4..].copy_from_slice(&129_u32.to_be_bytes());
        assert!(Session::open_checkpoint(&key, &key.seal(&bad, b"row").unwrap(), b"row").is_err());
    }
}
