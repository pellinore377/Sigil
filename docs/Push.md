# Push backend acceptance

Item 10 requires generic FCM/UnifiedPush wakeups, durable registration lifecycle
and delivery retries. Platform connector/lifecycle acceptance remains separate
from server/shared-Rust implementation; no provider delivery guarantee is inferred
from a successful HTTP response. Implementation is starting; nothing below is a
completion claim.

The server will persist one revisioned push registration and coalesced job per
device, enforce account/device authorization, prove endpoint ownership through a
pending challenge, and preserve retry identity across replacement/cancellation.
A wakeup contains no sender, conversation, message text, ciphertext or file key.
Transport polling and authenticated message acknowledgement remain authoritative.

UnifiedPush uses Web Push encryption and recommends channel confirmation plus
per-request resolution checks against non-global addresses. Private self-hosted
providers need explicit operator egress exceptions; clients cannot grant those
exceptions. Redirects/proxies and unbounded response/DNS work are excluded.
[UnifiedPush integration guidance](https://unifiedpush.org/developers/intro/).

FCM uses HTTP v1 and short-lived OAuth credentials. Registration tokens remain
sensitive provider capabilities. Confirmed invalid registrations stop delivery;
transport/configuration failures retain work with bounded backoff and Retry-After.
[FCM sending](https://firebase.google.com/docs/cloud-messaging/send/v1-api),
[FCM error meanings](https://firebase.google.com/docs/cloud-messaging/error-codes).

VAPID signatures use a separate application-server P-256 signing key and bind the
push-resource origin. Provider signing/encryption keys do not become Sigil
messaging identities or Triple Ratchet keys.
[RFC 8292](https://www.rfc-editor.org/rfc/rfc8292.html).

Dependency evaluation: `web-push-native` 0.5.0 with default features disabled now
provides RFC8291 encryption/request framing. Its optional JWT/HTTP clients are not
selected. Existing pinned `ring` 0.17.14 signs fixed ES256/RS256 provider assertions,
`base64ct` 1.8.3 encodes them, and `ureq` handles HTTPS. Added RustCrypto/ECE
dependencies retain permissive licenses; MIT alternatives and upstream notices
are included in Server-ThirdParty.txt. The two Web Push crates omit license files
from their registry archives; their MIT notice was retrieved from their recorded
upstream commit `6b0a3f10377af3c9401a7c1a60b1e34d41155c5d`.
The dependency advisory check on 2026-09-07 reports no vulnerabilities and one
unmaintained-build-macro warning (RUSTSEC-2026-0173, `proc-macro-error2` via the
existing libcrux/hax all-target dependency graph). It is not introduced by push,
and is absent from the current host build graph; it remains a dependency warning.

## Provider HTTPS increment

The server now uses the existing pinned `ureq` 3.4.0/rustls stack for a bounded
provider POST adapter. Each request creates a fresh agent and validates every
resolved address before connection. Default destinations are conservative public
unicast ranges on HTTPS port 443. An operator exception binds one exact canonical
host and port to explicit canonical CIDRs and optionally a private CA; when a
rule matches, all addresses must remain inside its ranges. Exceptions never
relax certificate/name validation. Redirects and proxies are disabled. Responses
are limited to 16 KiB, compressed responses are refused, and DNS workers retain
their four-job capacity until the underlying lookup actually exits, even after
a caller timeout. Errors omit endpoint URLs and response content.

The address policy deliberately excludes protocol/translation/documentation
ranges, including some global anycast assignments, unless explicitly excepted.
[IANA IPv4 registry](https://www.iana.org/assignments/iana-ipv4-special-registry),
[IANA IPv6 registry](https://www.iana.org/assignments/iana-ipv6-special-registry).

Three tests pass: URL/CIDR/address policy, actual TLS name/DNS/redirect/body-limit
behavior, and outstanding DNS capacity after timeouts. Workspace Clippy passes.
Registration, provider authentication/encryption and delivery jobs remain
unimplemented; this adapter alone does not complete push support. The shared
HTTP/TLS notices now accompany the server's container notices.

## Provider framing increment

The provider adapter now constructs RFC8291-encrypted UnifiedPush payloads with
separate persisted-key-capable VAPID signing, and fixed-scope Google service-account
OAuth assertions plus FCM HTTP v1 requests. Secret headers omit Debug output;
provider bodies and retained OAuth credentials use zeroizing buffers. This is not
a guarantee of erasure inside third-party libraries or the allocator.

The shared SGPW v0 framing permits only a nine-byte wake or a 73-byte registration
challenge containing a channel identifier and proof. A challenge never contains
conversation data. A client must bind proof confirmation to its own durable
pending channel; parsing a push alone grants no authority. Wake requests coalesce
under one generic provider topic/collapse key. Silent FCM requests use normal
priority because the opaque mailbox also carries background protocol traffic;
Doze may delay them. Prompt visible-notification/lifecycle acceptance remains open.
[Android priority guidance](https://firebase.google.com/docs/cloud-messaging/android-message-priority).

OAuth expiry is conservatively measured from request start with a refresh margin.
Explicit provider registration-invalid errors differ from transport, malformed,
authorization and quota responses; those retain retry work. Retry-After supports
seconds and HTTP dates, and overflowing values never become an early retry.
[Service-account assertions](https://developers.google.com/identity/protocols/oauth2/service-account#authorizingrequests).

Five provider tests pass: RFC8291's published decryption vector, independently
verified ES256/RS256 signatures, payload/key/origin binding, token/failure/retry
handling, and actual synthetic TLS requests. The initial TLS test hostname did
not match the synthetic certificate; correcting the fixture resolved it without
relaxing TLS validation. Workspace Clippy passes with warnings denied. No live
Google credentials, distributor or phone push acceptance has been exercised.
Registration, configuration persistence, delivery jobs and native lifecycle
integration remain open; provider primitives alone do not complete item 10.

## Server lifecycle increment (schema 14)

`/admin/v0/push` provides admin-authenticated, revision-checked configuration with
redacted reads. UnifiedPush needs a contact and a server-generated VAPID key;
FCM configuration accepts a bounded PKCS8 service-account key, email and project.
Keep/disable/replace operations distinguish credential rotation from a project
change. Egress exceptions apply only to UnifiedPush. Google credentials always
use the fixed Google endpoints and the default public-address policy.

Native authenticated routes are `/client/v0/push/providers`, `/client/v0/push`
(GET/PUT/DELETE), and `/client/v0/push/confirm`. The first registration/cancellation
reserves 8 KiB in the account storage budget; replacements reuse the slot.
Revisions and the last operation hash prevent delayed registration requests from
reviving a cancellation. Replacement is limited to once per minute. A ten-minute
pending challenge must be echoed by the matching client channel before a
30-day active registration is enabled. Provider acceptance alone does not confirm
ownership. Accepted challenges retry every 30 seconds until confirmation/expiry.

Mailbox insertion and wake-job creation share one transaction. At most one job
per device coalesces the mailbox high-water mark. Four explicit workers commit
45-second leases before releasing the database for HTTPS. A new arrival during
delivery survives the old completion; stale leases/configuration responses cannot
invalidate a newer channel. Accepted pushes do not acknowledge messages. Already
acknowledged/expired mail is skipped. Retries use bounded exponential jitter and
provider deadlines measured at response completion. Channel cooldowns survive job
expiry; FCM quota/auth/service failures also preserve a project cooldown. Private
UnifiedPush failures cannot impose that global FCM cooldown.

Provider replacement/disable, VAPID rotation, device revocation and account
disable deny new claims immediately; bounded maintenance clears old capabilities.
An already-issued external request cannot be recalled. Configuration/provider
capabilities live in the private server database and its backup, separate from
message encryption keys. Restore clears registration secrets/jobs, service-account
credentials and VAPID keys; the operator must explicitly configure push again and
clients must re-register. Valid older snapshots do not prove deletion freshness.

Server acceptance covers seven additional lifecycle/dispatcher/API tests, all
110 server tests, 452 workspace tests (nine skipped), warnings-denied Clippy and
rebuilt container restart/restore tests. Native durable proof handling, UnifiedPush
decryption, platform connector/Doze acceptance and live provider acceptance remain
open. This is experimental backend support, not a completed item-10 claim.

## Reproduced dependency framing defects

Before adding client decryption, a synthetic standalone reproduction demonstrated
that upstream `ece-native` 0.5.0 panics on zero `rs`, accepts header-only input as
empty plaintext, and accepts authenticated invalid padding delimiters. These are
parser/format defects; the padding example does not demonstrate key recovery or
forgery without the encryption key. The native client did not yet use that parser.
The existing server path uses encryption only.

A small pinned [upstream patch](../vendor/ece-native/README.md) rejects invalid
record sizes, absent records and incorrect delimiters. Its cipher/KDF code remains
upstream code. Eight upstream vector tests and two new malformed-input tests pass;
workspace Clippy passes. The patch is included as a workspace member so it stays
in regression runs. It has not been submitted upstream or independently audited.
[RFC8188 format requirements](https://www.rfc-editor.org/rfc/rfc8188.html#section-2).

The reproduction also showed that `web-push-native` sets `rs` equal to the one
encrypted record's length. The server adapter now sets that framing field to
4096, strictly larger than either Sigil hint record, as required by RFC8291.
This does not change the ciphertext or its cryptographic associated data.
[RFC8291 single-record restrictions](https://www.rfc-editor.org/rfc/rfc8291.html#section-4).
Reproduction: `/tmp/sigil-ece-repro.log`; checks:
`/tmp/sigil-ece-patch-tests.log`, `/tmp/sigil-ece-provider-tests.log`,
`/tmp/sigil-ece-patch-clippy.log`. The schema-49 workspace, optimized client and
container regression below include this patch.

## Native lifecycle increment (schema 49)

The connected client stores one encrypted push preference and exact pending
operation, bound to its account and device. FCM token changes, UnifiedPush
connector/key/endpoint changes and cancellation survive restart. Unknown or old
connector callbacks are ignored. Fresh UnifiedPush ECDH/authentication secrets
are generated locally; the distributor receives only the connection token and
VAPID public key. The platform must authenticate its distributor callback before
passing it to these Rust APIs.

`sync_push_due_online` performs one bounded request per scheduled step. Its
separate durable schedule preserves server Retry-After, reservations and newer
preference changes. Ambiguous operations retain their exact request until
reconciled; cancellation does not pretend an already-issued registration never
happened. Active registrations are checked daily and renewed before expiry.
Changed server VAPID configuration requires the platform to fetch provider
capabilities and prepare a new connector. Server replacement cooldowns still
apply, including endpoint changes shortly after a cancellation.

UnifiedPush accepts bounded single-record RFC8291 messages using the patched
upstream parser. Only the recorded, unexpired pending channel may queue a
confirmation proof. An early challenge before the registration receipt commits
is ignored and must be delivered again. Generic wake hints carry no conversation
or sender data and do not reset messaging backoff; platform scheduling invokes
the ordinary messaging worker. Push preferences/secrets are excluded from history
handoff. No physical-erasure guarantee is inferred from local deletion.

Five native tests cover actual synthetic HTTPS, restart, failed writes, ambiguous
responses, cancellation, callback races, decryption, Retry-After and device
isolation. Workspace: **467 passed, nine skipped**. Optimized client: **244 passed,
four skipped**, including 187 library tests; four lifetime cases run only in
release. With the previous unchanged 1 GiB cache acceptance, **472 distinct tests**
pass. Warnings-denied Clippy, Docker rebuild and container restart/restore pass.
Logs: `/tmp/sigil-native-push-tests.log`, `/tmp/sigil-schema49-workspace.log`,
`/tmp/sigil-schema49-release.log`, `/tmp/sigil-schema49-clippy.log`,
`/tmp/sigil-schema49-container-build.log`, `/tmp/sigil-schema49-container.log`.

Android distributor/FCM adapters, authenticated OS callbacks, background/Doze,
visible notification delivery and live provider acceptance remain open. The
production UI remains paused. Item 10 is not complete.
