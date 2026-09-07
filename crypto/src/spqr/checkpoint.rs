use super::*;
use crate::checkpoint::{Reader, Writer};

fn write_chain(chain: &Option<Chain>, out: &mut Writer) -> Result<(), Error> {
    out.u8(u8::from(chain.is_some()))?;
    if let Some(chain) = chain {
        out.u32(chain.n)?;
        out.put(chain.key.0.as_ref())?;
    }
    Ok(())
}
fn read_chain(input: &mut Reader<'_>) -> Result<Option<Chain>, Error> {
    match input.u8()? {
        0 => Ok(None),
        1 => Ok(Some(Chain {
            n: input.u32()?,
            key: input.secret()?,
        })),
        _ => Err(Error::Encoding),
    }
}

impl Ratchet {
    pub(crate) fn write(&self, out: &mut Writer) -> Result<(), Error> {
        out.put(self.root.0.as_ref())?;
        out.u8(u8::from(self.bob))?;
        for epoch in [self.epoch, self.sending_epoch, self.receiving_epoch] {
            out.u64(epoch)?;
        }
        out.u32(self.previous)?;
        out.u32(self.received_previous)?;
        out.u8(self.chains.len() as u8)?;
        for (epoch, chains) in &self.chains {
            out.u64(*epoch)?;
            write_chain(&chains.send, out)?;
            write_chain(&chains.receive, out)?;
        }
        out.u32(self.skipped.len() as u32)?;
        for ((epoch, number), key) in self.skipped.iter() {
            out.u64(*epoch)?;
            out.u32(*number)?;
            out.put(key.0 .0.as_ref())?;
        }
        self.scka.write(out)
    }

    pub(crate) fn read(input: &mut Reader<'_>) -> Result<Self, Error> {
        let root = input.secret()?;
        let bob = match input.u8()? {
            0 => false,
            1 => true,
            _ => return Err(Error::Encoding),
        };
        let epoch = input.u64()?;
        let sending_epoch = input.u64()?;
        let receiving_epoch = input.u64()?;
        let previous = input.u32()?;
        let received_previous = input.u32()?;
        if epoch.checked_sub(sending_epoch).is_none_or(|gap| gap > 1)
            || epoch.checked_sub(receiving_epoch).is_none_or(|gap| gap > 1)
            || (sending_epoch == 0 && previous != 0)
            || (receiving_epoch == 0 && received_previous != 0)
        {
            return Err(Error::State);
        }
        let count = input.u8()?;
        if count == 0 || count > 3 {
            return Err(Error::Limit);
        }
        let mut chains = BTreeMap::new();
        for _ in 0..count {
            let id = input.u64()?;
            let send = read_chain(input)?;
            let receive = read_chain(input)?;
            if id < sending_epoch.min(receiving_epoch)
                || id > epoch
                || send.is_some() != (id >= sending_epoch)
                || receive.is_some() != (id >= receiving_epoch)
                || chains.insert(id, Chains { send, receive }).is_some()
            {
                return Err(Error::State);
            }
        }
        for id in sending_epoch.min(receiving_epoch)..=epoch {
            if !chains.contains_key(&id) {
                return Err(Error::State);
            }
        }
        let count = input.u32()? as usize;
        if count > MAX_SKIPPED {
            return Err(Error::Limit);
        }
        let oldest = sending_epoch.max(receiving_epoch).saturating_sub(2);
        let mut skipped = Skipped::new();
        for _ in 0..count {
            let id = input.u64()?;
            let number = input.u32()?;
            let key = MessageKey(input.secret()?);
            if id < oldest
                || id > receiving_epoch
                || number == 0
                || (id == receiving_epoch
                    && number
                        >= chains
                            .get(&id)
                            .ok_or(Error::State)?
                            .receive
                            .as_ref()
                            .ok_or(Error::State)?
                            .n)
                || (id.checked_add(1) == Some(receiving_epoch) && number > received_previous)
                || skipped.insert((id, number), key).is_some()
            {
                return Err(Error::State);
            }
        }
        let scka = Scka::read(input)?;
        scka.validate_epoch(epoch, bob)?;
        Ok(Self {
            scka,
            root,
            bob,
            epoch,
            sending_epoch,
            receiving_epoch,
            previous,
            received_previous,
            chains,
            skipped,
        })
    }
}
