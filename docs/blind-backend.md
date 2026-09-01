# The Sigil backend: a server that is blind

**Design document. Nothing here is built yet.** This is the plan for replacing
the Matrix homeserver with a Sigil-native backend that is as convenient as
Matrix or iMessage (type a username, hit send; lose your phone, type your
password, get everything back) while the servers learn as close to nothing as
that convenience allows: not what is said, not who talks to whom, not how
many devices you own, not when you are online.

Scope is the backend and the engine's transport layer. The frontends do not
change: they speak the socket protocol in `core/docs/protocol.md`, which says
nothing about Matrix. SigilText does not change. Calls change last.

The document is written twice: **Part A** is the plain-English version and is
the one to read first. **Part B** is the engineering.

---

# Part A: in plain English

## A1. What we are building

A messenger where your address is `@pellinore:sigil.example`, like Matrix,
and where the server at `sigil.example` knows that you exist and what your
name is, and **nothing else**. It does not know who you talk to, what you
say, how many phones you have, whether you are online, or what is in your
backups.

The requirements, in order:

1. **Reach anyone by username.** No QR codes required, no phone numbers.
2. **Recover everything with a password.** New phone, type username and
   password, all your chats come back. Nothing to write down.
3. **Blind servers.** Even a big public server run by someone hostile learns
   nothing about your conversations.
4. **One program to host.** Runs on a home box with one config file.
5. **Convenience wins ties.** Typing indicators, read receipts and link
   previews are on by default and made as private as they can be.

## A2. The mail-slot picture

The server is a wall of numbered mail slots. When you and Bob first connect,
your apps agree on a secret. From that secret both apps calculate "our slot is
number 7,438,221". You drop letters there, Bob picks them up there, and every
so often the apps calculate a new number and move. Nobody ever told the server
that slot 7,438,221 means "Pellinore and Bob". The server sees slots appear,
get used, and go quiet.

The server also has a **front desk**, and this is the part that changed from
the first draft. The front desk has a list of names: `@pellinore` lives here,
and here is the public key to use to start a conversation with them. That
list is the price of "reach anyone by username", and it is the only list of
people the server keeps. The front desk hands out your public key to anyone
who asks. It does not know what they did with it, because the conversation
that follows happens in slots.

## A3. Reaching someone

You type `@bob:other.example`. Your app asks the front desk at
`other.example` for Bob's card, uses it to set up an encrypted conversation,
and drops the first message in Bob's **requests slot**. Bob sees "Pellinore
would like to message you" with the first message, like Signal or Instagram.
Accept, and it is a normal chat. Ignore, and it goes away.

What `other.example` learned: somebody asked for Bob's card, and something
landed in Bob's requests slot. Not who, not from where, not whether Bob
accepted.

## A4. The courier and the clerk

Every request from your app goes in a sealed bag through a **courier** to the
server's **clerk**. The courier sees your internet address and a sealed bag.
The clerk opens the bag, sees a slot number, and never sees where the bag came
from. To connect you to a slot, courier and clerk have to compare notes.

The courier and the clerk are two modes of the same program. A home server
can run both, in which case its operator sees your address and your slots but
still never your name next to a slot, never content, never who you talk to.
The privacy-minded choice is to run the clerk at home and use a public courier
somewhere else, or Tor, which the app has built in.

## A5. Losing your phone

Your app keeps an encrypted backup of everything (your identity, contacts,
chats, and media up to a size you choose) on your server, continuously.
The backup is a safe that needs two things to open: your password, and a
key. The server has the safe but never the key.

The key is cut into pieces. Any two pieces make a working key. The pieces are
held by: your own server, two other servers on the network picked
automatically, any other device you are still signed in on, and optionally
two friends you choose or a code you print. Nobody sees a whole key. Nobody
sees your password: each holder does a small piece of math on a scrambled
copy of your password and gives the result back; your new phone combines two
results into the key. Each holder allows ten attempts and then wipes its
piece, so guessing has to go through the holders and cannot be done in bulk.
Each piece is stored under a random label, not your name, and so is the
backup itself.

New phone: type username, type password, wait a moment, everything is back.
Behind the scenes: your other devices get a warning and there is a delay
before a recovery completes, so a recovery you did not start can be
cancelled.

## A6. What a hostile public server can and cannot learn

Suppose you are `@you:bigpublic.example` and the operator is hostile and
runs both courier and clerk.

They learn: your username, your public key, that your account exists, your
internet address when you connect, that you are connected, how many people
asked for your card, how many requests you received, and how much traffic
the whole server carries. They hold an encrypted backup they cannot open and
one piece of a key that is useless alone.

They do not learn: anything you said, who you talk to, which slots are yours,
how many devices you have, which groups you are in, who is in them, or
anything about people on other servers.

Use a separate courier or Tor and the first list loses "your internet
address" and "that you are connected".

---

# Part B: the engineering

## Contents

- B1. Threat model and what each party sees
- B2. Architecture
- B3. Identity, usernames and the front desk
- B4. Conversations are MLS groups
- B5. Slots: the address is a secret
- B6. Ordering, epochs and rotation
- B7. Requests from strangers
- B8. Many devices, one user
- B9. Waking a device
- B10. Abuse control without identity on the message path
- B11. Media
- B12. Recovery and backup
- B13. Convenience features and what they cost
- B14. Post-quantum
- B15. Calls
- B16. Retention and deletion
- B17. The wire protocol
- B18. The server: one binary
- B19. Fitting it into the engine
- B20. Phases
- B21. Decisions taken and the few left open
- B22. References

## B1. Threat model and what each party sees

Parties: the **home server** (front desk, clerk, backup store, one recovery
share), the **courier** (Envoy), **other servers** (holding shares, or hosting
the people you talk to), and the **network**.

What a single honest-but-curious operator learns, compared with the
alternatives:

| The operator learns… | iMessage | Matrix | Signal | **Sigil** |
|---|---|---|---|---|
| message content | no | no (E2EE rooms) | no | **no** |
| that you sent something | yes | yes | no | **no** |
| that you received something | yes | yes | yes | **no**; the recipient is a random slot |
| you and Bob are in contact | yes | yes | yes, from timing | **no** |
| phone number / email | yes | optional | yes | **never collected** |
| username | Apple ID | yes | if set | **yes**, by design |
| social graph | yes | yes | partly | **no** |
| when you are online | yes | yes | yes | courier only; **no** with a separate courier or Tor |
| IP address | yes | yes | yes | courier only, never beside a slot |
| device count | yes | yes | yes | **no** |
| group membership | yes | yes | no | **no** |
| group size | yes | yes | yes | approximate, per epoch, unlinked to any group |
| push token | yes | yes | yes | courier only, tied to a random handle |
| read receipts, typing | yes | yes | ciphertext | ciphertext, same padding as messages |
| backup contents | with iCloud | key backup only | no | **no** |
| cross-server traffic | n/a | yes, federation | n/a | **none exists** |

The coalition of courier and clerk can link an IP to the slots it touches
and rebuild a social graph over time; the split, or Tor, is the defence.
A global passive adversary correlating timing on every link is out of scope
for the default tier and addressed by the cover-traffic tier (B13).

## B2. Architecture

```
                 sees IP, sees sealed bags            sees slots, never an IP
                 ┌────────────────────┐              ┌────────────────────────┐
   sigil-engine  │       ENVOY        │              │      HOME SERVER       │
   ─────────────►│  TLS terminator    │─────────────►│  front desk (names)    │
   (Rust daemon, │  oblivious relay   │  bags: HPKE  │  slots (clerk)         │
    holds all    │  push fan-out      │  to the      │  key-package shelves   │
    keys)        │  batching + jitter │  server key  │  blob store            │
                 └────────────────────┘              │  backup store          │
        ▲                                            │  recovery share holder │
        │ or arti (Tor) instead of an Envoy          │  token issuer          │
        └───────────────────────────────────────────►└────────────────────────┘
```

One binary, `sigil-server`, roles `home`, `envoy`, or `both`.

- **Envoy**: Oblivious HTTP style relay (RFC 9458). Clients seal each
  request to the home server's public key with HPKE (RFC 9180); the Envoy
  forwards ciphertext. It sees IPs and bag sizes. It holds push tokens and
  wake channels keyed by random handles it invents.
- **Home server**: the authoritative front desk for the names it hosts, a
  slot store, a blob store, a backup store, a recovery share holder for its
  own users and for users of other servers, and a token issuer. It never
  accepts a direct client connection; everything arrives as a sealed bag.

**No federation.** `@alice:a.example` messaging `@bob:b.example` means
Alice's engine writes into a slot on whichever server hosts that
conversation, through Alice's Envoy. Servers never talk to each other about
users, rooms or messages. The only server-to-server relationship is holding a
recovery share for someone, which is a blob under a random label.

## B3. Identity, usernames and the front desk

A user is a key pair plus a name:

```
identity  = Ed25519 signing key            (long-term; on every device)
          + X25519 + ML-KEM-768 KEM keys   (hybrid; for key packages)
username  = @localpart:server               (registered at the front desk)
```

**The front desk** is the one table of people a server keeps:

```
localpart → ContactCard {
  username, identity_pub, slot_server (where this user's conversations
  are hosted, normally the same server), requests_slot_hint, key_package_shelf,
  signature by identity key
}
```

Registration: the client proves possession of the identity key and claims a
free localpart. A server may require an invite code, a proof-of-work, or
nothing. Lookup: anyone fetches `@bob:b.example` from `b.example`'s front
desk, through their own Envoy, so `b.example` learns "a card was fetched"
and not by whom. Changing a username re-signs the card; the old name is held
for 30 days and then freed.

**What is deliberately not on the front desk**: devices, contacts, groups,
presence, last-seen, avatars (avatars travel inside conversations), and any
link from the name to any slot.

**Key packages** are one-time hybrid-KEM public keys (MLS's prekeys) that let
someone add you to a conversation while you are offline. Each device keeps a
shelf on the home server at `H("sigil/kp/v1" ‖ identity_pub)`, encrypted
under `HKDF(identity_pub, "kp")`, so only someone who already has your card
can use them. The server sees a shelf drain and refill and can count the
drains; clients refill in fixed batches.

## B4. Conversations are MLS groups

Every conversation, including a two-person direct message, is an MLS group
(RFC 9420) whose members are *devices*. Your laptop, your phone and Bob's
phone are three leaves of one tree.

Why MLS rather than a Double Ratchet per device pair: one ciphertext per
message however many devices are listening (fan-out leaks device counts);
forward secrecy and post-compromise security on every commit; membership and
key rotation are cryptographic operations the server neither sees nor
participates in; and the **exporter** gives every member an identical secret
per epoch, which is exactly what a secret slot address needs.

The server is MLS's "Delivery Service" in name only: it stores ciphertext in
slots and orders it. It holds no group state and no notion that a group
exists. Group authority (who may add, remove, rename, pin) is a signed policy
document inside the group, enforced by clients when validating commits.

## B5. Slots: the address is a secret

Each epoch yields, for every member, the same values:

```
slot_seed  = MLS-Exporter("sigil/slot/v1", group_id, 32)
read_cap   = HKDF(slot_seed, "read")
write_key  = Ed25519-from-seed(HKDF(slot_seed, "write"))
address    = H(read_cap ‖ write_pub)
```

- **Write**: `(address, write_pub, envelope, sig)`. The first write pins
  `write_pub`; later writes must verify against it. The address is 256 bits of
  secret, so nobody squats on it first.
- **Read**: `(read_cap, write_pub, cursor)`. The server checks
  `H(read_cap ‖ write_pub) == address` and returns envelopes after the
  cursor. A writer never presents `read_cap`, so a server that only saw
  writes cannot read.
- The server stores address, pinned write key, a sequence of opaque
  fixed-size envelopes, and a TTL.

The server learns "address `7f3a…` has an authorised writer and some
readers" and cannot connect it to a name, a group, another address, or an IP.

Which server hosts a conversation's slots: the creator's `slot_server`,
recorded in the group's policy document. Members on other servers write to
it through their own Envoys. A group may migrate to another host by commit.

## B6. Ordering, epochs and rotation

Each envelope written to an address receives a sequence number; readers
fetch by cursor. Two members committing at once both write to the same
address; the second is rejected by every client when it fails to validate
against the new epoch, and its author retries. Standard MLS, no server
intelligence.

**Every commit rotates the address.** Members keep reading the old address
until they have caught up, then drop it; the server expires it by TTL.
Clients issue a self-update commit at least daily, so a quiet conversation
still moves every day.

Leak: the number of distinct `read_cap` presentations on one address
approximates the device count in that group for that epoch, unlinked to any
group and reset on every rotation.

## B7. Requests from strangers

Each user has a **requests slot**, `H("sigil/req/v1" ‖ identity_pub ‖
period)`, rotated monthly. To start a conversation, the sender fetches the
card, takes a key package, builds a two-device MLS group (or adds the
recipient to an existing one), and writes the MLS Welcome plus the first
message to the requests slot. Writing costs a token (B10).

The recipient's engine surfaces it as a request with the sender's username
and the first message decrypted. Accept: join the group and reply; the
conversation moves to its own rotating slots. Ignore: discard and forget.
Block: remember the identity key locally and discard future Welcomes from it
silently. Users can also require an invite code inside the Welcome, which
turns the request screen off for strangers entirely.

The server learns: the requests slot at a random address received N Welcomes
this month. Not from whom, not whether any was accepted.

Verification is Sigil's version of safety numbers: the card is signed by
the identity key, the identity key fingerprint can be compared out of band,
and a change of key for a known contact is flagged in the conversation.

## B8. Many devices, one user

Devices are MLS leaves. Linking a new device:

1. the new device shows a QR: its own device key;
2. an existing device scans it, sends the identity secret, the group list and
   the local history over a one-shot MLS group of two;
3. the existing device issues an *Add* commit in every group for the new
   leaf, which rotates every address.

The server never learns a device was added. A lost device is removed by a
*Remove* commit from a surviving device (or, after recovery, from the new
one), after which it cannot compute any new epoch's address.

## B9. Waking a device

1. The engine asks its Envoy to **subscribe** to an address. The bag to the
   server contains `address` and a `wake_handle` the Envoy chose at random.
   The Envoy remembers `wake_handle → this device's channel` (live socket or
   an APNs/FCM token) and forwards the bag.
2. The server records `address → [wake_handle…]`: handles, not devices.
3. On a write, the server emits `wake(wake_handle)` to the Envoy: constant
   size, no address, no content.
4. The Envoy wakes the device (a frame, or an empty push). The device fetches
   through the Envoy.

The Envoy knows "device D has 213 subscriptions and was woken twice". The
server knows "address A has three subscribers". Push tokens live only on the
Envoy, and a device re-randomises its handles on reconnect.

## B10. Abuse control without identity on the message path

Accounts exist (for names and recovery) but the message path never uses
them. Rate limiting is **Privacy Pass** (RFC 9576–9578):

- **Credential**: on registration, the client obtains a blind-issued
  credential by proving it owns an account. Blind, so the server cannot later
  recognise which account a credential belongs to.
- **Daily tokens**: presenting the credential (through the Envoy) yields a
  batch of blind-signed tokens, for example 2,000 a day. Every write, every
  subscription, every blob chunk, every share operation spends one. Tokens
  are single-use and unlinkable to issuance and to each other.

Cross-server: `a.example`'s user writing to a slot on `b.example` needs
`b.example`'s tokens. `b.example` issues a small daily batch to any holder of
a valid credential from a server it accepts (a public allow-list, or open by
default with proof-of-work), so a stranger can send a request but not a
flood. The server learns "a credential from `a.example` drew tokens", never
which account.

## B11. Media

A file is encrypted client-side with a random key, cut into fixed 256 KiB
chunks (the last padded), each uploaded through the Envoy with a token. The
blob id is the hash of the ciphertext; the message carries `(ids[], key)`
inside the MLS envelope. Recipients fetch chunks through the Envoy in random
order with jitter. The server sees identical grey bricks with a TTL and
cannot tell which bricks make a file or which slot they belong to.
Thumbnails, blurhashes and previews are generated on the sending device.

## B12. Recovery and backup

The requirement is: username and password, nothing else, everything back.
The threat is: the server holding the backup guesses the password forever.
The design is **password-authenticated threshold key recovery**:

**The backup.** The engine keeps an encrypted append-only backup on the home
server: identity keys, contacts, group states, message history, and media up
to a user-set cap (default 1 GiB, most recent first). It is encrypted under
`backup_key` and stored under label `H(backup_key, "label")`, so the server
cannot tell which encrypted blob belongs to which user. It is uploaded as
padded chunks through the Envoy like any other blob.

**The key.** `backup_key` is derived from a **threshold OPRF** (RFC 9497
family, t-of-n) over the password:

```
holders: home server, two other servers picked at random on setup,
         each other signed-in device, [optional] two friends, [optional]
         a printed code. Threshold t = 2 (configurable).
setup:   client picks a random OPRF secret k, splits it into n shares
         (Shamir), sends share_i to holder_i under label
         L_i = H("sigil/rec/v1" ‖ username ‖ holder_i ‖ salt),
         with the salt stored on the home server's front desk.
recover: client blinds the password, sends it to each reachable holder,
         each evaluates its share on the blinded value, client unblinds
         and combines any t results into OPRF_k(password), and
         backup_key = HKDF(OPRF_k(password) ‖ password, "backup").
```

Properties, plainly: no holder ever sees the password (blinded) or the key
(a share alone is useless, and combination happens on the client). A holder
that is broken into yields one share under a random label. To attack, an
adversary needs t shares *and* the password; and because evaluation is
online, each holder enforces **ten attempts per label, then wipes its
share**, so the password cannot be brute-forced offline unless the attacker
already holds t shares. Shares are rotated (new `k`, new shares) whenever the
holder set changes and on a schedule.

**Friends** as holders: their app stores a share it cannot use; recovery
sends them an approval prompt. They are a door, not the lock, since the
password is still required. **Printed code** as holder: a QR that *is* a
share, for users who want a piece nobody else holds. **Other devices** as
holders: instant restore when one is signed in.

**Safety net.** A recovery attempt notifies every device still in the
user's groups (via a self-group), waits 24 hours before it completes unless
an existing device approves, and can be cancelled from any existing device.
Once complete, the new device issues *Remove* commits for the lost device in
every group.

**What the home server holds**: an encrypted blob under a random label, one
share under a random label, and the salt. It cannot open the blob, cannot
use the share, and cannot guess the password without t holders cooperating.

**Password change**: re-derive, re-encrypt the backup key envelope (the
backup itself is under a data key wrapped by `backup_key`, so only the wrap
is rewritten), re-split. **Forgotten password**: the same as every
zero-knowledge system, the backup is gone; but a signed-in device can reset
the password without it, because it has the data key.

## B13. Convenience features and what they cost

Defaults follow "convenience wins, with the most privacy that fits".

| Feature | Default | How it stays private |
|---|---|---|
| typing indicators | on | an ordinary 1 KiB envelope, at most one per 5 s |
| read receipts | on | ordinary envelope |
| link previews | on | fetched by the *sender's* engine through the Envoy, embedded in the message; the recipient never contacts the site |
| online status | off | would be a broadcast; offered as "share with this chat" per group |
| contact sync | none | no phone numbers, no address book |
| history on new device | on | from backup or from a linked device, never from a server the group is on |

**Shape and timing.** Envelopes are padded to 1, 4 or 16 KiB; bags to the
Envoy are padded again to fixed sizes; the Envoy holds bags 0–2 s and
forwards shuffled batches. The **paranoid tier** adds Poisson cover traffic
(Loopix style) and Tor via `arti`, per user, opt-in.

## B14. Post-quantum

Every KEM is hybrid: key packages and MLS commits use X25519 + ML-KEM-768
(X-Wing, or the MLS hybrid suites as they finalise); bags to the server are
HPKE with the same hybrid KEM; slot addresses and capabilities are derived
symmetrically and inherit it. Signatures stay Ed25519, with ML-DSA behind
the same trait for later. This matches Signal's PQXDH plus SPQR bar.

## B15. Calls

First release stays on LiveKit, because Sigil already has a working E2EE
call stack: the SFrame key comes from the MLS exporter, the room name is
`MLS-Exporter("sigil/call/v1")`, and media goes through a TURN relay on the
Envoy so the SFU sees the Envoy's IP. Later, a native SFU inside
`sigil-server` (`str0m`) removes the last external service.

## B16. Retention and deletion

Slots expire 30 days after last write or when every subscriber has
acknowledged. Blobs expire 30 days after last fetch. Backups are kept while
the account exists. Deleting an account: leave every group (rotating every
address away), delete the backup and shares (the client knows the labels),
release the name. The front desk entry is the only thing the server deletes
by name.

## B17. The wire protocol

All client traffic is a **bag**: `HPKE-Seal(server_pub, request)` via the
Envoy over TLS or QUIC; the reply is sealed to a client ephemeral key. Bags
are padded to fixed sizes. Inside:

| Request | Fields | Server does |
|---|---|---|
| `name.register` | `localpart, card, sig, [invite]` | claim name |
| `name.lookup` | `localpart` | return card |
| `name.update` | `card, sig` | replace card |
| `slot.put` | `address, write_pub, envelope, sig, token` | pin/verify, append, wake |
| `slot.get` | `read_cap, write_pub, after_seq, limit` | verify, return |
| `slot.ack` | `read_cap, write_pub, seq` | retention |
| `slot.subscribe` | `address, wake_handle, token` | remember handle |
| `kp.put` / `kp.take` | `shelf, blob, sig, token` / `shelf` | refill / pop one |
| `blob.put` / `blob.get` | `chunk, token` / `id` | store / fetch |
| `backup.put` / `backup.get` | `label, chunk, token` / `label, cursor` | store / fetch |
| `share.put` | `label, share, policy, token` | hold a recovery share |
| `share.eval` | `label, blinded` | OPRF-evaluate, count the attempt |
| `share.delete` | `label, proof` | drop a share |
| `token.credential` | `account proof, blinded` | blind-issue credential |
| `token.issue` | `credential, blinded[]` | blind-sign the daily batch |

The Envoy speaks a plain control channel to the client for wakes and push
registration, and receives `wake(handle)` events from servers on a
long-lived stream.

Every identifier the slot, blob, backup and share paths carry is a hash of a
secret or a random handle. Only `name.*` and `token.credential` name an
account, and neither touches a conversation.

## B18. The server: one binary

- **One static Rust binary**, `sigil-server`, roles `home`, `envoy`, `both`.
  No Postgres, no Redis, no auth service, no sync proxy, no reverse proxy
  required.
- **Embedded storage**: `redb` for names, slots, shelves, shares, token
  double-spend sets; a content-addressed directory for blobs and backup
  chunks. Backup is copying two paths.
- **Transport**: `axum` + `rustls` with built-in ACME; `quinn` for the wake
  stream.
- **Config**: one TOML file, under twenty keys. `sigil-server init && sigil-server run`.
- **Sizing**: a Raspberry Pi serves a few thousand users outside media.
- **Logs and metrics** carry counts and latencies only; no field can hold an
  address, label, name or token, enforced by type.

## B19. Fitting it into the engine

```
core/src/
  transport/            NEW  trait Backend { rooms, timeline, send, media, calls, … }
    matrix/             MOVE the matrix-sdk glue behind the trait
    sigil/              NEW
      identity.rs       keys, usernames, contact cards, device linking
      mls.rs            openmls groups, exporter-derived secrets
      slots.rs          address derivation, put/get/ack, cursors
      envoy.rs          bags (hpke), subscriptions, wakes
      requests.rs       the requests slot, accept/ignore/block
      tokens.rs         Privacy Pass credential and daily batches
      blobs.rs          chunking, encryption, upload/fetch
      backup.rs         continuous encrypted backup
      recovery.rs       threshold OPRF, holders, safety net
      cover.rs          padding, jitter, cover traffic
      store.rs          local sqlite
server/                 NEW  sigil-server
```

Frontends keep speaking `rooms.list`, `room.open`, `message.send`; the item
shapes in `core/docs/protocol.md` are backend-neutral. `login.start`
becomes `account.create{server, localpart, password}` and
`account.recover{username, password}`; the recovery-key flow becomes device
linking.

Crates: `openmls`, `ml-kem` + `x25519-dalek` (`x-wing` as it lands), `hpke`,
`ed25519-dalek`, `voprf` (threshold variant to be written on top of it),
`privacypass`, `arti-client`, `redb`, `rusqlite`, `axum`, `rustls`,
`rustls-acme`, `quinn`, later `str0m`.

## B20. Phases

**0. Specification.** Freeze this into a wire spec with test vectors:
address derivation, bag format, padding, token issuance, share format.

**1. Home server.** Names, slots, shelves, blobs, tokens; `redb`; ACME. A
command-line client exchanges padded envelopes through addresses from a
shared secret.

**2. Client core.** `transport::sigil`: identity, `openmls` on a hybrid
suite, exporter-derived slots, requests slot, local store. Two engines hold a
direct message by username. Omarchy frontend unchanged.

**3. Devices and wake.** Linking; the Envoy role with subscriptions, handles
and push; Android receives an empty push and fetches.

**4. Recovery.** Continuous backup; threshold OPRF shares across home server,
two other servers and linked devices; the 24-hour safety net; password
change. New phone, username, password, everything back.

**5. Groups and media.** Policy documents; add, remove, rename; blob
chunking; retention; group migration between hosts.

**6. Shape.** Padding audit, Envoy jitter, cover traffic, `arti`, the
paranoid settings page; friends and printed code as optional holders.

**7. Calls.** Exporter-keyed SFrame on LiveKit behind an Envoy TURN relay;
then the native SFU.

Matrix stays behind the trait as long as it is useful.

## B21. Decisions taken and the few left open

Taken:

- **Usernames are `@name:server`**, hosted by a front desk that holds the
  name and the public key and nothing else about the person.
- **MLS for everything, including direct messages.**
- **Slot addresses come from the epoch secret**, never from an identifier.
- **The message path never carries an account**; abuse control is tokens.
- **No federation**; clients write straight to the hosting server.
- **Recovery is username + password**, backed by a threshold OPRF over
  holders, with online guess limits, blind labels and a 24-hour safety net.
- **Convenience wins ties**; privacy hardening is opt-in.
- **Hybrid post-quantum from day one.**
- **One binary, three roles.**

Open, with the recommendation:

1. **Default holders on a one-server network.** Two other servers may not
   exist. Recommendation: the home server ships with a short list of public
   share holders (a share is a blob under a random label, so this is a low
   trust ask), and falls back to "home server + linked device + printed code
   offered at setup".
2. **Large groups.** Above roughly 200 devices one address gets busy.
   Recommendation: raise the self-update interval for large groups and
   revisit only if real usage demands.
3. **Deniability.** MLS messages are signed. Recommendation: accept for v1.
4. **Open registration policy on public servers.** Recommendation: invite
   codes by default, proof-of-work as the open alternative.

## B22. References

- MLS: RFC 9420 (protocol), RFC 9750 (architecture).
- HPKE: RFC 9180. Oblivious HTTP: RFC 9458.
- Privacy Pass: RFC 9576, 9577, 9578.
- OPRF: RFC 9497. Threshold OPRFs: Jarecki, Kiayias, Krawczyk, *Round-Optimal
  Password-Protected Secret Sharing and T-PAKE in the Password-Only Model*
  (2014); Signal, *Secure Value Recovery* and *SVR3* (2023–2024).
- ML-KEM: FIPS 203. X-Wing: Barbosa et al., 2024.
- Signal: PQXDH (2023), SPQR (2025), Sealed Sender (2018), *The Signal Private
  Group System and Anonymous Credentials* (2020).
- Piotrowska et al., *The Loopix Anonymity System* (2017).
- Apple, *iCloud Private Relay Overview*; Apple, *Account Recovery Contact*.
