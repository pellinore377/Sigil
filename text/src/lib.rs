#![forbid(unsafe_code)]
//! Canonical SigilText domain data. Source text is never a serialized fallback.
pub mod action;
mod card_parse;
mod effects;
mod model;
mod parse;
pub mod recurrence;
pub mod structured;
pub use card_parse::{item_id, parse_card, Draft, Hint, Origin, Parsed};
pub use effects::{Animation, Color, Effects, Hue, Paint, Reveal};
pub use model::{Error, Limits, Run, Span, Text, MAX_WIRE_BYTES};
pub use parse::parse;
/// Validated rich event content; ordinary string messages never enter this decoder.
pub enum Document {
    Text(Text),
    Card(structured::Card),
    Action(action::Action),
}
impl Document {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        match Text::from_bytes(bytes) {
            Ok(text) => Ok(Self::Text(text)),
            Err(Error::Limit) => Err(Error::Limit),
            Err(_) => match structured::Card::from_bytes(bytes) {
                Ok(card) => Ok(Self::Card(card)),
                Err(Error::Limit) => Err(Error::Limit),
                Err(_) => action::Action::from_bytes(bytes).map(Self::Action),
            },
        }
    }
}
#[cfg(test)]
mod action_tests;
#[cfg(test)]
mod card_tests;
#[cfg(test)]
mod tests;
