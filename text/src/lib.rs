#![forbid(unsafe_code)]
//! Canonical SigilText domain data. Source text is never a serialized fallback.
pub mod action;
mod blocks;
mod card_parse;
pub mod code;
pub mod composition;
pub mod contact;
pub mod data;
pub mod editing;
mod effects;
pub mod help;
pub mod location;
pub mod math;
mod model;
pub mod motion;
pub mod numeric;
mod parse;
pub mod poll_close;
pub mod recurrence;
pub mod service;
pub mod structured;
pub mod time;
pub mod utility;
pub use blocks::{Block, BlockKind};
pub use card_parse::{item_id, parse_card, parse_card_with_dates, Draft, Hint, Origin, Parsed};
pub use effects::{Animation, Color, Effects, Hue, Paint, Reveal};
pub use model::{Error, Limits, Presentation, Run, Span, Text, MAX_WIRE_BYTES};
pub use parse::parse;
/// Validated rich event content; ordinary string messages never enter this decoder.
pub enum Document {
    Text(Text),
    Card(structured::Card),
    Action(action::Action),
    Composition(composition::Composition),
}
impl Document {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        match Text::from_bytes(bytes) {
            Ok(text) => Ok(Self::Text(text)),
            Err(Error::Limit) => Err(Error::Limit),
            Err(_) => match structured::Card::from_bytes(bytes) {
                Ok(card) => Ok(Self::Card(card)),
                Err(Error::Limit) => Err(Error::Limit),
                Err(_) => match composition::Composition::from_bytes(bytes) {
                    Ok(value) => Ok(Self::Composition(value)),
                    Err(Error::Limit) => Err(Error::Limit),
                    Err(_) => action::Action::from_bytes(bytes).map(Self::Action),
                },
            },
        }
    }
    pub fn authorize_origin(
        &self,
        message: &structured::Id,
        creator: &structured::Id,
        timestamp: u64,
    ) -> Result<(), Error> {
        match self {
            Self::Text(_) => Ok(()),
            Self::Card(value) => value.authorize_origin(message, creator, timestamp),
            Self::Composition(value) => value.authorize_origin(message, creator, timestamp),
            Self::Action(value) => value.authorize_origin(message, creator, timestamp),
        }
    }
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        match self {
            Self::Text(value) => value.to_bytes(),
            Self::Card(value) => value.to_bytes(),
            Self::Composition(value) => value.to_bytes(),
            Self::Action(value) => value.to_bytes(),
        }
    }
}
#[cfg(test)]
mod action_tests;
#[cfg(test)]
mod card_tests;
#[cfg(test)]
mod composition_tests;
#[cfg(test)]
mod contact_tests;
#[cfg(test)]
mod data_tests;
#[cfg(test)]
mod tests;
