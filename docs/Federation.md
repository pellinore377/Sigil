# Federation transport contract

Federation is disabled by default and requires explicit peer policy. Homeserver authentication never establishes user encryption identity, group membership or read receipts.

## Discovery and authentication

Discover `/.well-known/sigil/federation` through certificate-verified HTTPS for an allowed canonical DNS name. Pin the first descriptor, optionally against an operator-supplied fingerprint. Accept an identical pin or directly predecessor-signed successor; skipped generations/replacements require operator approval.

Discovery uses four bounded workers, one-hour refresh and 60-second failure backoff. Completion rechecks configuration/peer revisions and lease. DNS results and connection addresses are validated and pinned; redirects/proxies are refused. Private addresses/nonstandard ports require explicit operator egress exceptions with TLS hostname validation.

The restricted [RFC 9421](https://www.rfc-editor.org/rfc/rfc9421.html) / [RFC 9530](https://www.rfc-editor.org/rfc/rfc9530.html) request profile signs ordered `@method`, `@path`, `content-type`, `content-digest`, `sigil-origin`, `sigil-destination`. Only known POST paths, no query, JSON, SHA-256 digest, 32-byte nonce/key ID, Ed25519 and `sigil-federation-v0` are accepted. Duplicate headers, content encoding and noncanonical parameters fail. Ed25519 signs the base directly.

Signatures last at most 120 seconds with 30-second future skew. Predecessor-key requests must predate the successor activation. Nonce admission and stored delivery/lookup operations commit atomically; transport retries use new nonces and retain application idempotence. [Calling](Calls.md) admits the federation nonce before its separate media operation, whose durable connection sequence governs retries.

## Delivery and lookup

Incoming `/federation/v0/deliver` requires recipient permission for the exact origin/account/device. Signed requests freeze recipient, message, opaque bytes and expiry of at most seven days. Identical retries return the same receipt after acknowledgement/cleanup; conflicting fields cannot reuse an ID.

Sender permissions are revisioned and have random grant IDs. Removal invalidates the grant immediately; re-enable creates a different one. Old queued payloads cannot reappear. The native mailbox merges local and remote traffic into one sequence-ordered page of at most 16 records. Remote records carry origin/account/device; the client checks these against the independently verified binding. Ordinary acknowledgement handles both.

Outgoing `/client/v0/federation/messages` derives the sender from local credentials and freezes the body. Four workers use 45-second leases, fresh signatures, revision/pin/revocation checks and validated remote receipts. Backoff grows from five to 300 seconds with jitter and completion-time Retry-After; 429/503 apply peer cooldown. Local queue acceptance is not remote acceptance. Native journals keep ciphertext until a hash-bound remote acceptance receipt is recorded; pending receipts use read-only polling. New submissions recheck group/invitation authorization.

Lookup proxies bindings/prekey claims only to allowed peers with recipient permission. Local/remote claims share atomic inventory; exact request IDs recover the same assignment. Responses never bypass independent identity verification. Network work releases database locks and remains bounded after caller timeout. Database callers wait fairly behind the active operation; cancelled work retains its permit until it actually stops.

The same signed lookup envelope carries bounded private-authority operations and encrypted attachment chunks. Credential issuance requires the caller’s current published signed device binding, attested by its home server. Authority reads, group operations and chunk requests omit account/device identifiers from the remote envelope; group proofs or file capabilities remain mandatory. Clients independently verify pinned authority profiles, proofs, receipts, membership transitions and file ciphertext. No account is created on the remote server. Lookup requests are bounded to `2 * groups::MAX_BODY + 8192`; responses to approximately 2 MiB. Group persistence uses the authority’s separate storage quota. Proxy lookups consume the device read budget; remote origin limits and service authorization apply independently.

## Peer retirement

`POST /admin/v0/federation/peers/{server}/retire` accepts `expected_revision` and returns `revision`/`complete`. Repeat with the returned revision until complete. Each transaction disables transport, terminalizes up to 64 outgoing jobs, releases up to 64 incoming payloads and revokes up to 64 sender grants. Retained pins and charged replay evidence prevent unapproved key replacement or old-packet revival. In-flight completions cannot overwrite retirement. Reconfiguration is blocked until draining completes; re-enable requires an explicit operator decision and fresh sender grants. Retirement governs transport, not encrypted group membership.

## Quotas and restore

Per-origin rate budget: burst 20, refill one/second. Pending ingress: 64 per origin and 256 total per recipient. Pending egress: 16 per source device, 64 per peer. Nonces reserve 1 KiB; delivery evidence and lookup assignments reserve 2 KiB plus retained payload charges. Global ingress/egress limits are each 1 GiB; nonce/peer metadata limits are each 64 MiB. Cleanup is bounded and retains charged replay evidence.

Restore disables federation, discards the current server signing seed, clears nonce/payload work and requires fresh peer discovery/key approval. Pending snapshot outbox jobs become terminal `restored`, not retransmitted. Server-side key deletion is not forensic erasure.

Admin status endpoints return aggregate reservations, policy, queue/cooldown and eligibility from one snapshot without packet bodies/user IDs. Eligibility is not reachability. Current implementation status is in [Status.md](Status.md).
