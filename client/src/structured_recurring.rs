use super::*;
use sigil_protocol::text::{recurrence::Period, structured::ListMode};

pub struct RecurringState {
    pub card: Card,
    pub definition: CardDefinition,
    pub as_of: u64,
    pub period: Period,
    /// Only current items: creation's one-offs disappear after the first reset.
    pub checks: Vec<CheckState>,
}
impl ClientStore {
    /// Derive the current recurring state from a trusted caller clock and the
    /// creation's fixed timezone data. No reset packet or database sweep is needed.
    pub fn recurring_state(
        &self,
        conversation: Id,
        reference: Reference,
        now: u64,
    ) -> Result<RecurringState, Error> {
        observe_card_expiry(&self.db, &self.key, conversation, &reference, now)?;
        let tx = self.db.unchecked_transaction()?;
        let (scope, _) = account_context(&tx, &self.key)?;
        let (index, card) = visible_card(&tx, &self.key, &scope, &conversation, &reference)?;
        let definition = definition(&tx, &self.key, &index, &card)?;
        let Construct::Checklist(list) = &definition.content else {
            return Err(Error::InvalidEvent);
        };
        let ListMode::Recurring(rule) = &list.mode else {
            return Err(Error::InvalidEvent);
        };
        let period = invalid(rule.period_at(now))?;
        let initial = period.start == rule.anchor_at;
        let mut checks = Vec::new();
        for item in &list.items {
            if !initial && !item.persistent {
                continue;
            }
            let register = register_index(
                &self.key,
                &index,
                Register::recurring(item.id, period.start),
            )?;
            let prior = head(&tx, &self.key, &index, &register)?;
            let (checked, operation, actor) = if let Some(prior) = prior {
                if prior.action.change
                    != (Change::RecurringCheck {
                        item: item.id,
                        period: period.start,
                    })
                    || prior.depth != 1
                {
                    return Err(Error::InvalidStore);
                }
                (
                    true,
                    Some(invalid(prior.action.id())?),
                    Some(prior.action.actor),
                )
            } else {
                let checked = initial && item.checked;
                (checked, None, checked.then_some(card.creator))
            };
            checks.push(CheckState {
                item: item.id,
                checked,
                operation,
                actor,
            });
        }
        tx.commit()?;
        Ok(RecurringState {
            card,
            definition,
            as_of: now,
            period,
            checks,
        })
    }
}
