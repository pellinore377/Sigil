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
call forwarding unit on one UDP port. Not yet: ACME, OIDC, APNs and FCM,
open rooms, the client-side TPM session, a relay for UDP-blocked calls.

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

## Docker

```
docker build -f server/Dockerfile -t sigil-server .      # from the repo root
docker compose -f server/docker-compose.yml run --rm sigil --config /data/sigil.toml init --hostname sigil.example
docker compose -f server/docker-compose.yml up -d
```

Set `tls_cert` and `tls_key` in `data/sigil.toml` to PEM files inside the
volume. Until ACME lands, a certificate from any ACME client works.

## Config

```toml
role = "both"                 # home | envoy | both
hostname = "sigil.example"
listen = "0.0.0.0:8443"
data_dir = "/data"
tls_cert = "/data/fullchain.pem"
tls_key = "/data/privkey.pem"
registration = "invite"       # invite | open
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
