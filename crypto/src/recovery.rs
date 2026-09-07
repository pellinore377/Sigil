//! Experimental history-only archive objects. Never a live-state backup format.
use crate::{
    checkpoint::{Reader, Writer},
    identity::validate_public,
    storage::{StorageKey, MAX_RECORD},
    Error, Secret32, MAX_PLAINTEXT,
};
use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use zeroize::Zeroizing;

const DOMAIN: &[u8] = b"Sigil/experimental/recovery/v0_AES256GCMSIV_SHA-256";
pub const MAX_OBJECT_LEN: usize = MAX_RECORD + 36;
pub const MAX_PAGE_RECORDS: usize = 256;
pub const MAX_MANIFEST_PAGES: usize = 512;
pub type Id = [u8; 32];

pub struct RecoveryKey {
    master: Secret32,
    storage: StorageKey,
    scope: Id,
}

/// Public ciphertext, identified by its hash. This type carries no content keys.
pub struct Object {
    id: Id,
    bytes: Vec<u8>,
}
impl Object {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, Error> {
        let id = object_id(&bytes)?;
        Ok(Self { id, bytes })
    }
    pub fn id(&self) -> Id {
        self.id
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

pub fn object_id(bytes: &[u8]) -> Result<Id, Error> {
    if !(36..=MAX_OBJECT_LEN).contains(&bytes.len()) {
        return Err(Error::Limit);
    }
    Ok(Sha256::digest(bytes).into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Incoming,
    Outgoing,
}

pub enum Content {
    /// Only explicitly classified retained content belongs here. Disappearing,
    /// view-once and transient events must be filtered before archive creation.
    Retained(Zeroizing<Vec<u8>>),
    /// Canonical SGFC retained file descriptor/metadata, never live ratchet state.
    File(Zeroizing<Vec<u8>>),
    Rich(Zeroizing<Vec<u8>>),
    Deleted,
}

pub struct Record {
    pub id: Id,
    pub revision: u64,
    pub conversation: Id,
    pub author: Id,
    pub created_at: u64,
    pub direction: Direction,
    pub content: Content,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reference {
    pub id: Id,
    pub revision: u64,
    pub object: Id,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageReference {
    pub first: Id,
    pub last: Id,
    pub object: Id,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Head {
    pub generation: u64,
    pub manifest: Id,
}

pub struct Manifest {
    pub generation: u64,
    pub created_at: u64,
    pub previous: Option<Id>,
    pub pages: Vec<PageReference>,
}
impl Manifest {
    pub fn continues(&self, previous: &Head) -> bool {
        previous.generation.checked_add(1) == Some(self.generation)
            && self.previous == Some(previous.manifest)
    }
}

fn positive(value: u64) -> Result<(), Error> {
    if value == 0 || value > i64::MAX as u64 {
        Err(Error::Encoding)
    } else {
        Ok(())
    }
}
fn timestamp(value: u64) -> Result<(), Error> {
    if value > i64::MAX as u64 {
        Err(Error::Encoding)
    } else {
        Ok(())
    }
}
fn validate_references(records: &[Reference]) -> Result<(), Error> {
    if records.is_empty() || records.len() > MAX_PAGE_RECORDS {
        return Err(Error::Limit);
    }
    for record in records {
        positive(record.revision)?;
    }
    if records.windows(2).any(|pair| pair[0].id >= pair[1].id) {
        return Err(Error::Encoding);
    }
    if records
        .iter()
        .map(|record| record.object)
        .collect::<BTreeSet<_>>()
        .len()
        != records.len()
    {
        return Err(Error::Encoding);
    }
    Ok(())
}
fn validate_pages(pages: &[PageReference]) -> Result<(), Error> {
    if pages.len() > MAX_MANIFEST_PAGES {
        return Err(Error::Limit);
    }
    if pages.iter().any(|page| page.first > page.last)
        || pages.windows(2).any(|pair| pair[0].last >= pair[1].first)
    {
        return Err(Error::Encoding);
    }
    if pages
        .iter()
        .map(|page| page.object)
        .collect::<BTreeSet<_>>()
        .len()
        != pages.len()
    {
        return Err(Error::Encoding);
    }
    Ok(())
}

impl RecoveryKey {
    /// Scope must bind the canonical homeserver and stable account reference.
    pub fn from_secret(master: Secret32, scope: Id) -> Result<Self, Error> {
        let mut secret = Zeroizing::new([0; 32]);
        Hkdf::<Sha256>::new(Some(&[0; 32]), master.0.as_ref())
            .expand(&[DOMAIN, b":Storage Key", &scope].concat(), secret.as_mut())
            .map_err(|_| Error::State)?;
        Ok(Self {
            master,
            storage: StorageKey::new(Secret32(secret))?,
            scope,
        })
    }
    pub fn generate(scope: Id) -> Result<Self, Error> {
        Self::from_secret(Secret32::generate()?, scope)
    }
    /// Recovery material: protect separately from the server and live database.
    pub fn export_secret(&self) -> Zeroizing<Id> {
        Zeroizing::new(*self.master.0)
    }

    fn binding(&self, kind: u8) -> Vec<u8> {
        [DOMAIN, &self.scope, &[kind]].concat()
    }
    fn seal(&self, kind: u8, bytes: &[u8]) -> Result<Object, Error> {
        Object::from_bytes(self.storage.seal(bytes, &self.binding(kind))?)
    }
    fn open(&self, kind: u8, expected: &Id, bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
        if object_id(bytes)? != *expected {
            return Err(Error::Authentication);
        }
        self.storage.open(bytes, &self.binding(kind))
    }

    pub fn seal_record(&self, record: &Record) -> Result<(Reference, Object), Error> {
        positive(record.revision)?;
        timestamp(record.created_at)?;
        validate_public(&record.author)?;
        let mut out = Writer::archive();
        out.put(b"SGHR\0\x01\0\0")?;
        out.put(&record.id)?;
        out.u64(record.revision)?;
        out.put(&record.conversation)?;
        out.put(&record.author)?;
        out.u64(record.created_at)?;
        out.u8(match record.direction {
            Direction::Incoming => 0,
            Direction::Outgoing => 1,
        })?;
        match &record.content {
            Content::Retained(bytes) => {
                if bytes.len() > MAX_PLAINTEXT {
                    return Err(Error::Limit);
                }
                out.u8(0)?;
                out.blob(bytes)?;
            }
            Content::Deleted => out.u8(1)?,
            Content::File(bytes) => {
                sigil_protocol::file::File::from_bytes(bytes).map_err(|_| Error::Encoding)?;
                out.u8(2)?;
                out.blob(bytes)?;
            }
            Content::Rich(bytes) => {
                sigil_protocol::text::Document::from_bytes(bytes).map_err(|_| Error::Encoding)?;
                out.u8(3)?;
                out.blob(bytes)?;
            }
        }
        let object = self.seal(1, &out.finish())?;
        Ok((
            Reference {
                id: record.id,
                revision: record.revision,
                object: object.id,
            },
            object,
        ))
    }

    pub fn open_record(&self, expected: &Reference, bytes: &[u8]) -> Result<Record, Error> {
        let plaintext = self.open(1, &expected.object, bytes)?;
        let mut input = Reader::archive(&plaintext)?;
        if &input.take::<8>()? != b"SGHR\0\x01\0\0" {
            return Err(Error::Encoding);
        }
        let id = input.take()?;
        let revision = input.u64()?;
        positive(revision)?;
        if id != expected.id || revision != expected.revision {
            return Err(Error::Authentication);
        }
        let conversation = input.take()?;
        let author = input.take()?;
        validate_public(&author)?;
        let created_at = input.u64()?;
        timestamp(created_at)?;
        let direction = match input.u8()? {
            0 => Direction::Incoming,
            1 => Direction::Outgoing,
            _ => return Err(Error::Encoding),
        };
        let content = match input.u8()? {
            0 => Content::Retained(Zeroizing::new(input.blob(MAX_PLAINTEXT)?.to_vec())),
            1 => Content::Deleted,
            2 => {
                let bytes = input.blob(sigil_protocol::file::MAX_CONTENT)?;
                sigil_protocol::file::File::from_bytes(bytes).map_err(|_| Error::Encoding)?;
                Content::File(Zeroizing::new(bytes.to_vec()))
            }
            3 => {
                let bytes = input.blob(sigil_protocol::text::MAX_WIRE_BYTES)?;
                sigil_protocol::text::Document::from_bytes(bytes).map_err(|_| Error::Encoding)?;
                Content::Rich(Zeroizing::new(bytes.to_vec()))
            }
            _ => return Err(Error::Encoding),
        };
        input.finish()?;
        Ok(Record {
            id,
            revision,
            conversation,
            author,
            created_at,
            direction,
            content,
        })
    }

    pub fn seal_page(&self, records: &[Reference]) -> Result<(PageReference, Object), Error> {
        validate_references(records)?;
        let mut out = Writer::archive();
        out.put(b"SGHP\0\x01\0\0")?;
        out.u32(records.len() as u32)?;
        for reference in records {
            out.put(&reference.id)?;
            out.u64(reference.revision)?;
            out.put(&reference.object)?;
        }
        let object = self.seal(2, &out.finish())?;
        Ok((
            PageReference {
                first: records[0].id,
                last: records[records.len() - 1].id,
                object: object.id,
            },
            object,
        ))
    }

    pub fn open_page(
        &self,
        expected: &PageReference,
        bytes: &[u8],
    ) -> Result<Vec<Reference>, Error> {
        let plaintext = self.open(2, &expected.object, bytes)?;
        let mut input = Reader::archive(&plaintext)?;
        if &input.take::<8>()? != b"SGHP\0\x01\0\0" {
            return Err(Error::Encoding);
        }
        let count = input.u32()? as usize;
        if count == 0 || count > MAX_PAGE_RECORDS {
            return Err(Error::Limit);
        }
        let mut records = Vec::with_capacity(count);
        for _ in 0..count {
            records.push(Reference {
                id: input.take()?,
                revision: input.u64()?,
                object: input.take()?,
            });
        }
        input.finish()?;
        validate_references(&records)?;
        if expected.first != records[0].id || expected.last != records[count - 1].id {
            return Err(Error::Authentication);
        }
        Ok(records)
    }

    pub fn seal_manifest(
        &self,
        previous: Option<&Head>,
        created_at: u64,
        pages: &[PageReference],
    ) -> Result<(Head, Object), Error> {
        timestamp(created_at)?;
        validate_pages(pages)?;
        if let Some(previous) = previous {
            positive(previous.generation)?;
        }
        let generation = previous
            .map_or(Some(1), |head| head.generation.checked_add(1))
            .ok_or(Error::Limit)?;
        positive(generation)?;
        let mut out = Writer::archive();
        out.put(b"SGHM\0\x01\0\0")?;
        out.u64(generation)?;
        out.u64(created_at)?;
        out.u8(u8::from(previous.is_some()))?;
        if let Some(previous) = previous {
            out.put(&previous.manifest)?;
        }
        out.u32(pages.len() as u32)?;
        for page in pages {
            out.put(&page.first)?;
            out.put(&page.last)?;
            out.put(&page.object)?;
        }
        let object = self.seal(3, &out.finish())?;
        Ok((
            Head {
                generation,
                manifest: object.id,
            },
            object,
        ))
    }

    pub fn open_manifest(&self, expected: &Head, bytes: &[u8]) -> Result<Manifest, Error> {
        let plaintext = self.open(3, &expected.manifest, bytes)?;
        let mut input = Reader::archive(&plaintext)?;
        if &input.take::<8>()? != b"SGHM\0\x01\0\0" {
            return Err(Error::Encoding);
        }
        let generation = input.u64()?;
        positive(generation)?;
        if generation != expected.generation {
            return Err(Error::Authentication);
        }
        let created_at = input.u64()?;
        timestamp(created_at)?;
        let previous = match input.u8()? {
            0 => None,
            1 => Some(input.take()?),
            _ => return Err(Error::Encoding),
        };
        if (generation == 1) != previous.is_none() {
            return Err(Error::Encoding);
        }
        let count = input.u32()? as usize;
        if count > MAX_MANIFEST_PAGES {
            return Err(Error::Limit);
        }
        let mut pages = Vec::with_capacity(count);
        for _ in 0..count {
            pages.push(PageReference {
                first: input.take()?,
                last: input.take()?,
                object: input.take()?,
            });
        }
        input.finish()?;
        validate_pages(&pages)?;
        Ok(Manifest {
            generation,
            created_at,
            previous,
            pages,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IdentityKey;

    fn key() -> RecoveryKey {
        RecoveryKey::from_secret(Secret32::from_bytes([7; 32]), [8; 32]).unwrap()
    }
    fn record(id: u8, content: Content) -> Record {
        Record {
            id: [id; 32],
            revision: 1,
            conversation: [3; 32],
            author: IdentityKey::generate().unwrap().public_key(),
            created_at: 1000,
            direction: Direction::Incoming,
            content,
        }
    }

    #[test]
    fn rich_history_is_validated_and_never_reinterpreted_as_plain_text() {
        let text = sigil_protocol::text::parse(
            "redact::SYNTHETIC_SECRET; bold::retained;",
            Default::default(),
        )
        .unwrap();
        let bytes = text.to_bytes().unwrap();
        let key = key();
        let source = record(1, Content::Rich(Zeroizing::new(bytes.clone())));
        let (reference, object) = key.seal_record(&source).unwrap();
        let restored = key.open_record(&reference, object.bytes()).unwrap();
        let Content::Rich(restored) = restored.content else {
            panic!("lost rich history kind")
        };
        assert_eq!(*restored, bytes);
        assert!(!std::str::from_utf8(&restored)
            .unwrap()
            .contains("SYNTHETIC_SECRET"));
        assert!(matches!(
            key.seal_record(&record(
                2,
                Content::Rich(Zeroizing::new(b"ordinary text".to_vec()))
            )),
            Err(Error::Encoding)
        ));
        let (reference, object) = key
            .seal_record(&record(3, Content::Retained(Zeroizing::new(bytes))))
            .unwrap();
        assert!(matches!(
            key.open_record(&reference, object.bytes()).unwrap().content,
            Content::Retained(_)
        ));
        let sigil_protocol::text::Parsed::Card(card) = sigil_protocol::text::parse_card(
            "note::redact::SYNTHETIC_SECRET;",
            sigil_protocol::text::Origin {
                message: [9; 32],
                creator: [8; 32],
                created_at: 1000,
                timezone: None,
            },
            Default::default(),
        )
        .unwrap()
        .content
        else {
            panic!("lost structured note")
        };
        let bytes = card.to_bytes().unwrap();
        let (reference, object) = key
            .seal_record(&record(4, Content::Rich(Zeroizing::new(bytes.clone()))))
            .unwrap();
        let Content::Rich(restored) = key.open_record(&reference, object.bytes()).unwrap().content
        else {
            panic!("lost structured history kind")
        };
        assert_eq!(*restored, bytes);
        assert!(sigil_protocol::text::structured::Card::from_bytes(&restored).unwrap() == card);
    }
    #[test]
    fn retained_file_keys_are_typed_and_strict_without_reinterpreting_text() {
        let file_key = crate::attachment::FileKey::generate(0).unwrap();
        let chunk = file_key.seal_chunk(0, b"").unwrap();
        let mut list = crate::attachment::CiphertextList::new(file_key.shape()).unwrap();
        list.push(0, &chunk).unwrap();
        let descriptor = file_key.descriptor(list.finish().unwrap());
        let bytes = sigil_protocol::file::File {
            source: "chat.example",
            name: "synthetic.bin",
            media_type: "application/octet-stream",
            expires_at: None,
            access: &[4; 32],
            descriptor: &descriptor,
        }
        .to_bytes()
        .unwrap();
        let key = key();
        let source = record(1, Content::File(Zeroizing::new(bytes.clone())));
        let (reference, object) = key.seal_record(&source).unwrap();
        let restored = key.open_record(&reference, object.bytes()).unwrap();
        let Content::File(restored) = restored.content else {
            panic!("lost file content type")
        };
        assert_eq!(*restored, bytes);
        let file = sigil_protocol::file::File::from_bytes(&restored).unwrap();
        let (file_key, _) = crate::attachment::FileKey::from_descriptor(file.descriptor).unwrap();
        assert!(file_key.open_chunk(0, &chunk).unwrap().is_empty());
        let source = record(2, Content::Retained(Zeroizing::new(bytes)));
        let (reference, object) = key.seal_record(&source).unwrap();
        assert!(matches!(
            key.open_record(&reference, object.bytes()).unwrap().content,
            Content::Retained(_)
        ));
        assert!(matches!(
            key.seal_record(&record(
                3,
                Content::File(Zeroizing::new(b"not a file descriptor".to_vec()))
            )),
            Err(Error::Encoding)
        ));
    }

    #[test]
    fn complete_snapshot_and_tombstone_survive_recovery_key_export() {
        let key = key();
        let first = record(
            1,
            Content::Retained(Zeroizing::new(b"synthetic history".to_vec())),
        );
        let (reference, object) = key.seal_record(&first).unwrap();
        let (page_ref, page) = key.seal_page(&[reference]).unwrap();
        let (head, manifest) = key.seal_manifest(None, 1001, &[page_ref]).unwrap();
        let restored =
            RecoveryKey::from_secret(Secret32::from_bytes(*key.export_secret()), [8; 32]).unwrap();
        let manifest = restored.open_manifest(&head, manifest.bytes()).unwrap();
        assert_eq!(manifest.generation, 1);
        assert!(manifest.previous.is_none());
        let references = restored
            .open_page(&manifest.pages[0], page.bytes())
            .unwrap();
        let value = restored
            .open_record(&references[0], object.bytes())
            .unwrap();
        assert_eq!(value.id, first.id);
        assert_eq!(value.conversation, first.conversation);
        assert_eq!(value.author, first.author);
        assert!(
            matches!(value.content, Content::Retained(ref text) if text.as_slice() == b"synthetic history")
        );
        let deleted = Record {
            revision: 2,
            content: Content::Deleted,
            ..first
        };
        let (reference, object) = key.seal_record(&deleted).unwrap();
        let (page_ref, page) = key.seal_page(&[reference]).unwrap();
        let (new_head, manifest) = key.seal_manifest(Some(&head), 1002, &[page_ref]).unwrap();
        let manifest = key.open_manifest(&new_head, manifest.bytes()).unwrap();
        assert!(manifest.continues(&head));
        let references = key.open_page(&manifest.pages[0], page.bytes()).unwrap();
        let value = key.open_record(&references[0], object.bytes()).unwrap();
        assert_eq!(value.revision, 2);
        assert!(matches!(value.content, Content::Deleted));
    }

    #[test]
    fn keys_scopes_purposes_and_authenticated_references_cannot_be_substituted() {
        let key = key();
        let source = record(1, Content::Retained(Zeroizing::new(b"history".to_vec())));
        let (reference, object) = key.seal_record(&source).unwrap();
        for wrong in [
            RecoveryKey::from_secret(Secret32::from_bytes([9; 32]), [8; 32]).unwrap(),
            RecoveryKey::from_secret(Secret32::from_bytes([7; 32]), [9; 32]).unwrap(),
        ] {
            assert!(matches!(
                wrong.open_record(&reference, object.bytes()),
                Err(Error::Authentication)
            ));
        }
        let wrong_id = Reference {
            id: [2; 32],
            ..reference
        };
        let wrong_revision = Reference {
            revision: 2,
            ..reference
        };
        assert!(matches!(
            key.open_record(&wrong_id, object.bytes()),
            Err(Error::Authentication)
        ));
        assert!(matches!(
            key.open_record(&wrong_revision, object.bytes()),
            Err(Error::Authentication)
        ));
        let (other_ref, other) = key.seal_record(&record(2, Content::Deleted)).unwrap();
        assert!(matches!(
            key.open_record(&reference, other.bytes()),
            Err(Error::Authentication)
        ));
        assert!(matches!(
            key.open_record(
                &Reference {
                    object: other_ref.object,
                    ..reference
                },
                other.bytes()
            ),
            Err(Error::Authentication)
        ));
        let page_ref = PageReference {
            first: source.id,
            last: source.id,
            object: object.id,
        };
        assert!(matches!(
            key.open_page(&page_ref, object.bytes()),
            Err(Error::Authentication)
        ));
        let (page_ref, page) = key.seal_page(&[reference]).unwrap();
        assert!(matches!(
            key.open_page(
                &PageReference {
                    first: [0; 32],
                    ..page_ref
                },
                page.bytes()
            ),
            Err(Error::Authentication)
        ));
        let (head, manifest) = key.seal_manifest(None, 1000, &[page_ref]).unwrap();
        assert!(matches!(
            key.open_manifest(
                &Head {
                    generation: 2,
                    ..head
                },
                manifest.bytes()
            ),
            Err(Error::Authentication)
        ));
        assert!(StorageKey::new(Secret32::from_bytes([7; 32]))
            .unwrap()
            .open(object.bytes(), &key.binding(1))
            .is_err());
    }

    #[test]
    fn every_ciphertext_byte_and_object_size_are_checked() {
        let key = key();
        let (reference, object) = key.seal_record(&record(1, Content::Deleted)).unwrap();
        for index in 0..object.bytes.len() {
            let mut bytes = object.bytes.clone();
            bytes[index] ^= 1;
            assert!(matches!(
                key.open_record(&reference, &bytes),
                Err(Error::Authentication)
            ));
            let replaced = Reference {
                object: object_id(&bytes).unwrap(),
                ..reference
            };
            assert!(key.open_record(&replaced, &bytes).is_err());
        }
        assert!(Object::from_bytes(vec![0; 35]).is_err());
        assert!(Object::from_bytes(vec![0; MAX_OBJECT_LEN + 1]).is_err());
    }

    #[test]
    fn maximum_payload_and_index_bounds_are_enforced() {
        let key = key();
        let (reference, object) = key
            .seal_record(&record(
                1,
                Content::Retained(Zeroizing::new(vec![42; MAX_PLAINTEXT])),
            ))
            .unwrap();
        let restored = key.open_record(&reference, object.bytes()).unwrap();
        assert!(
            matches!(restored.content, Content::Retained(ref value) if value.len() == MAX_PLAINTEXT)
        );
        assert!(key
            .seal_record(&record(
                2,
                Content::Retained(Zeroizing::new(vec![0; MAX_PLAINTEXT + 1]))
            ))
            .is_err());
        let mut records = Vec::new();
        for index in 0..MAX_PAGE_RECORDS as u32 {
            let mut id = [0; 32];
            id[..4].copy_from_slice(&index.to_be_bytes());
            records.push(Reference {
                id,
                revision: 1,
                object: id,
            });
        }
        let (page_ref, page) = key.seal_page(&records).unwrap();
        assert_eq!(key.open_page(&page_ref, page.bytes()).unwrap(), records);
        records.push(Reference {
            id: [255; 32],
            revision: 1,
            object: [255; 32],
        });
        assert!(key.seal_page(&records).is_err());
        let mut pages = Vec::new();
        for index in 0..MAX_MANIFEST_PAGES as u32 {
            let mut id = [0; 32];
            id[..4].copy_from_slice(&index.to_be_bytes());
            pages.push(PageReference {
                first: id,
                last: id,
                object: id,
            });
        }
        let (head, manifest) = key.seal_manifest(None, 1, &pages).unwrap();
        assert_eq!(
            key.open_manifest(&head, manifest.bytes()).unwrap().pages,
            pages
        );
        pages.push(PageReference {
            first: [255; 32],
            last: [255; 32],
            object: [255; 32],
        });
        assert!(key.seal_manifest(None, 1, &pages).is_err());
        assert!(key
            .seal_manifest(
                Some(&Head {
                    generation: i64::MAX as u64,
                    manifest: [1; 32]
                }),
                1,
                &[]
            )
            .is_err());
    }

    #[test]
    fn duplicate_unsorted_and_malformed_archive_structures_fail_closed() {
        let key = key();
        let (reference, object) = key.seal_record(&record(1, Content::Deleted)).unwrap();
        assert!(key.seal_page(&[]).is_err());
        assert!(key.seal_page(&[reference, reference]).is_err());
        assert!(key
            .seal_page(&[Reference {
                revision: 0,
                ..reference
            }])
            .is_err());
        let (page_ref, _) = key.seal_page(&[reference]).unwrap();
        assert!(key.seal_manifest(None, 1, &[page_ref, page_ref]).is_err());
        let bytes = key.open(1, &reference.object, object.bytes()).unwrap();
        for len in [0, 7, 39, bytes.len() - 1] {
            let object = key.seal(1, &bytes[..len]).unwrap();
            assert!(key
                .open_record(
                    &Reference {
                        object: object.id,
                        ..reference
                    },
                    object.bytes()
                )
                .is_err());
        }
        let mut bytes = bytes;
        bytes.push(0);
        let object = key.seal(1, &bytes).unwrap();
        assert!(key
            .open_record(
                &Reference {
                    object: object.id,
                    ..reference
                },
                object.bytes()
            )
            .is_err());
        let mut out = Writer::archive();
        out.put(b"SGHP\0\x01\0\0").unwrap();
        out.u32(u32::MAX).unwrap();
        let object = key.seal(2, &out.finish()).unwrap();
        assert!(matches!(
            key.open_page(
                &PageReference {
                    object: object.id,
                    ..page_ref
                },
                object.bytes()
            ),
            Err(Error::Limit)
        ));
    }
}
