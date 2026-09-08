# Administration API

Admin UI work remains separate. All `/admin/v0` routes require the installation
bootstrap token or an unexpired device credential belonging to an authorized
account. Browser requests additionally require the exact configured HTTPS
`public_origin`; no cookies or cross-origin credential sharing are enabled.
The bootstrap token can set the first origin through `/admin/v0/policy`.

| Role | Permission |
| --- | --- |
| Member | Own authenticated client APIs |
| Auditor | Read Admin configuration, inventories and diagnostics; no backup contents |
| Operator | Auditor plus issuing/revoking enrollment invitations |
| Administrator | All Admin APIs, including sensitive maintenance |

Assign roles using `PUT /admin/v0/accounts/{account}` with `expected_revision`,
`role`, `disabled`, `quota_bytes` (null inherits the server default) and
`confirm:true`. The last active administrator cannot be disabled or demoted.
Disabling accounts revokes credentials; re-enabling requires fresh authorization.
Role changes never change client encryption identities or grant history access.

## Setup and controls

`GET /admin/v0/setup` gathers the canonical configuration for server, policy,
push, federation, groups, calls, maps, providers, OIDC and maintenance. Configure
the server domain once, enroll the initial administrator, assign its role, set
`public_origin`, and run `check_endpoint` through maintenance. Storage lives in
the private persistent data mount; relocating it requires a mount change/restart.

`GET/PUT /admin/v0/policy` controls revision, registration (`closed`,
`invitations`, `oidc`), maximum accounts, daily registrations and public origin.
Closed registration still permits authorized replacement of existing accounts.
Daily registration accounting and per-account quotas survive restart; rejected
storage writes preserve existing data. Account inventory exposes usage/quota so
Admin can warn before exhaustion. Existing recipient blocks and federation peer
policies remain authoritative; account discovery does not bypass them.

`GET /admin/v0/accounts`, `/invitations`, and `/accounts/{account}/devices` return
bounded inventories with `next_after`; pass `?after=<id>` for the next page.
Invitation secrets and device credentials are excluded. Delete a device through
`/accounts/{account}/devices/{device}` with `confirm:true`. Existing invitation,
replacement-invitation and account-disable routes remain available.

`GET /admin/v0/diagnostics` exports aggregate storage/queue/failure/version and
maintenance information without account IDs, labels, provider credentials or
message content. Existing federation, call, provider and push status APIs supply
their detailed diagnostics. This export is JSON suitable for an Admin download.

Authenticated exact-address discovery uses `POST /client/v0/discovery` with
`username`; federation uses a signed account lookup. Native clients expose
`discover_account_online("@user:server")`. The user's revisioned
`/client/v0/discovery/preference` controls visibility. Missing and opted-out
accounts both return not found; partial search and public directories are absent.

## OIDC

Use `GET/PUT /admin/v0/oidc`. Updates require `expected_revision`, `confirm:true`
and `provider` (null disables). Provider fields: HTTPS `issuer`, `client_id`,
`client_secret` (null for public clients), and explicit egress `exceptions` for
private/self-hosted endpoints. Disable OIDC before changing the public origin.
Secrets are write-only: resupply them when
replacing provider configuration. Register the exact returned `redirect_uri`
with Pocket ID or another standard authorization-code provider. Token,
authorization and JWKS endpoints must share the issuer origin. Client-secret
Basic/POST authentication follows provider discovery metadata. Configure TLS
trust explicitly for private providers; redirects and ambient proxies are disabled.

The Rust client persists a random flow ID, proof and fresh device credential
before opening the system browser. Authorization uses code flow, S256 PKCE and
nonce binding. ID tokens require valid signatures, issuer, audience, nonce and
times; supported signature algorithms are RS256, ES256 and EdDSA. Provider keys
are refreshed during exchange. Flows expire after ten minutes and provider
configuration changes invalidate pending flows/grants.

Successful browser authentication returns a separate completion proof through
`sigil://oidc/{request_id}/{completion}`. The platform adapter passes it to
`accept_oidc_callback`; Rust accepts only its locally pending flow and persists
the proof encrypted. `/finish` requires both proofs. Polling with the original
request alone cannot authorize a client through somebody else's browser session.
Callback replay cannot retrieve the completion proof; losing the browser return
before saving it requires restarting sign-in. Register the URI handler in each
platform adapter; it must never log or forward these URLs. Authenticated provider
linking uses `prepare_oidc_link` and `finish_oidc_link_online` with the same handoff.

`/client/v0/oidc/start`, `/link`, `/finish` support enrollment and explicit linking
from an authenticated device. `/bindings` lists linked issuers; DELETE with
`issuer` and `confirm:true` unlinks and invalidates outstanding link/replacement
grants. Association uses verified issuer/subject, never email or display name.
OIDC registration cannot claim an existing unlinked username. Existing-account
login requires explicit device replacement; old credentials are revoked and
the new device starts without the old encryption identity or recovery keys.
History still requires the client's separate encrypted recovery mechanism.
Native `restart_oidc_enrollment` abandons an unfinished flow with fresh proofs.

## Guided maintenance

`GET/PUT /admin/v0/maintenance/configuration` controls revision, maximum backup
file size (default 64 GiB), optional signed release URL/key and egress exceptions.
Files stay in private `data/maintenance`, or can be downloaded to operator-selected
storage. New snapshots/uploads stop at 120 directory entries, reserving space for
staging; two uploads and 64 operation records are admitted. Delete unused backups
and uploads explicitly; no history is silently pruned.

1. POST `/admin/v0/maintenance/prepare` with an action. Display its `effect`.
2. POST `/actions/{id}` with `confirm:true` using the same credential within five
   minutes. Authorization is checked again when the durable worker starts.
3. Poll GET `/actions/{id}`; GET `/actions` recovers current job IDs after reconnect.

Actions: `backup`, `check_endpoint`, `check_update`, `inspect`/`import`/`restore`
with a `file` ID, and `upgrade` with the selected signed `release`.

GET `/files` lists completed files. POST `/files` with `bytes` and hexadecimal
`sha256` reserves an upload; PUT `/files/{id}/{offset}` sends sequential chunks
of at most 4 MiB. GET `/files/{id}` reports committed progress. Exact retransmits
are idempotent. `import` checks the hash, SQLite integrity, schema and absence of
executable schema objects before publishing the file. GET `/files/{id}/{offset}`
downloads completed chunks; DELETE `/files/{id}` requires `confirm:true`.
Backups contain server credentials and metadata; protect downloaded copies.

Restore first backs up current state, prepares a sanitized replacement and
returns `restart_required`. Restart through the deployment manager activates it
under the installation lock, preserving the previous database. Restored device
credentials and external provider configuration are disabled; client recovery
keys are never supplied by Admin. The current installation bootstrap token is
retained for this in-place workflow.

Release envelopes contain `release` and a hexadecimal Ed25519 `signature` over
the compact UTF-8 JSON serialization of `release`, in this field order:
`version`, `image_digest`, `minimum_schema`, `target_schema`. Pin the publisher's
public key independently. An upgrade must be newer, compatible with the current
schema and identify an immutable `sha256:` image digest. `upgrade` rechecks that
exact signed release and creates a backup before returning the deployment action.
Dockge/Compose recreates the existing image repository at that digest; Sigil does
not hold a Docker socket. Rollback uses the previous image and preserved backup,
never an older binary against a newer database schema. No release feed is enabled
until an operator configures trusted publication infrastructure.
