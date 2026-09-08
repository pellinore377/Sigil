use crate::{
    action::{Action, Change},
    structured::{Card, Construct, Id},
    Error,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const PAGE_SIZE: usize = 64;
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Closure {
    #[serde(with = "crate::structured::id")]
    pub root: Id,
    pub voters: u64,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ballot {
    #[serde(with = "crate::structured::id")]
    pub actor: Id,
    #[serde(with = "crate::structured::id")]
    pub action: Id,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    #[serde(with = "crate::structured::id")]
    pub closure: Id,
    pub index: u64,
    pub ballots: Vec<Ballot>,
    #[serde(with = "crate::action::ids")]
    pub proof: Vec<Id>,
}
pub fn empty_root() -> Id {
    Sha256::digest(b"Sigil/poll-empty/v1").into()
}
pub fn branch(left: &Id, right: &Id) -> Id {
    Sha256::digest([b"Sigil/poll-branch/v1".as_slice(), left, right].concat()).into()
}
pub fn leaf(index: u64, ballots: &[Ballot]) -> Result<Id, Error> {
    if ballots.is_empty() || ballots.len() > PAGE_SIZE {
        return Err(Error::Limit);
    }
    let mut hash = Sha256::new();
    hash.update(b"Sigil/poll-page/v1");
    hash.update(index.to_be_bytes());
    hash.update((ballots.len() as u16).to_be_bytes());
    for ballot in ballots {
        hash.update(ballot.actor);
        hash.update(ballot.action);
    }
    Ok(hash.finalize().into())
}
impl Closure {
    pub fn pages(&self) -> u64 {
        self.voters.div_ceil(PAGE_SIZE as u64)
    }
    pub fn validate(&self) -> Result<(), Error> {
        if self.root == [0; 32]
            || self.voters > i64::MAX as u64
            || (self.voters == 0 && self.root != empty_root())
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
impl Page {
    pub fn validate(&self) -> Result<(), Error> {
        if self.closure == [0; 32]
            || self.index > i64::MAX as u64
            || self.proof.len() > 63
            || self.ballots.is_empty()
            || self.ballots.len() > PAGE_SIZE
            || self
                .ballots
                .iter()
                .any(|b| b.actor == [0; 32] || b.action == [0; 32])
            || self
                .ballots
                .windows(2)
                .any(|pair| pair[0].actor >= pair[1].actor)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub fn verify(&self, closure: &Closure) -> Result<(), Error> {
        self.validate()?;
        closure.validate()?;
        let mut width = closure.pages();
        let mut index = self.index;
        if index >= width {
            return Err(Error::Invalid);
        }
        let expected = (closure.voters - self.index * PAGE_SIZE as u64).min(PAGE_SIZE as u64);
        if self.ballots.len() as u64 != expected {
            return Err(Error::Invalid);
        }
        let mut root = leaf(index, &self.ballots)?;
        for sibling in &self.proof {
            if width <= 1 || (index ^ 1 >= width && sibling != &root) {
                return Err(Error::Invalid);
            }
            root = if index.is_multiple_of(2) {
                branch(&root, sibling)
            } else {
                branch(sibling, &root)
            };
            width = width.div_ceil(2);
            index /= 2;
        }
        if width != 1 || root != closure.root {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    /// Ballots are previously authenticated accepted actions, not sender-supplied assertions.
    pub fn validate_votes(
        &self,
        card: &Card,
        close: &Action,
        votes: &[&Action],
    ) -> Result<(), Error> {
        let Change::ClosePoll(closure) = &close.change else {
            return Err(Error::Invalid);
        };
        close.validate_for(card)?;
        if close.id()? != self.closure
            || votes.len() != self.ballots.len()
            || !matches!(card.content, Construct::Poll(_))
        {
            return Err(Error::Invalid);
        }
        self.verify(closure)?;
        for (entry, vote) in self.ballots.iter().zip(votes) {
            vote.validate_for(card)?;
            if vote.actor != entry.actor
                || vote.id()? != entry.action
                || !matches!(&vote.change, Change::Vote { choices } if !choices.is_empty())
            {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ballots(count: u8) -> Vec<Ballot> {
        (1..=count)
            .map(|id| Ballot {
                actor: [id; 32],
                action: [id + 1; 32],
            })
            .collect()
    }
    #[test]
    fn merkle_pages_bind_positions_cardinality_and_every_ballot() {
        let a = ballots(64);
        let b = ballots(64);
        let c = ballots(1);
        let leaves = [
            leaf(0, &a).unwrap(),
            leaf(1, &b).unwrap(),
            leaf(2, &c).unwrap(),
        ];
        let left = branch(&leaves[0], &leaves[1]);
        let right = branch(&leaves[2], &leaves[2]);
        let close = Closure {
            root: branch(&left, &right),
            voters: 129,
        };
        let page = Page {
            closure: [1; 32],
            index: 2,
            ballots: c,
            proof: vec![leaves[2], left],
        };
        page.verify(&close).unwrap();
        let mut changed = page.clone();
        changed.ballots[0].action = [99; 32];
        assert!(changed.verify(&close).is_err());
        let mut changed = page.clone();
        changed.index = 1;
        assert!(changed.verify(&close).is_err());
        let mut changed = page.clone();
        changed.proof.push([1; 32]);
        assert!(changed.verify(&close).is_err());
        let changed = Closure {
            voters: 130,
            ..close
        };
        assert!(page.verify(&changed).is_err());
        assert_eq!(
            Closure {
                root: empty_root(),
                voters: 0
            }
            .pages(),
            0
        );
    }
}
