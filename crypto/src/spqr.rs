//! Sparse message-key ratchet, with previous-chain sealing (spec §5.7).
//! Key methods mutate a private candidate; the Triple Ratchet owns AEAD commit.
use crate::skipped::Skipped;
use crate::{
    braid::{self, Scka},
    Error, MessageKey, Secret32,
};
use hkdf::Hkdf;
use sha2::Sha256;
use std::collections::BTreeMap;
use zeroize::Zeroizing;
mod checkpoint;

const INFO: &[u8] = b"Sigil/experimental/spqr/v0_MLKEM1024_SHA-256_RaptorQ64";
const MAX_SKIPPED: usize = crate::skipped::MAX;

pub(crate) struct Header {
    pub message: braid::Message,
    pub previous: u32,
    pub number: u32,
}

impl Header {
    pub fn to_bytes(&self) -> Vec<u8> {
        [
            &self.previous.to_be_bytes()[..],
            &self.number.to_be_bytes(),
            &self.message.to_bytes(),
        ]
        .concat()
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() != 17 && bytes.len() != 85 {
            return Err(Error::Encoding);
        }
        let previous = u32::from_be_bytes(bytes[..4].try_into().map_err(|_| Error::Encoding)?);
        let number = u32::from_be_bytes(bytes[4..8].try_into().map_err(|_| Error::Encoding)?);
        if number == 0 {
            return Err(Error::Encoding);
        }
        Ok(Self {
            previous,
            number,
            message: braid::Message::from_bytes(&bytes[8..])?,
        })
    }
}

struct Chain {
    key: Secret32,
    n: u32,
}
impl Chain {
    fn candidate(&self) -> Self {
        Self {
            key: Secret32::from_bytes(*self.key.0),
            n: self.n,
        }
    }
    fn next(&mut self) -> Result<MessageKey, Error> {
        let n = self.n.checked_add(1).ok_or(Error::Limit)?;
        let mut output = Zeroizing::new([0; 64]);
        // §5.2 requires the current chain key and counter. §7.2's `sk` is
        // undefined there and omits ctr; the concrete profile is documented.
        Hkdf::<Sha256>::new(Some(&[0; 32]), self.key.0.as_ref())
            .expand(
                &[INFO, b"Chain Step", &n.to_be_bytes()].concat(),
                output.as_mut(),
            )
            .map_err(|_| Error::State)?;
        self.key = Secret32::from_bytes(output[..32].try_into().map_err(|_| Error::State)?);
        self.n = n;
        Ok(MessageKey(Secret32::from_bytes(
            output[32..].try_into().map_err(|_| Error::State)?,
        )))
    }
}

struct Chains {
    send: Option<Chain>,
    receive: Option<Chain>,
}
impl Chains {
    fn candidate(&self) -> Self {
        Self {
            send: self.send.as_ref().map(Chain::candidate),
            receive: self.receive.as_ref().map(Chain::candidate),
        }
    }
}

pub(crate) struct Ratchet {
    scka: Scka,
    root: Secret32,
    bob: bool,
    epoch: u64,
    sending_epoch: u64,
    receiving_epoch: u64,
    previous: u32,
    received_previous: u32,
    chains: BTreeMap<u64, Chains>,
    skipped: Skipped<(u64, u32)>,
}

fn derive(
    root: &[u8; 32],
    key: &Secret32,
    label: &[u8],
    bob: bool,
) -> Result<(Secret32, Chains), Error> {
    let mut output = Zeroizing::new([0; 96]);
    Hkdf::<Sha256>::new(Some(root), key.0.as_ref())
        .expand(&[INFO, label].concat(), output.as_mut())
        .map_err(|_| Error::State)?;
    let root = Secret32::from_bytes(output[..32].try_into().map_err(|_| Error::State)?);
    let a = Chain {
        key: Secret32::from_bytes(output[32..64].try_into().map_err(|_| Error::State)?),
        n: 0,
    };
    let b = Chain {
        key: Secret32::from_bytes(output[64..].try_into().map_err(|_| Error::State)?),
        n: 0,
    };
    let (send, receive) = if bob { (b, a) } else { (a, b) };
    Ok((
        root,
        Chains {
            send: Some(send),
            receive: Some(receive),
        },
    ))
}

impl Ratchet {
    #[cfg(test)]
    pub(crate) fn status(&self) -> (u64, u64, u64, usize, usize) {
        (
            self.epoch,
            self.sending_epoch,
            self.receiving_epoch,
            self.chains.len(),
            self.skipped.len(),
        )
    }
    pub fn new(secret: Secret32, bob: bool) -> Result<Self, Error> {
        let scka = if bob {
            Scka::bob(&secret)?
        } else {
            Scka::alice(&secret)?
        };
        let (root, chains) = derive(&[0; 32], &secret, b"Chain Start", bob)?;
        Ok(Self {
            scka,
            root,
            bob,
            epoch: 0,
            sending_epoch: 0,
            receiving_epoch: 0,
            previous: 0,
            received_previous: 0,
            chains: BTreeMap::from([(0, chains)]),
            skipped: Skipped::new(),
        })
    }

    pub fn candidate(&self) -> Self {
        Self {
            scka: self.scka.candidate(),
            root: Secret32::from_bytes(*self.root.0),
            bob: self.bob,
            epoch: self.epoch,
            sending_epoch: self.sending_epoch,
            receiving_epoch: self.receiving_epoch,
            previous: self.previous,
            received_previous: self.received_previous,
            chains: self
                .chains
                .iter()
                .map(|(&id, chain)| (id, chain.candidate()))
                .collect(),
            skipped: self.skipped.candidate(),
        }
    }

    fn update(&mut self, output: Option<braid::Output>) -> Result<(), Error> {
        if let Some(output) = output {
            if self.epoch.checked_add(1) != Some(output.epoch) {
                return Err(Error::State);
            }
            let (root, chains) = derive(&self.root.0, &output.key, b"Chain Add Epoch", self.bob)?;
            self.chains.insert(output.epoch, chains);
            self.epoch = output.epoch;
            self.root = root;
        }
        Ok(())
    }

    fn prune(&mut self) -> Result<(), Error> {
        self.chains
            .retain(|_, chains| chains.send.is_some() || chains.receive.is_some());
        if self.chains.len() > 3 {
            return Err(Error::Limit);
        }
        // Bound delayed-message retention to this and the preceding two epochs.
        // The separate global count also bounds gaps within those epochs.
        let oldest = self
            .sending_epoch
            .max(self.receiving_epoch)
            .saturating_sub(2);
        self.skipped.retain(|(epoch, _)| *epoch >= oldest);
        Ok(())
    }

    pub fn send_key(&mut self) -> Result<(Header, MessageKey), Error> {
        let (message, epoch, output) = self.scka.send()?;
        self.update(output)?;
        if epoch != self.sending_epoch {
            if self.sending_epoch.checked_add(1) != Some(epoch) {
                return Err(Error::State);
            }
            let prior = self
                .chains
                .get_mut(&self.sending_epoch)
                .ok_or(Error::State)?
                .send
                .take()
                .ok_or(Error::State)?;
            self.previous = prior.n;
            self.sending_epoch = epoch;
        }
        let chain = self
            .chains
            .get_mut(&epoch)
            .ok_or(Error::State)?
            .send
            .as_mut()
            .ok_or(Error::State)?;
        let key = chain.next()?;
        let header = Header {
            message,
            previous: self.previous,
            number: chain.n,
        };
        self.prune()?;
        Ok((header, key))
    }

    fn skip_through(&mut self, epoch: u64, until: u32, work: &mut usize) -> Result<(), Error> {
        let chain = self
            .chains
            .get_mut(&epoch)
            .ok_or(Error::Replay)?
            .receive
            .as_mut()
            .ok_or(Error::Replay)?;
        let gap = until.checked_sub(chain.n).ok_or(Error::Replay)? as usize;
        if gap > MAX_SKIPPED || *work + gap > MAX_SKIPPED {
            return Err(Error::Limit);
        }
        *work += gap;
        for _ in 0..gap {
            let key = chain.next()?;
            self.skipped.insert((epoch, chain.n), key);
        }
        Ok(())
    }

    pub fn receive_key(&mut self, header: &Header) -> Result<MessageKey, Error> {
        let mut work = 0;
        if header.number == 0 {
            return Err(Error::Encoding);
        }
        let (epoch, output) = self.scka.receive(&header.message)?;
        self.update(output)?;
        if epoch > self.receiving_epoch {
            if self.receiving_epoch.checked_add(1) != Some(epoch) {
                return Err(Error::State);
            }
            self.skip_through(self.receiving_epoch, header.previous, &mut work)?;
            self.chains
                .get_mut(&self.receiving_epoch)
                .ok_or(Error::State)?
                .receive = None;
            self.receiving_epoch = epoch;
            self.received_previous = header.previous;
        }
        if epoch == self.receiving_epoch && header.previous != self.received_previous {
            return Err(Error::State);
        }
        self.prune()?;
        if let Some(key) = self.skipped.remove(&(epoch, header.number)) {
            return Ok(key);
        }
        // Counters start at one; cache messages strictly before the target.
        self.skip_through(epoch, header.number - 1, &mut work)?;
        self.chains
            .get_mut(&epoch)
            .ok_or(Error::Replay)?
            .receive
            .as_mut()
            .ok_or(Error::Replay)?
            .next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_and_counter_kdfs_match_openssl() {
        let expected = crate::vectors::ratchet_vector;
        for (salt, input, label, name) in [
            ([0; 32], [1; 32], b"Chain Start".as_slice(), "spqr_init"),
            (
                [3; 32],
                [2; 32],
                b"Chain Add Epoch".as_slice(),
                "spqr_epoch",
            ),
        ] {
            let (root, chains) = derive(&salt, &Secret32::from_bytes(input), label, false).unwrap();
            assert_eq!(
                [
                    root.0.as_slice(),
                    chains.send.as_ref().unwrap().key.0.as_slice(),
                    chains.receive.as_ref().unwrap().key.0.as_slice()
                ]
                .concat(),
                expected(name)
            );
            let (_, bob) = derive(&salt, &Secret32::from_bytes(input), label, true).unwrap();
            assert_eq!(*bob.send.unwrap().key.0, *chains.receive.unwrap().key.0);
            assert_eq!(*bob.receive.unwrap().key.0, *chains.send.unwrap().key.0);
        }
        let mut chain = Chain {
            key: Secret32::from_bytes([4; 32]),
            n: 0,
        };
        for name in ["spqr_step1", "spqr_step2"] {
            let key = chain.next().unwrap();
            assert_eq!(
                [chain.key.0.as_slice(), key.0 .0.as_slice()].concat(),
                expected(name)
            );
        }
    }

    fn pair() -> (Ratchet, Ratchet) {
        (
            Ratchet::new(Secret32::from_bytes([9; 32]), false).unwrap(),
            Ratchet::new(Secret32::from_bytes([9; 32]), true).unwrap(),
        )
    }

    #[test]
    fn counters_begin_at_one_and_gaps_do_not_derive_the_target_twice() {
        let (mut alice, mut bob) = pair();
        let (first, first_key) = alice.send_key().unwrap();
        assert_eq!(first.number, 1);
        let (second, second_key) = alice.send_key().unwrap();
        assert_eq!(second.number, 2);
        assert_eq!(*bob.receive_key(&second).unwrap().0 .0, *second_key.0 .0);
        assert_eq!(*bob.receive_key(&first).unwrap().0 .0, *first_key.0 .0);
        assert!(matches!(bob.receive_key(&first), Err(Error::Replay)));
        assert!(bob.skipped.is_empty());
    }

    #[test]
    fn skip_budget_and_counter_overflow_are_checked() {
        let (mut alice, bob) = pair();
        let (mut header, _) = alice.send_key().unwrap();
        header.number = MAX_SKIPPED as u32 + 2;
        assert!(matches!(
            bob.candidate().receive_key(&header),
            Err(Error::Limit)
        ));
        header.number = 0;
        assert!(matches!(
            bob.candidate().receive_key(&header),
            Err(Error::Encoding)
        ));
        alice.chains.get_mut(&0).unwrap().send.as_mut().unwrap().n = u32::MAX;
        assert!(matches!(alice.send_key(), Err(Error::Limit)));
    }

    #[test]
    fn previous_chain_sealing_retains_delayed_keys_and_erases_chain_keys() {
        let (mut alice, mut bob) = pair();
        let (delayed, key) = alice.send_key().unwrap();
        for _ in 0..200 {
            let (header, sent) = alice.send_key().unwrap();
            assert_eq!(*bob.receive_key(&header).unwrap().0 .0, *sent.0 .0);
            let (header, sent) = bob.send_key().unwrap();
            assert_eq!(*alice.receive_key(&header).unwrap().0 .0, *sent.0 .0);
            if bob.receiving_epoch == 1 {
                break;
            }
        }
        assert_eq!(bob.receiving_epoch, 1);
        assert!(bob
            .chains
            .get(&0)
            .is_none_or(|chains| chains.receive.is_none()));
        assert_eq!(*bob.receive_key(&delayed).unwrap().0 .0, *key.0 .0);
        assert!(matches!(bob.receive_key(&delayed), Err(Error::Replay)));
        for _ in 0..200 {
            let (header, sent) = alice.send_key().unwrap();
            assert_eq!(*bob.receive_key(&header).unwrap().0 .0, *sent.0 .0);
            let (header, sent) = bob.send_key().unwrap();
            assert_eq!(*alice.receive_key(&header).unwrap().0 .0, *sent.0 .0);
            assert!(alice.chains.len() <= 2 && bob.chains.len() <= 2);
        }
        assert!(alice.epoch >= 5 && bob.epoch >= 5);
    }
}
