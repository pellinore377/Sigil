//! A page: metadata and one delta body.
use crate::delta::{Component, Delta, Insert, MAX_REPLAY};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

/// Longest title in scalars.
pub const MAX_TITLE: usize = 200;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Page,
    List,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meta {
    pub title: String,
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub archived: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Page {
    pub meta: Meta,
    pub body: Delta,
}
/// A loaded page must be one that replaying ops could have built.
impl<'de> Deserialize<'de> for Page {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            meta: Meta,
            body: Delta,
        }
        let Raw { meta, body } = Raw::deserialize(d)?;
        let page = Page { meta, body };
        page.check().map_err(D::Error::custom)?;
        Ok(page)
    }
}
impl Page {
    /// Whether the page is canonical and within limits, as `apply` keeps it.
    pub fn check(&self) -> Result<(), &'static str> {
        let canonical = self
            .body
            .ops
            .iter()
            .cloned()
            .fold(Delta::new(), Delta::push);
        if !self.body.is_body() || canonical != self.body {
            return Err("body is not canonical");
        }
        if !embeds(&self.body).all(embed_ok) {
            return Err("invalid embed");
        }
        if !ids_unique(&self.body) {
            return Err("duplicate embed id");
        }
        if self.body.size() > MAX_REPLAY {
            return Err("document too large");
        }
        if self.meta.title.chars().count() > MAX_TITLE {
            return Err("title too long");
        }
        Ok(())
    }
    /// An empty page: one empty paragraph.
    pub fn new(kind: Kind, title: &str) -> Self {
        Page {
            meta: Meta {
                title: title.into(),
                kind,
                archived: false,
            },
            body: Delta::new().insert("\n"),
        }
    }
}

/// Embed kinds addressed by id.
pub const KEYED: [&str; 2] = ["table", "block"];

/// Whether an embed is one kind, a table or block having exactly its fields.
pub fn embed_ok(value: &Value) -> bool {
    let Some(object) = value.as_object().filter(|o| o.len() == 1) else {
        return false;
    };
    match object.iter().next() {
        Some((kind, v)) if kind == "table" => table_ok(v),
        Some((kind, v)) if kind == "block" => v.as_object().is_some_and(|b| {
            b.len() == 2
                && b.get("id").is_some_and(Value::is_string)
                && b.get("source").is_some_and(Value::is_string)
        }),
        _ => true,
    }
}
/// `{ id, widths, rows }` with numeric widths and every row one canonical cell per width.
fn table_ok(v: &Value) -> bool {
    let Some(t) = v.as_object().filter(|t| t.len() == 3) else {
        return false;
    };
    let (Some(Value::String(_)), Some(Value::Array(widths)), Some(Value::Array(rows))) =
        (t.get("id"), t.get("widths"), t.get("rows"))
    else {
        return false;
    };
    widths.iter().all(Value::is_number)
        && rows.iter().all(|r| {
            r.as_array()
                .is_some_and(|r| r.len() == widths.len() && r.iter().all(cell_ok))
        })
}
/// A cell is a body without keyed embeds, stored as `push` would build it.
fn cell_ok(v: &Value) -> bool {
    let Ok(cell) = serde_json::from_value::<Delta>(v.clone()) else {
        return false;
    };
    let canonical = cell.ops.iter().cloned().fold(Delta::new(), Delta::push);
    cell.is_body()
        && !inserts_keyed(&cell)
        && embeds(&cell).all(embed_ok)
        && serde_json::to_value(canonical).ok().as_ref() == Some(v)
}
/// The `(kind, id)` of a keyed embed.
pub fn embed_key(value: &Value) -> Option<(&'static str, &str)> {
    let object = value.as_object().filter(|o| o.len() == 1)?;
    KEYED
        .iter()
        .find_map(|kind| Some((*kind, object.get(*kind)?.get("id")?.as_str()?)))
}
/// The embeds a delta inserts.
pub fn embeds(delta: &Delta) -> impl Iterator<Item = &Value> {
    delta.ops.iter().filter_map(|p| match p {
        Component::Insert {
            insert: Insert::Embed(e),
            ..
        } => Some(e),
        _ => None,
    })
}
/// Whether a delta inserts a keyed embed.
pub fn inserts_keyed(delta: &Delta) -> bool {
    embeds(delta).any(|e| embed_key(e).is_some())
}
/// Whether a delta inserts the embed of this kind and id.
pub fn inserts_embed(delta: &Delta, kind: &str, id: &str) -> bool {
    embeds(delta).any(|e| embed_key(e) == Some((kind, id)))
}
/// Whether every keyed embed in a body has its own id.
pub fn ids_unique(body: &Delta) -> bool {
    let mut seen = BTreeSet::new();
    embeds(body).all(|e| embed_key(e).is_none_or(|key| seen.insert(key)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn a_new_page_is_one_empty_paragraph_and_serializes_compactly() {
        let page = Page::new(Kind::List, "Groceries");
        assert_eq!(page.body.text(), "\n");
        let v = serde_json::to_value(&page).unwrap();
        assert_eq!(
            v,
            json!({ "meta": { "title": "Groceries", "kind": "list" }, "body": [{ "insert": "\n" }] })
        );
        assert_eq!(serde_json::from_value::<Page>(v).unwrap(), page);
        assert!(serde_json::from_value::<Page>(
            json!({ "meta": { "title": "", "kind": "page", "tabs": [] }, "body": [] })
        )
        .is_err());
    }
    #[test]
    fn a_loaded_page_is_canonical_and_valid() {
        let load = |body: Value| {
            serde_json::from_value::<Page>(
                json!({ "meta": { "title": "T", "kind": "page" }, "body": body }),
            )
        };
        let page = load(json!([{ "insert": "a" }, { "insert": "" }, { "insert": "b\n", "attrs": { "bold": null } }])).unwrap();
        assert_eq!(page.body, Delta::new().insert("ab\n"));
        let block = json!({ "insert": { "block": { "id": "b", "source": "" } } });
        for bad in [
            json!([{ "retain": 1 }]),
            json!([{ "delete": 1 }]),
            json!([block, block]),
            json!([{ "insert": 7 }]),
            json!([{ "insert": { "table": { "id": "t" } } }]),
            json!([{ "insert": "x".repeat(crate::delta::MAX_REPLAY + 1) }]),
        ] {
            assert!(load(bad.clone()).is_err(), "{bad}");
        }
        let long =
            json!({ "meta": { "title": "x".repeat(MAX_TITLE + 1), "kind": "page" }, "body": [] });
        assert!(serde_json::from_value::<Page>(long).is_err());
        let mut odd = Page::new(Kind::Page, "T");
        odd.body.ops.push(Component::Insert {
            insert: Insert::Text("x".into()),
            attrs: Default::default(),
        });
        assert_eq!(odd.check(), Err("body is not canonical"));
    }
    #[test]
    fn keyed_embeds_are_found_by_kind_and_id() {
        let table = json!({ "table": { "id": "t", "rows": [], "widths": [] } });
        let block = json!({ "block": { "id": "t", "source": "" } });
        let body = Delta::new()
            .embed(table.clone(), Default::default())
            .embed(block.clone(), Default::default())
            .embed(json!({ "hr": true }), Default::default());
        assert_eq!(embed_key(&table), Some(("table", "t")));
        assert_eq!(embed_key(&json!({ "hr": true })), None);
        assert!(
            inserts_keyed(&body)
                && inserts_embed(&body, "block", "t")
                && !inserts_embed(&body, "block", "u")
        );
        assert!(ids_unique(&body), "one id may name a table and a block");
        assert!(!ids_unique(&body.clone().embed(table, Default::default())));
        let both = json!({ "table": { "id": "x" }, "block": { "id": "x" } });
        assert!(!embed_ok(&both) && embed_key(&both).is_none() && !embed_ok(&json!("hr")));
    }
    #[test]
    fn tables_and_blocks_must_be_well_formed() {
        let table =
            |rows: Value| json!({ "table": { "id": "t", "widths": [1.0, 2], "rows": rows } });
        assert!(embed_ok(&table(
            json!([[[], [{ "insert": "a", "attrs": { "bold": true } }]]])
        )));
        assert!(
            embed_ok(&json!({ "hr": true }))
                && embed_ok(&json!({ "block": { "id": "b", "source": "" } }))
        );
        for bad in [
            table(json!([[[]]])),
            table(json!([[[], [{ "insert": "a" }, { "insert": "b" }]]])),
            table(json!([[[], [{ "retain": 1 }]]])),
            table(json!([[[], [{ "insert": "a", "attrs": { "bold": null } }]]])),
            table(json!([[[], [{ "insert": { "block": { "id": "b", "source": "" } } }]]])),
            json!({ "table": { "id": "t", "widths": ["x"], "rows": [] } }),
            json!({ "table": { "id": "t", "widths": [], "rows": [], "fill": [] } }),
            json!({ "block": { "id": "b" } }),
            json!({ "block": { "id": 1, "source": "" } }),
        ] {
            assert!(!embed_ok(&bad), "{bad}");
        }
    }
}
