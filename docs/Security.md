# Backend security boundaries

These experimental profiles implement published designs independently; they are not Signal wire compatibility or an independent security certification.

## Compatibility

| Boundary | Accepted profile | Rejection rule |
| --- | --- | --- |
| Client/Admin HTTP | Explicit `/client/v0` and `/admin/v0` routes | Unknown routes/methods fail; native clients require HTTPS, with no fallback |
| Federation | Discovery 0, signed `/federation/v0` paths and `sigil-federation-v0` | Unknown version/signature profile, unapproved key replacement and stale generations fail |
| Pairwise messaging | `SGHI` 2, authenticated Triple Ratchet suite 2 | Classical sessions are history-only; reframing cannot select another suite |
| Encrypted events | Exact canonical `SGEV`/group frames and `SGCO` 1 | Unknown framing/content/fields fail; never reinterpret as ordinary text |
| History recovery | Authenticated `SGHR`, `SGHP`, `SGHM` 1 | Wrong scope, hash, key, lineage or version fails; no live-state import |
| Local databases | Server 27, client 68, cache 5 | Validate identity/key/schema before use; newer schemas fail closed |

`/versions` describes storage APIs. Empty messaging arrays do not certify unfinished interoperability. There is no opportunistic negotiation: future incompatible profiles need explicit versioned routes/framing and authenticated selection. An older binary must use a pre-upgrade server backup on separate storage, followed by restore sanitization; never open a migrated database or roll back live client state.

## Erasure

SQLite connections enable `secure_delete`, DELETE journaling and EXTRA synchronization. Superseded records are cleared from live database pages before commit returns; the rollback journal is removed. Upgrades first drain old WAL state and vacuum historical free pages. A durable pending marker makes interrupted cleanup retry on open. Migration requires temporary disk space and exclusive access; failure prevents opening the store.

Bounded journal maintenance removes obsolete message bodies while retaining authenticated replay identifiers and commitments. Unacknowledged incoming results remain until acknowledgement. Legacy history-sync fragments require complete retained parts to recover their source association; incomplete transfers remain retained. Recovery retention governs separately archived copies. The platform must keep the wrapping key outside the database and exclude live client state from system backups.

These guarantees concern accessible application files. They do not sanitize filesystem snapshots, exported backups, swap, compiler/library copies, SSD remapping or recipient copies. Device encryption and platform key destruction remain necessary; changing a wrapping alias alone does not erase old keys. [SQLite's guarantees](https://www.sqlite.org/pragma.html#pragma_secure_delete) do not extend to the underlying storage medium.

## Metadata and dependencies

Homeservers observe account/device identifiers, peers, timestamps, sizes, queues, storage ownership and transport addresses. Local mailbox users can infer aggregate activity from global cursor gaps; federation acceptance tokens disclose no such counter. Private-group proofs conceal membership content, not all timing/correlation. Credential issuance trusts the user's homeserver; identity-signed membership and endpoint-held Sender Keys remain separate requirements. Relays see call routing and encrypted packet sizes/timing. Push providers receive generic wake-ups and endpoint identifiers. Configured integrations see explicitly disclosed queries. Redacted diagnostics omit identifying payloads; they do not eliminate this metadata.

Dependency advisories require reachability review. `rsa` is used by OIDC for public-key verification only; Sigil never uses its private-key operations implicated by [RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071.html). The 3MF dependency pins older `quick-xml`: a patched parser preflight rejects excessive attributes, names and DTDs before entering it, and preview workers have process limits. The two XML advisories remain visible until the upstream dependency upgrades. The unmaintained `proc-macro-error2` dependency is build-time Hax tooling.
