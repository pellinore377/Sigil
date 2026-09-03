# sigil-server

The Sigil home server and Envoy, one binary. Design:
[`docs/blind-backend.md`](../docs/blind-backend.md). Wire protocol:
[`docs/spec/sigil-wire-v1.md`](../docs/spec/sigil-wire-v1.md).

## Status

Phase 1. Implemented: names with invite-code registration, slots (put, get,
ack, subscribe), the requests slot, key-package shelves, blobs, backups and
wraps, blind credentials and daily tokens with double-spend detection, the
Envoy role with per-device queues (capped at 1,000 per handle), jitter,
delivery streams (in-process when both roles run in one process, WebSocket
otherwise), UnifiedPush wake-ups for offline devices, an expiry sweep, TLS
from PEM files, raw TPM relay for recovery Path 2, cover traffic, and the
call forwarding unit on one UDP port. The OIDC gate (registration
through Pocket ID or any OpenID Connect provider). Not yet: ACME, APNs and
FCM, the client-side TPM session, a relay for UDP-blocked calls.

## Run it locally

```
cargo build
target/debug/sigil-server -c sigil.toml init --hostname sigil.test --listen 127.0.0.1:8443
target/debug/sigil-server -c sigil.toml run          # plain HTTP: local testing only
target/debug/sigil-server -c sigil.toml invite       # prints an invite code
```

Then, with the command-line client from `client/`:

```
sigil-cli -s alice.json init --username @alice:sigil.test --envoy ws://127.0.0.1:8443/envoy
sigil-cli -s alice.json register --invite <code>   # name, credential, tokens, key packages
sigil-cli -s bob.json   requests --accept          # wait for a request and join it
sigil-cli -s alice.json dm @bob:sigil.test "hi"    # Welcome through bob's requests slot
sigil-cli -s alice.json send 0 "hello again"
sigil-cli -s bob.json   listen 0                   # backfill, then live
```

`client/tests/e2e.sh` runs the whole flow, including the offline queue and
a server restart.

## Docker and Dockge

`server/docker-compose.yml` is a complete stack: paste it into Dockge (or
`docker compose up -d`), fill in the `SIGIL_*` variables, start it. The
image comes from GitHub's registry, built by `.github/workflows/server-image.yml`
on every push (`ghcr.io/pellinore377/sigil-server:encrypt`); the commented
`build:` block compiles it on the server instead.

There is no config step: on the first start the server writes
`data/sigil.toml` from the environment, and on every later start the
`SIGIL_*` variables win over the file, so changing one in Dockge and
restarting is enough. Everything the server keeps lives in `./data` next
to the compose file (the config, the database, the admin token).

| Variable | Config key | Meaning |
|---|---|---|
| `SIGIL_HOSTNAME` | `hostname` | the public name, the `:server` half of every `@name:server` |
| `SIGIL_REGISTRATION` | `registration` | `invite`, `open` or `oidc` |
| `SIGIL_OIDC_ISSUER`, `SIGIL_OIDC_CLIENT_ID` | `oidc_issuer`, `oidc_client_id` | the provider and the client id for `oidc` |
| `SIGIL_MEDIA_PUBLIC` | `media_public` | where callers send media: your public IP or name, `:8444` |
| `SIGIL_CALLS` | `calls` | `true`/`false` |
| `SIGIL_LISTEN`, `SIGIL_ROLE`, `SIGIL_MEDIA_UDP` | `listen`, `role`, `media_udp` | rarely needed in a container |
| `SIGIL_TLS_CERT`, `SIGIL_TLS_KEY` | `tls_cert`, `tls_key` | only without a reverse proxy |
| `SIGIL_TOKENS_PER_DAY`, `SIGIL_JITTER_MAX_MS`, `SIGIL_COVER_PER_MINUTE` | as named | tuning |

Behind a reverse proxy that terminates TLS (Nginx Proxy Manager, Traefik,
Caddy), proxy `https://<hostname>` to the container's port 8443 **with
WebSocket support** (the app's live connection is a WebSocket at
`/envoy`); leave the TLS keys out. Calls need UDP 8444 forwarded on the
router straight to the host and `SIGIL_MEDIA_PUBLIC` set to your public
IP or a DNS-only name (`sigil.example.com:8444`; it is resolved once at
start-up, so restart after an address change, and a name behind
Cloudflare's proxy resolves to Cloudflare, not to you). Without it the
server guesses the address of its own network card, which inside a
container is a private address nobody outside can reach.

Invite codes, when registration is `invite`:

```
docker compose exec sigil sigil-server --config /data/sigil.toml invite
```

## Signing in with Pocket ID (the OIDC gate)

With `registration = "oidc"` an ID token from your identity provider takes
the invite code's place: the app opens the provider's login page in the
browser, the provider sends the browser back to the app on the device
(`http://127.0.0.1:44713/callback`), and the app hands the resulting token
to the server inside the registration bag. The server checks it against
the provider's published keys (signature, issuer, audience, expiry) and
remembers only which login holds which name: one login, one name. The
provider learns that a login happened; it never sees a key, a message or
a conversation (design B24 in `docs/blind-backend.md`).

In Pocket ID: *OIDC Clients → Add*, name it Sigil, tick **Public Client**
(PKCE, no secret), and give it two callback URLs,
`http://127.0.0.1:44713/callback` and `http://127.0.0.1:*/callback` (the
second, with the port wildcarded, covers the rare case that the first port
is taken on someone's device). Copy the client id into
`SIGIL_OIDC_CLIENT_ID`, put Pocket ID's address (`https://id.example.com`)
in `SIGIL_OIDC_ISSUER`, set `SIGIL_REGISTRATION=oidc`. Any other provider
(Authentik, Keycloak, Authelia) works the same way: a public client with
PKCE and those two redirect URIs.

The app still asks for a username after the sign-in; the provider's
username is offered as the suggestion. Only registration is gated:
messages, calls, backups and daily tokens carry no login, as before.

`GET /oidc` returns `{"issuer", "client_id"}` when the gate is on (404
otherwise); the server card's flag bit 1 says whether to ask.

## Config

```toml
role = "both"                 # home | envoy | both
hostname = "sigil.example"
listen = "0.0.0.0:8443"
data_dir = "/data"
tls_cert = "/data/fullchain.pem"
tls_key = "/data/privkey.pem"
registration = "invite"       # invite | open | oidc
# oidc_issuer = "https://id.example.com"   # for oidc: the provider …
# oidc_client_id = "sigil"                 # … and Sigil's client id there
tokens_per_day = 2000
jitter_max_ms = 2000          # Envoy: a bag waits up to this long before forwarding
cover_per_minute = 0          # Envoy: dummy writes per minute per server, 0 = off
cover_credentials = true      # Home: let Envoys draw a credential for cover traffic
calls = true                  # Home: run the call forwarding unit
media_udp = "0.0.0.0:8444"    # its UDP socket; publish this port too
# media_public = "203.0.113.5:8444"   # if the container's own address is not reachable
[servers]                     # Envoy: base-URL overrides, for testing
# "sigil.test" = "http://127.0.0.1:8443"
```

The forwarding unit tells participants to send media to `media_public`
when set, otherwise to the address the host would use to reach the
internet, on the `media_udp` port. Behind NAT or a cloud firewall, set it.

## What the store holds

One `redb` file. Slots, envelopes, subscriptions, shelves, blobs, names,
wraps, backups, spent-token ids, issuing keys, Envoy queues. No column
anywhere carries a wall-clock time; expiry is a day number. Logs carry no
client address.
