# sigil-client

The Sigil client core as a library, plus `sigil-cli`. Identity, account
setup, the Envoy link, MLS conversations, slots, requests and a local store.
The engine wraps this behind its backend trait; the CLI drives it directly.

Design: [`docs/blind-backend.md`](../docs/blind-backend.md). Wire protocol:
[`docs/spec/sigil-wire-v1.md`](../docs/spec/sigil-wire-v1.md).

## What works

- `init`, `register`: identity, name registration by invite, blind
  credential, daily tokens, ten key packages sealed on the shelf.
- `dm @name:server "text"`: takes a key package, creates an MLS group,
  sends the Welcome and first message through the recipient's requests
  slot, sealed to their identity key.
- `requests --accept`: proves ownership of the requests slot, receives
  the Welcome, checks the sender's credential against their card, joins.
- `send`, `listen`: application messages through the epoch-derived slot,
  backfill deduped against live delivery, own messages read back from the
  local record, commits merged and the address rotated.

- `link-offer`, `link-scan`: add a device. The new device shows an offer,
  the existing one scans it, both show seven emoji, the existing device
  confirms and transfers the identity, tokens and conversations, then adds
  the new device's MLS leaf to every conversation.

State is two JSON files next to each other: the account (`<name>.json`)
and the MLS store (`<name>.mls.json`). Both hold secrets in Phase 2; the
engine's keystore takes them over in Phase 2b.

## Test

```
(cd ../server && cargo build) && cargo build && tests/e2e.sh
```
