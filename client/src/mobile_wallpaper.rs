use super::*;

const LIMIT: usize = 2 * 1024 * 1024;
const CHUNK: usize = 64 * 1024;
const DOMAIN: &[u8] = b"Sigil/device-wallpaper/v1";

impl ClientStore {
    fn wallpaper_id(&mut self, peer: &str) -> Result<Id, Error> {
        let conversation = self.mobile_conversation(peer)?;
        self.key
            .commitment(
                &[self.connected_account_scope()?.as_slice(), &conversation].concat(),
                DOMAIN,
            )
            .map_err(Into::into)
    }
    /// Empty bytes remove the image. Wallpaper is device-local, never a message.
    pub fn mobile_set_wallpaper(&mut self, peer: &str, bytes: &[u8]) -> Result<(), Error> {
        if bytes.len() > LIMIT {
            return Err(Error::Limit);
        }
        let id = self.wallpaper_id(peer)?;
        let mut framed = Vec::new();
        if !bytes.is_empty() {
            let mut revision = [0; 32];
            getrandom::fill(&mut revision).map_err(|_| sigil_crypto::Error::Entropy)?;
            framed.extend_from_slice(&revision);
            framed.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
            let context = [DOMAIN, &id, framed.as_slice()].concat();
            for (index, chunk) in bytes.chunks(CHUNK).enumerate() {
                framed.extend_from_slice(&self.key.seal(
                    chunk,
                    &[context.as_slice(), &(index as u32).to_be_bytes()].concat(),
                )?);
            }
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let used: i64 = tx.query_row(
            "SELECT coalesce(sum(length(state)),0) FROM mobile_wallpapers WHERE id<>?1",
            [id.as_slice()],
            |r| r.get(0),
        )?;
        if !bytes.is_empty() && used.saturating_add(framed.len() as i64) > 64 * 1024 * 1024 {
            return Err(Error::Limit);
        }
        if bytes.is_empty() {
            tx.execute("DELETE FROM mobile_wallpapers WHERE id=?1", [id.as_slice()])?;
        } else {
            tx.execute("INSERT INTO mobile_wallpapers VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET state=excluded.state", (id.as_slice(), framed))?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn mobile_wallpaper(&mut self, peer: &str) -> Result<Option<Zeroizing<Vec<u8>>>, Error> {
        let id = self.wallpaper_id(peer)?;
        let framed: Option<Vec<u8>> = self.db.query_row("SELECT CASE WHEN length(state)<=2098344 THEN state END FROM mobile_wallpapers WHERE id=?1", [id.as_slice()], |r| r.get(0)).optional()?;
        let Some(framed) = framed else {
            return Ok(None);
        };
        let header = framed.get(..40).ok_or(Error::InvalidStore)?;
        let length = usize::try_from(u64::from_be_bytes(
            header[32..40].try_into().map_err(|_| Error::InvalidStore)?,
        ))
        .map_err(|_| Error::InvalidStore)?;
        if length == 0
            || length > LIMIT
            || framed.len() != 40 + length + length.div_ceil(CHUNK) * 36
        {
            return Err(Error::InvalidStore);
        }
        let context = [DOMAIN, &id, header].concat();
        let mut bytes = Zeroizing::new(Vec::with_capacity(length));
        for (index, chunk) in framed[40..].chunks(CHUNK + 36).enumerate() {
            bytes.extend_from_slice(&self.key.open(
                chunk,
                &[context.as_slice(), &(index as u32).to_be_bytes()].concat(),
            )?);
        }
        if bytes.len() != length {
            return Err(Error::InvalidStore);
        }
        Ok(Some(bytes))
    }
}
