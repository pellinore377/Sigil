use sigil_crypto::{storage::StorageKey, triple::Session, DhKey, Secret32};
use std::path::Path;
fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus");
    for target in ["content", "wire", "checkpoints", "federation"] {
        std::fs::create_dir_all(root.join(target)).unwrap();
    }
    for (n, source) in [
        "synthetic text",
        "# heading\n**bold** and `code`",
        "poll::open::Question?\n- One\n- Two;",
        "checklist::Things\n- Water;",
    ]
    .iter()
    .enumerate()
    {
        std::fs::write(root.join(format!("content/source{n}")), source).unwrap();
        let text = sigil_text::parse(source, Default::default()).unwrap();
        std::fs::write(
            root.join(format!("content/text{n}")),
            text.to_bytes().unwrap(),
        )
        .unwrap();
        if let sigil_text::Parsed::Card(card) = sigil_text::parse_card(
            source,
            sigil_text::Origin {
                message: [1; 32],
                creator: [2; 32],
                created_at: 1788770000,
                timezone: Some("America/Chicago"),
            },
            Default::default(),
        )
        .unwrap()
        .content
        {
            std::fs::write(
                root.join(format!("content/card{n}")),
                card.to_bytes().unwrap(),
            )
            .unwrap();
        }
    }
    let op = sigil_protocol::conversation::Operation {
        id: [1; 32],
        version: sigil_protocol::conversation::Version {
            device: [2; 32],
            counter: 1,
        },
        action: sigil_protocol::conversation::Action::Post {
            body: sigil_protocol::conversation::Body::Text("synthetic".into()),
            reply: None,
            thread: None,
            expires_at: None,
            view_once: false,
        },
    }
    .to_bytes()
    .unwrap();
    std::fs::write(root.join("wire/operation"), &op).unwrap();
    let direct = sigil_protocol::event::Direct {
        message: [1; 32],
        conversation: [3; 32],
        sender: [2; 32],
        recipient: [4; 32],
        timestamp: 1000,
        content: sigil_protocol::event::Content::Conversation(&op),
    }
    .to_bytes()
    .unwrap();
    std::fs::write(root.join("wire/direct"), &direct).unwrap();
    let dh = DhKey::generate().unwrap();
    let mut session =
        Session::initiator(Secret32::from_bytes([7; 32]), dh.public_key(), [8; 32]).unwrap();
    let key = StorageKey::new(Secret32::from_bytes([7; 32])).unwrap();
    for n in 0..3 {
        let packet = session.send(&direct).unwrap();
        std::fs::write(root.join(format!("wire/triple{n}")), packet.to_bytes()).unwrap();
        let sealed = session.seal_checkpoint(&key, b"seed").unwrap();
        let mut raw = vec![0; 33];
        raw.extend_from_slice(&key.open(&sealed, b"seed").unwrap());
        std::fs::write(root.join(format!("checkpoints/triple{n}")), raw).unwrap();
    }
    std::fs::write(
        root.join("federation/json"),
        b"\x07{\"version\":0,\"server\":\"origin.example\"}",
    )
    .unwrap();
}
