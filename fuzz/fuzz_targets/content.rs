#![no_main]
use libfuzzer_sys::fuzz_target;
use sigil_text::{Document, Origin, Parsed};
fuzz_target!(|bytes: &[u8]| {
    if bytes.len()>65536 { return; }
    if let Ok(document)=Document::from_bytes(bytes) {
        let encoded=match document {
            Document::Text(value)=>value.to_bytes().unwrap(),
            Document::Card(value)=>value.to_bytes().unwrap(),
            Document::Action(value)=>value.to_bytes().unwrap(),
        };
        assert_eq!(encoded,bytes);
    }
    let Ok(source)=std::str::from_utf8(bytes) else { return; };
    if let Ok(text)=sigil_text::parse(source,Default::default()) {
        assert!(sigil_text::Text::from_bytes(&text.to_bytes().unwrap()).unwrap()==text);
    }
    if let Ok(draft)=sigil_text::parse_card(source,Origin {message:[1;32],creator:[2;32],created_at:1788770000,timezone:Some("America/Chicago")},Default::default()) {
        match draft.content {
            Parsed::Card(card)=>assert!(sigil_text::structured::Card::from_bytes(&card.to_bytes().unwrap()).unwrap()==card),
            Parsed::Text(text)=>assert!(sigil_text::Text::from_bytes(&text.to_bytes().unwrap()).unwrap()==text),
        }
    }
});
