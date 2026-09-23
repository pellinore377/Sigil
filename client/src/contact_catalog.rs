use super::*;
use crate::conversations::{Action, Operation, Private};

const PART_BYTES: usize = 2000;
const MAX_PARTS: usize = 40;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    server: String,
    username: String,
    account: Id,
    blocked: bool,
    /// Pinned account key; its endorsements decide device trust after import.
    account_key: Id,
    verified: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    parts: usize,
    digest: Id,
}

pub(crate) fn scope(account: Id, device: Id) -> Id {
    Sha256::digest([b"Sigil/contact-catalog/v1".as_slice(), &account, &device].concat()).into()
}
pub(super) fn publish(
    tx: &rusqlite::Transaction<'_>,
    key: &StorageKey,
    own: &[u8],
    contact: &Contact,
) -> Result<(), Error> {
    if !contact.accepted() {
        return Ok(());
    }
    let Some(pin) = crate::account::pin(tx, key, &contact.server, &contact.account)? else {
        return Ok(());
    };
    let data = Zeroizing::new(
        serde_json::to_vec(&Snapshot {
            server: contact.server.clone(),
            username: contact.username.clone(),
            account: contact.account,
            blocked: contact.blocked,
            account_key: pin.key,
            verified: pin.verified,
        })
        .map_err(|_| Error::InvalidStore)?,
    );
    if data.len() > MAX_PARTS * PART_BYTES {
        return Err(Error::Limit);
    }
    let binding = peers::parse(own)?.binding;
    let author = event::account(&binding);
    let conversation = scope(author, device_fingerprint(own)?);
    let name = format!("contact.{}", transport::hex(&contact.id()));
    let current = conversations::preferences(tx, key, conversation, author)?.ui;
    let manifest = serde_json::to_string(&Manifest {
        parts: data.len().div_ceil(PART_BYTES),
        digest: Sha256::digest(&data).into(),
    })
    .map_err(|_| Error::InvalidStore)?;
    if current.get(&name) == Some(&manifest) {
        return Ok(());
    }
    let mut values: Vec<_> = data
        .chunks(PART_BYTES)
        .enumerate()
        .map(|(i, p)| (format!("{name}.{i}"), transport::hex(p)))
        .collect();
    values.push((name, manifest));
    for (name, value) in values {
        if current.get(&name) == Some(&value) {
            continue;
        }
        let mut id = [0; 32];
        getrandom::fill(&mut id).map_err(|_| sigil_crypto::Error::Entropy)?;
        let operation = conversations::new_operation(
            tx,
            key,
            device_fingerprint(own)?,
            id,
            Action::Private {
                conversation,
                value: Private::UiSetting {
                    key: name,
                    value: Some(value),
                },
            },
        )?;
        conversations::retain(
            tx,
            key,
            own,
            [0; 32],
            (author, binding.identity),
            conversations::now(),
            sigil_protocol::event::Content::Conversation(
                &operation.to_bytes().map_err(|_| Error::InvalidEvent)?,
            ),
        )?;
    }
    Ok(())
}

// Imports from same-account devices and from the recovery archive, which the recovery
// secret authenticates exactly as it does the account key itself.
pub(crate) fn receive(
    tx: &rusqlite::Transaction<'_>,
    key: &StorageKey,
    author: Id,
    operation: &Operation,
) -> Result<(), Error> {
    let Action::Private {
        conversation,
        value: Private::UiSetting { key: name, .. },
    } = &operation.action
    else {
        return Ok(());
    };
    if *conversation != scope(author, operation.version.device) {
        return Ok(());
    }
    let Some(suffix) = name.strip_prefix("contact.") else {
        return Ok(());
    };
    let reference =
        id(suffix.split('.').next().unwrap_or_default()).map_err(|_| Error::InvalidEvent)?;
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM mobile_contacts WHERE id=?1)",
        [reference.as_slice()],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(());
    }
    let name = format!("contact.{}", transport::hex(&reference));
    let values = conversations::preferences(tx, key, *conversation, author)?.ui;
    let Some(manifest) = values.get(&name) else {
        return Ok(());
    };
    let manifest: Manifest = serde_json::from_str(manifest).map_err(|_| Error::InvalidEvent)?;
    if !(1..=MAX_PARTS).contains(&manifest.parts) {
        return Err(Error::InvalidEvent);
    }
    let mut data = Zeroizing::new(Vec::new());
    for n in 0..manifest.parts {
        let Some(part) = values.get(&format!("{name}.{n}")) else {
            return Ok(());
        };
        if part.len() > PART_BYTES * 2 || !part.len().is_multiple_of(2) {
            return Err(Error::InvalidEvent);
        }
        for pair in part.as_bytes().as_chunks::<2>().0 {
            data.push(
                u8::from_str_radix(
                    std::str::from_utf8(pair).map_err(|_| Error::InvalidEvent)?,
                    16,
                )
                .map_err(|_| Error::InvalidEvent)?,
            );
        }
    }
    if <Id>::from(Sha256::digest(&data)) != manifest.digest {
        return Ok(());
    }
    let snapshot: Snapshot = serde_json::from_slice(&data).map_err(|_| Error::InvalidEvent)?;
    if !sigil_protocol::valid_server_name(&snapshot.server)
        || !sigil_protocol::accounts::valid_username(&snapshot.username)
        || event::account_reference(&snapshot.server, &snapshot.account) != reference
        || reference == author
    {
        return Err(Error::InvalidEvent);
    }
    if tx.query_row("SELECT count(*) FROM mobile_contacts", [], |r| {
        r.get::<_, u32>(0)
    })? >= 4096
    {
        return Err(Error::Limit);
    }
    if crate::account::pin(tx, key, &snapshot.server, &snapshot.account)?.is_none() {
        crate::account::save_pin(
            tx,
            key,
            &snapshot.server,
            &snapshot.account,
            &crate::account::Pin {
                key: snapshot.account_key,
                verified: snapshot.verified,
            },
        )?;
    }
    let contact = Contact {
        server: snapshot.server,
        username: snapshot.username,
        account: snapshot.account,
        legacy: None,
        outgoing: None,
        receipt: None,
        incoming: None,
        decision: None,
        work_at: 0,
        blocked: snapshot.blocked,
        block_pending: snapshot.blocked,
        review: None,
        qr_fingerprint: None,
        linked_accepted: true,
    };
    let bytes = Zeroizing::new(serde_json::to_vec(&contact).map_err(|_| Error::InvalidStore)?);
    tx.execute(
        "INSERT INTO mobile_contacts VALUES(?1,0,?2)",
        (reference.as_slice(), key.seal(&bytes, &aad(&reference))?),
    )?;
    Ok(())
}
