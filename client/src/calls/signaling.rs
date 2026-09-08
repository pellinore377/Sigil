use super::*;
use sigil_calls::{Answer, Connect, Layout, Relay, RelayRequest, SignedConnect};
impl ClientStore {
    /// The adapter creates a fresh peer connection for each reconnect attempt.
    pub fn prepare_call_connection(
        &mut self,
        id: Id,
        sdp: String,
        layout: Layout,
        now: u64,
    ) -> Result<SignedConnect, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = crate::conversations::time_floor(&tx, &self.key, now)?;
        let mut record = load(&tx, &self.key, &id)?;
        record.authorize(&tx, &self.key, now)?;
        if !record.commits.is_empty() {
            return Err(Error::Unprepared);
        }
        record.connect_sequence = record
            .connect_sequence
            .checked_add(1)
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or(Error::Limit)?;
        let proof = Connect {
            call: id,
            roster: record.state.roster.roster.digest().map_err(failure)?,
            participant: record.own_id()?,
            sequence: record.connect_sequence,
            sdp,
            layout,
        }
        .sign(&record.key(&self.key)?)
        .map_err(failure)?;
        proof
            .verify(&record.state.roster.roster, now)
            .map_err(failure)?;
        save(&tx, &self.key, &record)?;
        tx.commit()?;
        Ok(proof)
    }
    pub fn connect_call_online(
        &mut self,
        proof: &SignedConnect,
        now: u64,
    ) -> Result<Answer, Error> {
        let now = crate::conversations::time_floor(&self.db, &self.key, now)?;
        let record = load(&self.db, &self.key, &proof.request.call)?;
        record.authorize(&self.db, &self.key, now)?;
        if record.connect_sequence != proof.request.sequence
            || record.own_id()? != proof.request.participant
        {
            return Err(Error::Obsolete);
        }
        let answer = self
            .connected_client()?
            .connect_call(&record.state.roster, proof, now)?;
        let current = load(&self.db, &self.key, &proof.request.call)?;
        current.authorize(&self.db, &self.key, now)?;
        if current.connect_sequence != proof.request.sequence
            || current.state.roster.roster.digest().map_err(failure)? != proof.request.roster
        {
            return Err(Error::Obsolete);
        }
        Ok(answer)
    }
    pub fn call_relay_online(&mut self, id: Id, now: u64) -> Result<Option<Relay>, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = crate::conversations::time_floor(&tx, &self.key, now)?;
        let record = load(&tx, &self.key, &id)?;
        record.authorize(&tx, &self.key, now)?;
        let proof = RelayRequest::new(&record.state.roster.roster, &record.key(&self.key)?, now)
            .map_err(failure)?;
        tx.commit()?;
        let relay = self
            .connected_client()?
            .call_relay(&record.state.roster, &proof, now)?;
        let current = load(&self.db, &self.key, &id)?;
        current.authorize(&self.db, &self.key, now)?;
        if current.state.roster.roster.digest().map_err(failure)? != proof.roster {
            return Err(Error::Obsolete);
        }
        Ok(relay)
    }
    pub fn calls(&mut self, now: u64) -> Result<Vec<Call>, Error> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = crate::conversations::time_floor(&tx, &self.key, now)?;
        let rows: Vec<Vec<u8>> = tx
            .prepare("SELECT id FROM calls ORDER BY id LIMIT 257")?
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        if rows.len() > 256 {
            return Err(Error::Limit);
        }
        let mut values = Vec::new();
        for row in rows {
            let index: Id = row.try_into().map_err(|_| Error::InvalidStore)?;
            let mut record = load_index(&tx, &self.key, &index)?;
            if record.expire(now) {
                save(&tx, &self.key, &record)?;
            }
            if now >= record.state.roster.roster.expires {
                record.finish(Phase::Ended);
                save(&tx, &self.key, &record)?;
            } else {
                values.push(record.view(&tx, &self.key)?);
            }
        }
        tx.commit()?;
        Ok(values)
    }
}
