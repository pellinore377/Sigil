//! vCard 3.0 and 4.0. The two versions spell the same things differently (`TYPE=CELL` vs
//! `TYPE=cell`, `ENCODING=b` vs `data:` URI). Input is untrusted, so everything is bounded
//! and a malformed card yields `None` rather than a panic.

use serde_json::{json, Value};

/// Refuse a file larger than this outright.
pub const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
/// Refuse a decoded photo larger than this and fall back to initials.
pub const MAX_PHOTO_BYTES: usize = 512 * 1024;
/// No more cards than this from one file.
pub const MAX_CARDS: usize = 50;
/// No more values than this in any one repeated field.
const MAX_VALUES: usize = 12;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Typed {
    /// "cell", "home", "work"… lower-cased, empty when unlabelled.
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Card {
    pub name: String,
    pub phones: Vec<Typed>,
    pub emails: Vec<Typed>,
    pub org: String,
    pub title: String,
    pub address: String,
    pub note: String,
    /// The raw `PHOTO` value, still encoded. Decoding is the caller's call.
    pub photo: String,
    /// `X-MATRIX-ID`, or an MXID recovered from `NOTE`.
    pub matrix_id: String,
}

impl Card {
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "phones": self.phones.iter().map(|t| json!({"kind": t.kind, "value": t.value})).collect::<Vec<_>>(),
            "emails": self.emails.iter().map(|t| json!({"kind": t.kind, "value": t.value})).collect::<Vec<_>>(),
            "org": self.org,
            "title": self.title,
            "address": self.address,
            "note": self.note,
            "matrixId": self.matrix_id,
            "hasPhoto": !self.photo.is_empty(),
        })
    }
}

/// Undo RFC 6350 line folding. Must run first, or a folded `PHOTO` splits.
fn unfold(src: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in src.split(['\r', '\n']).filter(|l| !l.is_empty()) {
        if (raw.starts_with(' ') || raw.starts_with('\t')) && !out.is_empty() {
            let last = out.last_mut().unwrap();
            // Exactly one character: trimming the lot would eat a folded name's spaces.
            last.push_str(&raw[1..]);
            continue
        }
        // Quoted-printable soft break. Gated on the encoding: a base64 PHOTO also ends in `=`.
        let soft = out.last().map(|l: &String| {
            l.ends_with('=') && l.to_ascii_uppercase().contains("QUOTED-PRINTABLE")
        }).unwrap_or(false);
        if soft {
            let last = out.last_mut().unwrap();
            last.pop();
            last.push_str(raw);
            continue
        }
        out.push(raw.to_string());
    }
    out
}

/// `=4A=65` → `Je`. Phone exports use quoted-printable constantly.
fn decode_qp(v: &str) -> String {
    let b = v.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'=' && i + 2 < b.len() {
            let hex = |c: u8| -> Option<u8> {
                match c {
                    b'0'..=b'9' => Some(c - b'0'),
                    b'A'..=b'F' => Some(c - b'A' + 10),
                    b'a'..=b'f' => Some(c - b'a' + 10),
                    _ => None,
                }
            };
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push(h << 4 | l);
                i += 3;
                continue
            }
        }
        out.push(b[i]);
        i += 1;
    }
    // Usually UTF-8; lossy rather than losing the whole field.
    String::from_utf8_lossy(&out).into_owned()
}

fn is_qp(params: &[String]) -> bool {
    params.iter().any(|p| p.contains("quoted-printable"))
}

/// `\n`, `\,`, `\;`, `\\` — the whole escaping table.
fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c != '\\' { out.push(c); continue }
        match chars.next() {
            Some('n') | Some('N') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

fn escape(v: &str) -> String {
    v.replace('\\', "\\\\").replace('\n', "\\n").replace(',', "\\,").replace(';', "\\;")
}

/// Split `NAME;PARAM=X;PARAM=Y:value` into its three parts.
fn split_line(line: &str) -> Option<(String, Vec<String>, String)> {
    // The colon that ends the property; a quoted parameter may contain one.
    let mut in_quotes = false;
    let mut colon = None;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => { colon = Some(i); break }
            _ => {}
        }
    }
    let colon = colon?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let mut parts = head.split(';');
    let name = parts.next()?.trim().to_ascii_uppercase();
    // A group prefix — `item1.TEL` — is a 3.0-ism and carries no meaning here.
    let name = name.rsplit('.').next().unwrap_or(&name).to_string();
    let params: Vec<String> = parts.map(|p| p.trim().to_ascii_lowercase()).collect();
    Some((name, params, value.to_string()))
}

/// The `TYPE=` label: 3.0 writes bare `;CELL`, 4.0 writes `;TYPE="voice,cell"`; both appear.
fn type_of(params: &[String]) -> String {
    for p in params {
        let v = p.strip_prefix("type=").unwrap_or(p);
        let v = v.trim_matches('"');
        for word in v.split(',') {
            match word {
                "cell" | "mobile" => return "cell".into(),
                "home" => return "home".into(),
                "work" => return "work".into(),
                "fax" => return "fax".into(),
                _ => {}
            }
        }
    }
    String::new()
}

fn push_capped(v: &mut Vec<Typed>, t: Typed) {
    if v.len() < MAX_VALUES && !t.value.is_empty() { v.push(t) }
}

/// Every card in one file. Empty means fall back to a plain file attachment.
pub fn parse(src: &str) -> Vec<Card> {
    if src.len() > MAX_FILE_BYTES { return Vec::new() }
    let mut cards = Vec::new();
    let mut cur: Option<Card> = None;

    for line in unfold(src) {
        let upper = line.trim().to_ascii_uppercase();
        if upper.starts_with("BEGIN:VCARD") {
            // A nested BEGIN is malformed; start a fresh card rather than lose the rest.
            if let Some(c) = cur.take() { if !c.is_empty() { cards.push(c) } }
            cur = Some(Card::default());
            continue
        }
        if upper.starts_with("END:VCARD") {
            if let Some(c) = cur.take() { if !c.is_empty() { cards.push(c) } }
            if cards.len() >= MAX_CARDS { break }
            continue
        }
        let Some(card) = cur.as_mut() else { continue };
        let Some((name, params, value)) = split_line(&line) else { continue };
        let value = if is_qp(&params) { decode_qp(&value) } else { value };
        let val = unescape(&value);
        match name.as_str() {
            "FN" => if !val.trim().is_empty() { card.name = val.trim().to_string() },
            "N" if card.name.is_empty() => {
                // family;given;middle;prefix;suffix — rendered given-first.
                let f: Vec<&str> = value.split(';').collect();
                let given = f.get(1).copied().unwrap_or("").trim();
                let family = f.first().copied().unwrap_or("").trim();
                let joined = format!("{given} {family}");
                let joined = joined.trim();
                if !joined.is_empty() { card.name = unescape(joined) }
            }
            "TEL" => push_capped(&mut card.phones, Typed { kind: type_of(&params), value: val.trim().to_string() }),
            "EMAIL" => push_capped(&mut card.emails, Typed { kind: type_of(&params), value: val.trim().to_string() }),
            "ORG" => if card.org.is_empty() { card.org = val.split(';').next().unwrap_or("").trim().to_string() },
            "TITLE" => if card.title.is_empty() { card.title = val.trim().to_string() },
            "ADR" => if card.address.is_empty() {
                // po;ext;street;locality;region;postcode;country
                let f: Vec<String> = value.split(';').map(|p| unescape(p).trim().to_string()).collect();
                let keep: Vec<String> = f.into_iter().skip(2).filter(|p| !p.is_empty()).collect();
                card.address = keep.join(", ");
            },
            "NOTE" => if card.note.is_empty() { card.note = val.trim().to_string() },
            "PHOTO" => if card.photo.is_empty() { card.photo = value.trim().to_string() },
            "X-MATRIX-ID" => card.matrix_id = val.trim().to_string(),
            _ => {}
        }
    }
    // The exporter writes the MXID twice, so NOTE may still carry it.
    for c in cards.iter_mut() {
        if c.matrix_id.is_empty() {
            if let Some(id) = find_mxid(&c.note) { c.matrix_id = id }
        }
    }
    cards
}

impl Card {
    fn is_empty(&self) -> bool {
        // A card with nothing but a note is not a contact.
        self.name.is_empty() && self.phones.is_empty() && self.emails.is_empty()
            && self.org.is_empty() && self.title.is_empty() && self.matrix_id.is_empty()
    }
}

/// The first `@user:server` in a blob of text.
fn find_mxid(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let at = s.find('@')?;
    let rest = &s[at..];
    let end = rest
        .char_indices()
        .find(|(_, c)| c.is_whitespace() || *c == ',' || *c == ';')
        .map(|(i, _)| i)
        .unwrap_or(rest.len());
    let cand = &rest[..end];
    let _ = bytes;
    if cand.contains(':') && cand.len() > 3 { Some(cand.to_string()) } else { None }
}

/// The inline photo's bytes, if not absurd. Both 3.0's `ENCODING=b` and 4.0's `data:`
/// URI appear. A URL is never fetched — that would be a request to a stranger's server.
pub fn photo_bytes(raw: &str) -> Option<Vec<u8>> {
    let payload = match raw.split_once("base64,") {
        Some((_, b)) => b,
        None => {
            if raw.starts_with("http://") || raw.starts_with("https://") { return None }
            raw
        }
    };
    let cleaned: String = payload.chars().filter(|c| !c.is_whitespace()).collect();
    // Base64 is 4 bytes per 3; refuse before decoding rather than after.
    if cleaned.len() / 4 * 3 > MAX_PHOTO_BYTES { return None }
    let out = base64_decode(&cleaned)?;
    if out.len() > MAX_PHOTO_BYTES { return None }
    Some(out)
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a') as u32 + 26,
            b'0'..=b'9' => (c - b'0') as u32 + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => return None,
            _ => return None,
        })
    }
    let bytes: Vec<u8> = s.bytes().filter(|b| *b != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let mut acc = 0u32;
        let mut n = 0;
        for b in chunk {
            acc = (acc << 6) | val(*b)?;
            n += 1;
        }
        if n < 2 { return None }
        acc <<= 6 * (4 - n);
        let bs = acc.to_be_bytes();
        out.push(bs[1]);
        if n >= 3 { out.push(bs[2]) }
        if n >= 4 { out.push(bs[3]) }
    }
    Some(out)
}

/// A vCard 4.0 for a Matrix contact: the MXID goes in **both** `X-MATRIX-ID` and `NOTE`.
pub fn to_vcf(display_name: &str, user_id: &str) -> String {
    let name = if display_name.trim().is_empty() { user_id } else { display_name };
    format!(
        "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:{}\r\nX-MATRIX-ID:{}\r\nNOTE:Matrix: {}\r\nEND:VCARD\r\n",
        escape(name.trim()),
        escape(user_id),
        escape(user_id),
    )
}

/// Every card in a file, as JSON for the view.
pub fn to_json(cards: &[Card]) -> Value {
    json!(cards.iter().map(Card::to_json).collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Alice Waters\r\nTEL;TYPE=CELL:+1 555 0100\r\n\
EMAIL;TYPE=work:alice@example.com\r\nORG:Example Corp;Ops\r\nTITLE:Engineer\r\nEND:VCARD\r\n";

    #[test]
    fn a_single_card_yields_its_fields() {
        let c = &parse(ONE)[0];
        assert_eq!(c.name, "Alice Waters");
        assert_eq!(c.phones[0].value, "+1 555 0100");
        assert_eq!(c.phones[0].kind, "cell");
        assert_eq!(c.emails[0].value, "alice@example.com");
        assert_eq!(c.emails[0].kind, "work");
        // ORG is `company;unit`; only the company is worth a line.
        assert_eq!(c.org, "Example Corp");
        assert_eq!(c.title, "Engineer");
    }

    #[test]
    fn several_cards_in_one_file_all_come_back() {
        let two = format!("{ONE}BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Sam\r\nEND:VCARD\r\n");
        let cards = parse(&two);
        assert_eq!(cards.len(), 2, "a .vcf may hold many");
        assert_eq!(cards[1].name, "Sam");
    }

    #[test]
    fn folded_lines_are_rejoined_before_anything_reads_them() {
        let folded = "BEGIN:VCARD\r\nFN:A Very Long\r\n  Name Indeed\r\nEND:VCARD";
        assert_eq!(parse(folded)[0].name, "A Very Long Name Indeed");
    }

    #[test]
    fn the_escaping_table() {
        let v = "BEGIN:VCARD\r\nFN:Doe\\, John\r\nNOTE:line one\\nline two\\; still\r\nEND:VCARD";
        let c = &parse(v)[0];
        assert_eq!(c.name, "Doe, John");
        assert_eq!(c.note, "line one\nline two; still");
    }

    #[test]
    fn n_is_used_when_fn_is_missing() {
        let v = "BEGIN:VCARD\r\nN:Waters;Alice;;;\r\nEND:VCARD";
        assert_eq!(parse(v)[0].name, "Alice Waters");
        let both = "BEGIN:VCARD\r\nFN:Alice W\r\nN:Waters;Alice;;;\r\nEND:VCARD";
        assert_eq!(parse(both)[0].name, "Alice W");
    }

    #[test]
    fn both_vcard_versions_spell_the_type_label_differently() {
        let v3 = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:A\r\nTEL;HOME:1\r\nEND:VCARD";
        let v4 = "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:A\r\nTEL;TYPE=\"voice,home\":1\r\nEND:VCARD";
        assert_eq!(parse(v3)[0].phones[0].kind, "home");
        assert_eq!(parse(v4)[0].phones[0].kind, "home");
    }

    #[test]
    fn a_group_prefix_does_not_hide_the_property() {
        // Apple writes `item1.TEL`, which is legal and means nothing here.
        let v = "BEGIN:VCARD\r\nFN:A\r\nitem1.TEL;TYPE=cell:+1 555 0111\r\nEND:VCARD";
        assert_eq!(parse(v)[0].phones[0].value, "+1 555 0111");
    }

    #[test]
    fn addresses_drop_the_post_office_box_fields() {
        let v = "BEGIN:VCARD\r\nFN:A\r\nADR;TYPE=home:;;1 Example Way;Springfield;IL;62701;USA\r\nEND:VCARD";
        assert_eq!(parse(v)[0].address, "1 Example Way, Springfield, IL, 62701, USA");
    }

    #[test]
    fn malformed_input_is_empty_rather_than_a_panic() {
        for bad in ["", "not a vcard at all", "BEGIN:VCARD", "END:VCARD",
                    "BEGIN:VCARD\r\nFN\r\nEND:VCARD", "BEGIN:VCARD\r\n:::\r\nEND:VCARD"] {
            let _ = parse(bad);
        }
        assert!(parse("not a vcard at all").is_empty());
        assert!(parse("BEGIN:VCARD\r\nEND:VCARD").is_empty(), "an empty card is not a card");
    }

    #[test]
    fn an_oversized_file_is_refused_outright() {
        let huge = format!("BEGIN:VCARD\r\nFN:A\r\nNOTE:{}\r\nEND:VCARD", "x".repeat(MAX_FILE_BYTES));
        assert!(parse(&huge).is_empty());
    }

    #[test]
    fn photos_decode_from_both_spellings_and_are_capped() {
        assert_eq!(photo_bytes("SGk=").unwrap(), b"Hi");
        assert_eq!(photo_bytes("data:image/png;base64,SGk=").unwrap(), b"Hi");
        assert!(photo_bytes("https://example.com/me.jpg").is_none());
        let big = "A".repeat(MAX_PHOTO_BYTES * 2);
        assert!(photo_bytes(&big).is_none());
        assert!(photo_bytes("!!!not base64!!!").is_none());
    }

    #[test]
    fn export_then_reimport_keeps_the_matrix_id() {
        let vcf = to_vcf("Alice", "@alice:example.com");
        assert!(vcf.contains("VERSION:4.0"));
        assert!(vcf.contains("X-MATRIX-ID:@alice:example.com"));
        let back = &parse(&vcf)[0];
        assert_eq!(back.name, "Alice");
        assert_eq!(back.matrix_id, "@alice:example.com");
    }

    #[test]
    fn an_address_book_that_dropped_the_custom_field_still_round_trips() {
        let stripped = "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Alice\r\nNOTE:Matrix: @alice:example.org\r\nEND:VCARD";
        assert_eq!(parse(stripped)[0].matrix_id, "@alice:example.org");
    }

    #[test]
    fn the_name_falls_back_to_the_id_when_there_is_no_display_name() {
        let vcf = to_vcf("  ", "@a:b.com");
        assert!(vcf.contains("FN:@a:b.com"), "{vcf}");
    }

    #[test]
    fn quoted_printable_names_are_decoded() {
        let v = "BEGIN:VCARD\r\nVERSION:2.1\r\nFN;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:=41=6C=69=63=65\r\nEND:VCARD";
        assert_eq!(parse(v)[0].name, "Alice");
    }

    #[test]
    fn a_quoted_printable_soft_break_joins_its_continuation() {
        let v = "BEGIN:VCARD\r\nVERSION:2.1\r\nFN:A\r\nNOTE;ENCODING=QUOTED-PRINTABLE:first part =\r\nsecond part\r\nEND:VCARD";
        assert_eq!(parse(v)[0].note, "first part second part");
    }

    #[test]
    fn base64_padding_is_not_mistaken_for_a_soft_break() {
        // The join is gated on the encoding: a PHOTO ends in `=`.
        let v = "BEGIN:VCARD\r\nFN:A\r\nPHOTO;ENCODING=b:SGk=\r\nTEL:123\r\nEND:VCARD";
        let c = &parse(v)[0];
        assert_eq!(c.phones[0].value, "123", "the TEL survived");
        assert_eq!(photo_bytes(&c.photo).unwrap(), b"Hi");
    }
}
