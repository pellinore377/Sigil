use super::*;
use sigil_protocol::{
    services::{Catalog, Query, Resolved},
    text::{
        self,
        composition::{Composition, Part},
        Document, Text,
    },
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ServiceForm {
    kind: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    language: String,
    place: Option<sigil_protocol::services::Place>,
    #[serde(default)]
    forecast: bool,
}
impl ClientStore {
    pub(super) fn mobile_service(
        &mut self,
        action: &str,
        request: Option<String>,
        catalog: Option<Catalog>,
        provider: Option<String>,
        form: Option<ServiceForm>,
    ) -> Result<Value, Error> {
        if action == "catalog" {
            return Ok(json!({"catalog":self.connected_client()?.service_catalog()?}));
        }
        let request = id(&request.ok_or(Error::InvalidEvent)?)?;
        if action == "discard" {
            self.discard_service_query(request)?;
            return Ok(json!({}));
        }
        if action != "resolve" {
            return Err(Error::InvalidEvent);
        }
        let form = form.ok_or(Error::InvalidEvent)?;
        let query = match form.kind.as_str() {
            "Translation" => Query::Translate {
                text: Text::plain(&form.text, Default::default())
                    .map_err(|_| Error::InvalidEvent)?,
                source: None,
                target: form.language,
            },
            "Definition" => Query::Define {
                word: form.text,
                language: form.language,
            },
            "Locate" => Query::Locate {
                name: form.text,
                language: form.language,
            },
            "Weather" => Query::Weather {
                place: form.place.ok_or(Error::InvalidEvent)?,
                forecast: form.forecast,
            },
            _ => return Err(Error::InvalidEvent),
        };
        self.prepare_service_query(
            request,
            &catalog.ok_or(Error::InvalidEvent)?,
            &provider.ok_or(Error::InvalidEvent)?,
            query,
            false,
        )?;
        let result =
            self.resolve_service_query(request, &std::sync::atomic::AtomicBool::new(false))?;
        Ok(match result {
            Resolved::Snapshot(snapshot) => {
                json!({"request":transport::hex(&request),"preview":{"id":"preview","kind":"service","text":"","service":snapshot.presentation(conversations::now()).map_err(|_|Error::InvalidEvent)?}})
            }
            Resolved::Places(places) => json!({"places":places}),
        })
    }
    pub(super) fn mobile_service_body(
        &self,
        query: &str,
        message: &str,
        created_at: u64,
        caption: &str,
    ) -> Result<Body, Error> {
        let message = id(message)?;
        let card = self.service_card(
            id(query)?,
            if caption.is_empty() {
                message
            } else {
                text::composition::card_id(&message, 0)
            },
            created_at,
        )?;
        shared_body(card, message, created_at, caption)
    }
    pub(super) fn mobile_contact_preview(
        &self,
        address: &str,
    ) -> Result<text::contact::Contact, Error> {
        let found = self.discover_account_online(address)?;
        let (username, server) = address
            .strip_prefix('@')
            .and_then(|s| s.split_once(':'))
            .ok_or(Error::InvalidEvent)?;
        if !found.valid_for(username, server) {
            return Err(Error::InvalidStore);
        }
        Ok(text::contact::Contact {
            user_id: crate::event::account_reference(server, &id(&found.account)?),
            address: address.into(),
            display_name: Text::plain(username, Default::default())
                .map_err(|_| Error::InvalidEvent)?,
            avatar_url: None,
        })
    }
    pub(super) fn mobile_shared_contact_body(
        &self,
        contact: &text::contact::Contact,
        message: &str,
        created_at: u64,
        caption: &str,
    ) -> Result<Body, Error> {
        let current = self.mobile_contact_preview(&contact.address)?;
        if current.user_id != contact.user_id {
            return Err(Error::SharedContactChanged);
        }
        let message = id(message)?;
        let card = text::structured::Card {
            id: if caption.is_empty() {
                message
            } else {
                text::composition::card_id(&message, 0)
            },
            creator: self.account_reference()?,
            created_at,
            content: text::structured::Construct::Contact(current),
        };
        shared_body(card, message, created_at, caption)
    }
}
fn shared_body(
    card: text::structured::Card,
    message: Id,
    created_at: u64,
    caption: &str,
) -> Result<Body, Error> {
    let document = if caption.is_empty() {
        Document::Card(card)
    } else {
        Document::Composition(Composition {
            id: message,
            creator: card.creator,
            created_at,
            parts: vec![
                Part::Card(Box::new(card)),
                Part::Text(
                    text::parse(caption, Default::default()).map_err(|_| Error::InvalidEvent)?,
                ),
            ],
        })
    };
    Ok(Body::Rich(
        document.to_bytes().map_err(|_| Error::InvalidEvent)?,
    ))
}
