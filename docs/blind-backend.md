# The Sigil backend: a server that is blind

**Design document. Nothing here is built yet.** This is the plan for replacing
the Matrix homeserver with a Sigil-native backend whose server learns as close
to nothing as the laws of physics allow: not what is said, not who is talking to
whom, not who anyone is, not where anyone is.

Scope is the backend and the engine's transport layer. The frontends do not
change: they speak the socket protocol in `core/docs/protocol.md`, and that
protocol says nothing about Matrix. SigilText does not change. Calls change
last.

---

## Contents

1. [Why](#1-why)
2. [The mailman problem, answered](#2-the-mailman-problem-answered)
3. [Who sees what](#3-who-sees-what)
4. [Architecture](#4-architecture)
5. [Building blocks](#5-building-blocks)
   - 5.1 Identity: keys, not names
   - 5.2 Contact cards and key packages
   - 5.3 Conversations are MLS groups
   - 5.4 Mailboxes: the address is a secret
   - 5.5 Ordering, epochs, and rotation
   - 5.6 Many devices, one user
   - 5.7 Waking a device without knowing whose it is
   - 5.8 Sending without an account: anonymous tokens
   - 5.9 Media: blind blobs
   - 5.10 Handles: a directory that cannot read its own index
   - 5.11 Shape and time: padding, jitter, cover traffic
   - 5.12 Post-quantum
   - 5.13 Calls
   - 5.14 Retention and deletion
6. [The wire protocol](#6-the-wire-protocol)
7. [What is still visible, and the paranoid tier](#7-what-is-still-visible-and-the-paranoid-tier)
8. [The server: one binary](#8-the-server-one-binary)
9. [Fitting it into the engine](#9-fitting-it-into-the-engine)
10. [Phases](#10-phases)
11. [Decisions taken, and the few left open](#11-decisions-taken-and-the-few-left-open)
12. [References](#12-references)

---

## 1. Why

Matrix's homeserver is a database of everything: who is in which room, who
said what when, every device, every read receipt, every membership change, in
plain text if the room is unencrypted and in rich metadata even when it is.
Federation means that database is replicated to every other homeserver with a
member in the room. Running it means running Synapse, Postgres, a
matrix-authentication-service, a sliding-sync proxy, a TURN server, a LiveKit
SFU and an identity server, and any of them can break the rest.

Signal fixed the content problem with the Double Ratchet and the sender
problem with sealed sender. It did not fix the rest: the server still knows
your phone number, your username, your device list, your push token, when you
are online, and, because sealed sender still needs a *recipient*, exactly who
receives every message. iMessage is worse on every row of that list.

The goal here is a backend where the server holds only **opaque blobs at
random addresses**, where no table on it maps to a person, and where a
subpoena for "everything about Alice" returns nothing because the server has
no idea which of its blobs, if any, are Alice's. And it has to run as one
process on one home machine.

## 2. The mailman problem, answered

> "How does the mailman get the mail to the mailbox if the mailman doesn't
> know where the mailbox is?"

The mailman does not need to know *whose* mailbox it is. He only needs to know
*which slot*. Three tricks make the slot meaningless to him:

**Trick 1: mailboxes have numbers, not names.** The server is a wall of
numbered slots. There is no list of residents. There is no registration.
Anyone can put an envelope in any slot if they know its number, and anyone
who holds the slot's key can empty it.

**Trick 2: the two friends pick the number using a secret only they share,
and they pick a new number for every conversation epoch.** Alice and Bob
agree on a secret once (the same handshake that sets up encryption). From that
secret both can compute "our next slot is 7f3a…". Alice drops the letter there.
Bob checks there. The number is unguessable, was never sent to anyone, and the
next one will be unrelated. The server sees a slot get filled and emptied and
has no way to connect it to the one before or to any other slot.

**Trick 3: the courier who knows the street never sees the slot number, and
the mailman who sees the slot number never sees the street.** Every request
goes through a relay (the *Envoy*) that sees the sender's IP address but only
an encrypted bag. The storage server (the *Vault*) opens the bag, sees the slot
number and the sealed envelope, and never sees where the bag came from. To
link a person to a slot you need both parties to collude.

Everything else in this document is the engineering to make those three
tricks hold up against a hostile operator, a hostile network, spam, phones
that sleep, users with five devices, and groups of five hundred.

## 3. Who sees what

What the *server operator* can learn about a conversation between Alice and
Bob, by system. "Yes" means the operator learns it in the normal course of
running the service.

| The operator learns… | iMessage | Matrix | Signal | **Sigil** |
|---|---|---|---|---|
| message content | no | no (E2EE rooms) | no | **no** |
| that Alice sent *something* | yes | yes | no (sealed sender) | **no** |
| that Bob received something | yes | yes | yes | **no** — the recipient is a random slot |
| Alice and Bob are in contact | yes | yes | yes, from recipient + timing | **no** |
| Alice's phone number / email | yes | optional | yes | **never collected** |
| a username | Apple ID | yes, `@alice:server` | yes, if set | **optional, and the server cannot read its own index** (5.10) |
| the social graph | yes | yes | partially | **no** |
| when Alice is online | yes | yes | yes | **no** — the Vault sees no connections, the Envoy sees no identities |
| Alice's IP address | yes | yes | yes | **Envoy only**, never together with anything else |
| how many devices Alice has | yes | yes | yes | **no** |
| the members of a group | yes | yes | no (zkgroups) | **no** |
| the size of a group | yes | yes | yes | approximately, per epoch, without knowing which group (5.5) |
| a push token | yes | yes | yes | **Envoy only**, tied to a random handle |
| that a message was read | yes | yes | ciphertext | **ciphertext**, same padding as a message |
| who talks to whom across servers | n/a | yes, federation | n/a | **no**; there is no server-to-server traffic at all (4) |

The honest remainder, meaning what a Vault operator *can* still see, is in
section 7. It is: the number of slots that exist, how much traffic flows, and
coarse timing.

## 4. Architecture

```
                 sees IP, sees nothing else          sees slots, sees no IPs
                 ┌────────────────────┐              ┌────────────────────────┐
   sigil-engine  │       ENVOY        │              │         VAULT          │
   ─────────────►│  TLS terminator    │─────────────►│  numbered slots        │
   (Rust daemon, │  oblivious relay   │  bags: HPKE  │  key-package shelves   │
    holds all    │  push fan-out      │  to the      │  blob store            │
    keys)        │  batching + jitter │  Vault's key │  token issuer          │
                 └────────────────────┘              │  handle directory      │
                                                     └────────────────────────┘
        ▲                                                        ▲
        │ optional: arti (Tor) instead of, or before, the Envoy  │
        └────────────────────────────────────────────────────────┘
```

Two roles, one binary (`sigil-server --role vault|envoy|both`).

- The **Envoy** is an Oblivious HTTP style relay (RFC 9458). The client
  encrypts each request to the Vault's public key with HPKE (RFC 9180), and
  the Envoy forwards the ciphertext. It terminates TLS, so it sees IPs and
  request sizes and nothing else. It also holds push tokens and long-lived
  wake channels, keyed by random handles it invents.
- The **Vault** decrypts bags, stores slots and blobs, issues tokens and
  answers directory lookups. It never accepts a direct client connection. It
  sees slot numbers, opaque envelopes, and blind tokens.

**There is no federation.** A conversation between Alice on `vault-a` and Bob
on `vault-b` works because Alice's engine writes straight into Bob's slots on
`vault-b`, through Alice's Envoy, and Bob's writes into hers. Vaults never
talk to each other, never replicate, never need to trust each other. A "server
outage" only affects the slots on that Vault, and a user may list several
Vaults in their contact card for redundancy.

**Split trust is optional but cheap.** Running `--role both` on one machine
collapses the split and the operator sees IPs *and* slots (still no names, no
content, no graph). The intended deployment is: run your own Vault at home,
and point the engine at any community Envoy, or at Tor. An Envoy is
stateless apart from wake channels and can be run by anyone; it cannot read
anything.

## 5. Building blocks

### 5.1 Identity: keys, not names

A user *is* a key pair. There is no account, no registration, no user table.

```
identity  = Ed25519 signing key            (long-term, on every device)
          + X25519 + ML-KEM-768 KEM keys   (hybrid, for key packages)
```

The identity key is created on the first device and shared with the user's
other devices during linking (5.6). Its fingerprint is what a QR code carries
and what a safety-number comparison checks. Locally, contacts get nicknames;
the server never sees a nickname.

Nothing on the server is indexed by the identity key in the clear. Wherever
the design needs "an address only people who know Bob's key can find", it
uses `H(IK_pub ‖ purpose)` for the address and `HKDF(IK_pub, purpose)` for an
encryption key, so the Vault holds ciphertext at a hash and learns neither the
key nor who is asking.

### 5.2 Contact cards and key packages

A **contact card** is what you hand someone so they can start a conversation:

```
ContactCard {
  identity_pub,                    // Ed25519
  vaults: ["vault.example", …],    // where my slots live
  welcome_slot_hint,               // see below
  signature                        // by identity key
}
```

It travels as a QR code, a `sigil:` link, or a directory record (5.10).

**Key packages** are the MLS equivalent of Signal's prekeys: one-time,
signed, hybrid-KEM public keys that let someone add you to a group while you
are offline. Each device publishes a shelf of them on its Vault:

```
shelf address  = H("sigil/kp/v1" ‖ identity_pub)
shelf contents = Enc_{HKDF(identity_pub, "kp")}( [KeyPackage…] )
```

Anyone holding the contact card can compute the address and the key, take one
package (the Vault removes it), and decrypt it. The Vault sees a shelf at a
random address being drained and refilled. It can count the drains, which is
"how many conversations were started with this person this month"; clients
blunt that by refilling in fixed batches and occasionally draining their own.

The **welcome slot** is a fixed inbox for MLS Welcome messages, derived the
same way (`H("sigil/welcome/v1" ‖ identity_pub ‖ period)`), rotated monthly
by the `period` counter. Writing to it costs a token (5.8), and the client can
require an invite code inside the Welcome before it accepts a stranger.

### 5.3 Conversations are MLS groups

Every conversation, including a two-person direct message, is an **MLS group**
(RFC 9420) whose members are *devices*. Alice's laptop, Alice's phone and
Bob's phone are three leaves of one tree.

Why MLS rather than a Double Ratchet per device pair:

- one ciphertext per message regardless of member count (Signal's fan-out is
  one per device, and the count of writes leaks device counts);
- forward secrecy and post-compromise security in every commit;
- membership, admin, and key rotation are cryptographic operations the
  server does not participate in and cannot see;
- the **exporter** (`MLS-Exporter(label, context, len)`) gives every member an
  identical secret per epoch, which is exactly what trick 2 needs;
- hybrid post-quantum cipher suites exist for it today (5.12).

The server is MLS's "Delivery Service" in name only. It stores ciphertext in
slots and orders it. It holds no group state, no membership list, and no
notion that a group exists.

Group *authority* (who may add or remove members, rename the group, pin
messages) is a signed policy document inside the group, enforced by clients
when validating commits. The server is not asked.

### 5.4 Mailboxes: the address is a secret

Each MLS epoch yields, for every member, the same three values:

```
slot_seed  = MLS-Exporter("sigil/slot/v1",  group_id, 32)
read_cap   = HKDF(slot_seed, "read")            // proves the right to read
write_key  = Ed25519-from-seed(HKDF(slot_seed, "write"))
address    = H(read_cap ‖ write_pub)            // the slot number
```

- To **write**, a member sends `(address, write_pub, envelope, signature)`.
  The first write to an address pins `write_pub`; later writes must verify
  against it. Nobody outside the group can forge that signature, and the
  address itself is 256 bits of secret, so nobody can squat on it first.
- To **read**, a member sends `(read_cap, cursor)`. The Vault checks
  `H(read_cap ‖ write_pub) == address`, then returns envelopes after the
  cursor. Note that a writer never presents `read_cap`, so a Vault that only
  ever saw writes cannot read the slot either.
- The Vault stores: address, pinned write key, a sequence of opaque
  fixed-size envelopes, and a TTL. Nothing else.

The Vault therefore learns "address `7f3a…` has an authorised writer and some
readers". It cannot connect `7f3a…` to a person, to a group, to any other
address, or to the IP the bags came from.

### 5.5 Ordering, epochs, and rotation

MLS commits need a total order per group; application messages do not, but
chat wants one anyway. The slot provides it: each envelope written to an
address receives a sequence number from the Vault, and readers fetch by
cursor. Two members committing at once both write to the same address; the
one that lands second is rejected by every client when it fails to validate
against the new epoch, and its author retries. That is the standard MLS
strategy and needs no server intelligence.

Every commit starts a new epoch, so **every commit rotates the address**.
Members keep reading the old address until they have caught up, then drop it;
the Vault expires it by TTL. Clients also issue a self-update commit at least
daily (MLS recommends this for post-compromise security), so a quiet
conversation still moves to a fresh address every day. The Vault sees
addresses appear, receive a burst, and go dark, with nothing tying one to the
next.

What this leaks: the number of distinct `read_cap` presentations on one
address approximates the number of devices in that group for that epoch.
The Vault cannot say *which* group, and the number resets with every
rotation. For the sensitive case, a direct message, "this address has three
readers" says nothing useful.

### 5.6 Many devices, one user

Devices are MLS leaves, so every device is a first-class member of every
group. Linking a new device is:

1. new device shows a QR: its own device key pair;
2. an existing device scans it, sends the identity secret and the current
   group list over a one-shot MLS group of two;
3. the existing device issues an *Add* commit in every group for the new
   device's leaf, which also rotates every address.

The server never learns a device was added. Contacts do not need to do
anything; the commit is self-describing.

Losing a device means an existing device issues *Remove* commits, which is
post-compromise security in action: the removed device cannot compute any
new epoch's address.

### 5.7 Waking a device without knowing whose it is

Polling every address would cost a request per conversation per interval and
would let the Vault cluster addresses by the burst. Instead:

1. The engine asks its Envoy to **subscribe** to an address. The request is a
   bag (encrypted to the Vault) that contains `address` and a `wake_handle`
   the Envoy chose at random for this subscription. The Envoy remembers
   `wake_handle → this device's channel` (a live connection, or an APNs/FCM
   token) and forwards the bag.
2. The Vault records `address → [wake_handle…]`. It sees handles, not devices,
   and it cannot tell which handles share a device.
3. On a write, the Vault emits `wake(wake_handle)` to the Envoy. No address,
   no content, a constant-size ping.
4. The Envoy wakes the device: a frame on the live socket, or an empty push
   notification. The device then fetches through the Envoy as usual.

Split knowledge, again: the Envoy knows *device D has 213 subscriptions and
was woken twice today*. The Vault knows *address A has three subscribers*.
Nobody knows both. Push tokens live only on the Envoy, tied to nothing but a
handle, and a device re-randomises all its handles when it reconnects.

Subscriptions are per address, so they are re-issued on every rotation; the
subscribe request is one bag, cheap, and batched with the commit itself.

### 5.8 Sending without an account: anonymous tokens

With no accounts there is nothing to rate-limit, so a blind server would be
an open spam relay. The answer is **Privacy Pass** (RFC 9576–9578): blind
signatures that prove "this request is paid for" without saying by whom.

Two levels:

- **Membership credential**, long-lived. Obtained once, from the operator,
  by whatever policy they like: an invite code from an existing member, a
  proof-of-work, a payment, or nothing at all on an open Vault. Issued blind,
  so the operator who handed out the invite cannot recognise the credential
  later.
- **Daily tokens**. Presenting the credential (through the Envoy, so with no
  IP) yields a batch of blind-signed tokens, for example 2,000 per day.
  Every write, every subscription, every blob chunk spends one. Tokens are
  single-use and unlinkable to issuance and to each other.

The Vault learns "credential C drew its tokens today" and "some token was
spent on some slot". The two facts cannot be joined. Version 2 replaces the
pseudonymous credential with rate-limited anonymous credentials (the
ARC / Privacy Pass rate-limited issuance work) so that even the daily draw is
unlinkable; the interface does not change.

Reads are free but bounded by the subscription that made them possible, which
was paid for.

### 5.9 Media: blind blobs

A file is encrypted client-side with a random key, cut into fixed 256 KiB
chunks (the last one padded), and each chunk uploaded through the Envoy with
a token. The blob id is the hash of the ciphertext. The message carries
`(ids[], key)` inside the MLS envelope. Recipients fetch chunks through the
Envoy in random order with jitter.

The Vault sees content-addressed ciphertext chunks of one size with a TTL.
It cannot tell which chunks make a file, which file belongs to which slot,
or which slot to which chunk fetch.

Thumbnails, blurhashes and previews are generated on the sending client and
travel inside the message, which the engine already does for Matrix.

### 5.10 Handles: a directory that cannot read its own index

Usernames are optional. Discovery by QR code or link needs no directory. For
users who want `@pellinore` to be findable, the directory is built so that
the server holding it cannot read it and cannot see what anyone looks up.

- **Storage**: record `H(OPRF_k(handle)) → Enc_{HKDF(handle)}(ContactCard)`.
  The key `k` is the directory's OPRF key (RFC 9497). The record is
  encrypted under a key derived from the handle itself.
- **Lookup**: the client *blinds* the handle, the directory evaluates the
  OPRF on the blinded value, the client unblinds and hashes to get the
  record address, fetches the record, and decrypts it because it knows the
  handle. The directory never sees the handle, in either direction.
- **Claiming**: writing a record costs a token and the record is signed by
  the identity key. First-come, and a claimer must prove knowledge of the
  handle it claims, so the directory cannot mint records for names it has
  not been shown.

What a single operator can do: run its own OPRF over a dictionary of likely
handles offline and recover the cards for guessable names. That is the same
exposure as a public phone book, and it is why handles are optional. Closing
it is a two-party OPRF with the key split between the Envoy operator and the
Vault operator, which is on the roadmap and changes nothing on the wire.

Address-book upload is not a feature and will not become one.

### 5.11 Shape and time: padding, jitter, cover traffic

Content is hidden; *shape* and *timing* are the remaining side channels.

- **Padding.** Every envelope is padded to a bucket: 1 KiB, 4 KiB or 16 KiB.
  Read receipts, typing notices, reactions, edits and text all land in the
  1 KiB bucket and are indistinguishable. Bags to the Envoy are padded again
  to fixed sizes so the Envoy cannot tell a fetch from a write.
- **Jitter.** The Envoy holds each bag for a random 0–2 s and forwards in
  shuffled batches, so "a write, then three reads" is not a visible pattern
  at the Vault.
- **Typing indicators** are the worst offender in every messenger: a stream of
  precisely-timed tiny messages. They are off by default, rate-limited to one
  per 5 s when on, and use the same envelope as everything else.
- **Cover traffic** (the paranoid tier, section 7): the engine emits dummy
  writes to dummy addresses and dummy reads on a Poisson schedule, Loopix
  style, so that real activity is hidden in a constant background rate.
  Costs tokens and battery; opt-in.

### 5.12 Post-quantum

Signal's PQXDH (2023) made the initial handshake quantum-safe; their SPQR
ratchet (2025) made the ongoing ratchet quantum-safe too. Sigil sets the bar
at "every secret that protects a message is hybrid":

- key packages and MLS commits use a **hybrid X25519 + ML-KEM-768 cipher
  suite** (X-Wing, or the MLS hybrid suites as they finalise), so every epoch
  is protected against harvest-now-decrypt-later;
- bags to the Vault are HPKE with the same hybrid KEM;
- slot addresses, read capabilities and write keys are derived symmetrically
  from the epoch secret, so they inherit its security for free;
- signatures stay Ed25519 for now (a quantum forger needs to act live; a
  quantum decryptor can act in ten years), with ML-DSA available behind the
  same trait when the ecosystem settles.

### 5.13 Calls

Calls stay on LiveKit for the first release, because Sigil already has a
working end-to-end-encrypted MatrixRTC stack, and the changes are contained:

- the SFrame key comes from the MLS exporter instead of Matrix key events;
- the LiveKit room name is `MLS-Exporter("sigil/call/v1")`, so the SFU sees a
  random room, not a group;
- media goes through a TURN relay on the Envoy, so the SFU sees the Envoy's
  IP, not participants'.

What the SFU still learns is "N random peers were in random room X for
12 minutes". The later step is a Sigil-native SFU inside `sigil-server`
(`str0m` or `webrtc-rs`), which removes the last external service.

### 5.14 Retention and deletion

- Slots expire 30 days after their last write, or when every subscriber has
  acknowledged, whichever is first; acknowledgement is a bag with `read_cap`
  and a cursor.
- Blobs expire 30 days after last fetch, extended by fetching.
- Key package shelves are refilled by their owner and never expire.
- A user "deleting their account" is a client-side operation: leave every
  group (a commit, which rotates every address away from you), drain your
  own shelves, and forget the identity key. The server had nothing to delete.
- History lives on devices. A new device receives history from an existing
  device during linking, encrypted device to device, not from the server.

## 6. The wire protocol

All client traffic is a **bag**: `HPKE-Seal(vault_pub, request)`, POSTed to
the Envoy over TLS or sent as a QUIC stream; the Envoy forwards to the Vault,
which returns `HPKE-Seal(client_ephemeral, response)`. Bags are padded to
fixed sizes. Inside a bag, one of:

| Request | Fields | Vault does |
|---|---|---|
| `slot.put` | `address, write_pub, envelope, sig, token` | pin/verify writer, append, assign seq, wake subscribers |
| `slot.get` | `read_cap, write_pub, after_seq, limit` | verify address, return envelopes |
| `slot.ack` | `read_cap, write_pub, seq` | mark read for retention |
| `slot.subscribe` | `address, wake_handle, token` | remember handle for wake |
| `kp.put` | `shelf, blob, sig, token` | replace shelf contents |
| `kp.take` | `shelf` | pop one package |
| `welcome.put` | `address, envelope, token` | append to welcome inbox |
| `blob.put` | `chunk, token` | store by hash |
| `blob.get` | `id` | return chunk |
| `token.issue` | `credential, blinded[]` | blind-sign the batch |
| `dir.eval` | `blinded_handle` | OPRF evaluation |
| `dir.put` / `dir.get` | `record_addr, record, sig, token` / `record_addr` | store / fetch |

The Envoy additionally speaks a plain (non-bag) control channel to the
client for wakes, and registers push tokens against wake handles. Its
Vault-facing side receives `wake(handle)` events on a long-lived stream.

Nothing in the protocol carries an identity, a display name, a group id, or a
device id. Every identifier the Vault ever sees is either a hash of a secret
or a random number the Envoy made up.

## 7. What is still visible, and the paranoid tier

Honest accounting of what a **single honest-but-curious operator** learns:

| Vault | Envoy |
|---|---|
| number of live slots, per-slot writes and readers | client IPs and when they connect |
| total traffic volume and its daily rhythm | per-device count of subscriptions and wakes |
| key-package drain counts per shelf | bag sizes (fixed) and timing (jittered) |
| which credential drew tokens today | push tokens |

And what a **coalition of Envoy and Vault** learns: the link from an IP to
the slots it touches, from which the social graph can be rebuilt over time.
That is the one attack this architecture does not defeat by itself; it is
defeated by not letting one party be both.

The **paranoid tier** in the engine's settings turns on, per user:

1. **Tor instead of an Envoy**, through `arti`, embedded in the engine. The
   Envoy's knowledge collapses to "some Tor exit sent a bag".
2. **Cover traffic** at a chosen rate, so the Vault sees a flat line.
3. **Two-operator OPRF** for the directory, once implemented.
4. **Multiple Vaults** in the contact card, with conversations spread across
   them, so no single Vault holds all of one user's slots.

A global passive adversary watching every link on the internet can still
correlate flows by timing; that is Tor's limit too, and only mixnet-grade
latency defeats it. Sigil makes it *possible* to trade latency for that
protection (cover traffic plus Envoy batching) rather than pretending the
problem is absent.

## 8. The server: one binary

The single hardest requirement after "blind" is "runs at home without
breaking". So:

- **One static Rust binary**, `sigil-server`, roles selected by flag or
  config: `vault`, `envoy`, `both`. No Postgres, no Redis, no reverse proxy
  required, no auth service, no sync proxy.
- **Embedded storage**: `redb` (pure Rust, ACID, single file) for slots,
  shelves, directory records and token double-spend sets; a content-addressed
  directory of files for blob chunks. Backup is copying two paths while the
  server runs; restore is copying them back.
- **Transport**: HTTPS via `axum` with `rustls` and built-in ACME so a bare
  box gets a certificate by itself, and QUIC via `quinn` for the wake stream.
- **Config**: one TOML file, under twenty keys, all with defaults. A fresh
  install is `sigil-server init && sigil-server run`.
- **Resource envelope**: a slot is a few hundred bytes plus its envelopes; a
  busy user generates tens of kilobytes a day outside media. A Raspberry Pi
  serves a few thousand users. Token verification is one Ed25519 or RSA-blind
  check per write, which is nothing.
- **Nothing to moderate, nothing to migrate**: there are no schemas with
  people in them. A version bump migrates slot formats and that is all.
- **Observability without surveillance**: metrics are counts and latencies
  only, and the log line format is reviewed so that no field can carry an
  address, a handle or a token.

## 9. Fitting it into the engine

The engine already splits the world into "what the frontends see" (the
socket protocol) and "how it is obtained" (`matrix-sdk` calls). The work is
to make the second half pluggable.

```
core/src/
  transport/            NEW  trait Backend { rooms, timeline, send, media, calls, … }
    matrix/             MOVE  the existing matrix-sdk glue behind the trait
    sigil/              NEW  the blind backend
      identity.rs       keys, contact cards, device linking
      mls.rs            openmls groups, exporter-derived secrets
      slots.rs          address derivation, put/get/ack, cursors
      envoy.rs          bags (hpke), subscriptions, wake handling
      tokens.rs         Privacy Pass credential + daily batches
      blobs.rs          chunking, encryption, upload/fetch
      directory.rs      OPRF lookup and claim
      cover.rs          padding buckets, jitter, cover traffic
      store.rs          local sqlite: groups, history, pending sends
server/                 NEW  sigil-server (vault, envoy)
```

Frontends keep speaking `rooms.list`, `room.open`, `message.send` and the
rest; the item shapes in `core/docs/protocol.md` are already backend-neutral.
The recovery-key flow becomes device linking. `login.start{homeserver}`
becomes `login.start{vault}` or `identity.create` for a brand-new key.

Crates, all pure Rust, all already maintained:

| Need | Crate |
|---|---|
| MLS | `openmls` (with `openmls_rust_crypto`) |
| hybrid KEM | `ml-kem` (RustCrypto) + `x25519-dalek`; `x-wing` as it lands |
| HPKE bags | `hpke` |
| signatures | `ed25519-dalek` |
| OPRF | `voprf` |
| Privacy Pass | `privacypass` (Cloudflare's Rust implementation) |
| Tor | `arti-client` |
| storage (server) | `redb`; (client) `rusqlite`, as today |
| HTTP / QUIC | `axum`, `rustls`, `quinn`, `rustls-acme` |
| SFU, later | `str0m` |

## 10. Phases

Each phase ends with something that runs.

**Phase 0 — Specification.** Freeze this document into a wire spec with test
vectors: address derivation, bag format, envelope padding, token issuance.
Write the threat model as a checklist so every later change is judged against
it.

**Phase 1 — The Vault.** `sigil-server --role both` with slots, shelves,
blobs and tokens, `redb` storage, ACME. A command-line test client that
proves two processes can exchange padded envelopes through addresses derived
from a shared secret. No MLS yet; shared secret from a test vector.

**Phase 2 — The client core.** `transport::sigil` with identity, `openmls`
groups on a hybrid suite, exporter-derived slots, put/get/ack, local store.
Two engines on one machine hold a direct message end to end. The Omarchy
frontend works unchanged against it.

**Phase 3 — Devices and wake.** Device linking; the Envoy as a separate role
with subscriptions, wake handles, and push tokens; Android receives an empty
push and fetches. Multi-device conversations rotate on link and unlink.

**Phase 4 — Groups and media.** Group policy documents; add, remove, rename
as commits; blob chunking; the large-group path; retention.

**Phase 5 — Discovery.** Contact cards as QR and `sigil:` links; the OPRF
directory; the welcome inbox with invite codes.

**Phase 6 — Shape.** Padding buckets audited, Envoy jitter, cover traffic,
`arti` integration, the paranoid settings page.

**Phase 7 — Calls.** Exporter-keyed SFrame on LiveKit behind an Envoy TURN
relay; then the native SFU.

Matrix support stays in the engine behind the trait for as long as it is
useful; nothing forces a cutover date.

## 11. Decisions taken, and the few left open

Taken, so they do not get re-argued:

- **MLS for everything, including direct messages.** Not a per-pair Double
  Ratchet. Reason: one ciphertext, tree-based groups, and the exporter.
- **Slot addresses come from the epoch secret**, not from any identifier.
  This is the whole design; if it ever needs an identifier, the design is
  wrong.
- **No accounts, ever.** Abuse control is tokens, not identity.
- **No federation.** Clients write directly to the recipient's Vault.
- **Split trust as two roles of one binary**, so home users are not forced to
  run two machines and privacy-minded users are not forced to trust one.
- **Handles are optional** and the directory is OPRF-blinded.
- **Hybrid post-quantum from day one** in every KEM.
- **History is on devices**, not the server.

Left open, with the recommendation:

1. **Large groups.** Above roughly 200 devices the per-epoch address gets
   busy and commit contention rises. Recommendation: keep one address, raise
   the self-update interval for large groups, and revisit sender keys only if
   real usage demands it.
2. **Deniability.** MLS messages are signed, so they are not deniable the way
   Signal's are. Recommendation: accept for v1; MLS deniability proposals
   exist and slot in later.
3. **Who runs public Envoys.** Technically anyone; socially, a list of
   community Envoys in the client is a curation job. Recommendation: ship
   with Tor as the built-in "no trust needed" option and add a curated list
   when there are operators to list.
4. **Token economics on open Vaults.** A fully open Vault with free
   credentials will be spammed. Recommendation: default to invite codes, and
   offer proof-of-work as the open alternative.

## 12. References

- MLS: RFC 9420, *The Messaging Layer Security Protocol*; RFC 9750,
  architecture.
- HPKE: RFC 9180.
- Oblivious HTTP: RFC 9458.
- Privacy Pass: RFC 9576 (architecture), RFC 9577 (auth scheme), RFC 9578
  (issuance).
- OPRF: RFC 9497, *Oblivious Pseudorandom Functions using Prime-Order
  Groups*.
- ML-KEM: FIPS 203; X-Wing: *X-Wing: The Hybrid KEM You've Been Looking
  For* (Barbosa et al., 2024).
- Signal: *The PQXDH Key Agreement Protocol* (2023); *The Triple Ratchet /
  SPQR* (2025); *Sealed Sender* (2018); *The Signal Private Group System and
  Anonymous Credentials* (Chase, Perrin, Zaverucha, 2020).
- Piotrowska et al., *The Loopix Anonymity System* (USENIX Security 2017),
  for Poisson cover traffic and batching.
- Apple, *iCloud Private Relay Overview*, for the two-hop split-trust model in
  production at scale.
