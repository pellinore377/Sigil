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

Mail comes back the same way, in reverse: when a letter lands in a slot
you are watching, the clerk hands it to the courier under a random ticket
number, and the courier hands it to your phone. If your phone is off, the
courier holds the sealed letter until it is back. The clerk never sees you
come and go.

The courier and the clerk are two modes of the same program. A home server
can run both, in which case its operator sees your address and your slots but
still never your name next to a slot, never content, never who you talk to.
The privacy-minded choice is to run the clerk at home and use a public courier
somewhere else, or Tor, which the app has built in.

## A5. Losing your phone

Your app keeps an encrypted backup of everything (your identity, contacts,
chats, and media up to a size you choose) on your server, continuously. The
backup opens with two things together: your **password** and a **recovery
key**. Guessing your password gets nowhere without the key, and stealing the
key gets nowhere without your password.

There are three ways back in, and the app uses whichever you have:

**1. Another device you are still signed in on.** This is the fast path and
works the way Google, Signal and WhatsApp add a device. The new phone shows a
QR code, you scan it with your laptop, both screens show an emoji, and if
they match you tap it. That match proves nobody slipped in between the two
devices. The laptop then sends the recovery key and your history across,
sealed. The server sees a sealed bag and never learns a device was added.

**2. Username and password, if your server has a security chip.** Nearly
every PC made in the last decade has a TPM: a small chip that stores a
secret, releases it only for the right password, and locks up after a
handful of wrong guesses. Your server keeps the recovery key inside that
chip, not on disk. Someone who copies the disk gets an encrypted backup and
no key. Someone who steals the whole machine gets a few guesses and then a
growing lockout. You get your messages back with nothing but your password.
A Raspberry Pi has no chip built in; a small add-on board or a USB security
key does the same job.

**3. Username, password and a recovery code**, for servers with no chip, or
for people who do not want to rely on one. At signup the app shows a code to
print or save in a password manager. It is the recovery key, on paper.

The app tells you which options you have. On your own server with a chip it
says "you are covered". On a server without a chip it insists on the paper
code. On a public server you do not run it recommends keeping the code even
if the operator says they have a chip, because you would be trusting them.

If you lose every device, your server has no chip, and you never saved the
code, the backup is gone for good, and so is the key that proves you own
your username. On your own server you release the name and start fresh.

## A6. What a hostile public server can and cannot learn

Suppose you are `@you:bigpublic.example` and the operator is hostile and
runs both courier and clerk.

They learn: your username, your public key, that your account exists, your
internet address when you connect, that you are connected, how many people
asked for your card, how many requests you received, and how much traffic
the whole server carries. They hold an encrypted backup they cannot open, and, if they have a chip,
a recovery key inside it that lets them try a few passwords before it
locks.

They do not learn: anything you said, who you talk to, which slots are yours,
how many devices you have, which groups you are in, who is in them, or
anything about people on other servers.

Use a separate courier or Tor and the first list loses "your internet
address" and "that you are connected".

---

## A7. The timing problem, and what we do about it

Slots hide *what* and *who*. They do not by themselves hide *when*. On a
server with six users, an operator watching the clock can see "a letter
went into some slot at 9:14 and a phone got woken at 9:14" and, with roles
combined on one box, "that phone belongs to this internet address". With
enough patience that rebuilds who talks to whom. This is the one attack that
no clever numbering defeats, so it gets three defences, layered:

1. **The clerk keeps no clocks.** Nothing on disk carries a timestamp: not a
   slot, not an envelope, not a log line. Envelopes are numbered, not dated.
   A seized or copied server cannot be replayed as a timeline, because the
   timeline was never written down.
2. **The clerk never sees your phone come and go.** Deliveries are pushed
   from clerk to courier to phone, and the courier holds them while the
   phone is offline. A phone reconnecting after a night off talks only to
   the courier. The clerk sees no burst, no reconnect, no "fetch all my
   chats".
3. **A steady drip of fake letters.** In the paranoid setting, the courier
   sends the clerk a fixed number of bags every second, real or dummy, and
   the dummies are indistinguishable. The clerk sees a flat line. Turn it up
   and the flat line hides everything; it costs bandwidth, which on a home
   connection is cheap.

With roles split across two machines, defences 1 and 2 already leave the
clerk with nothing timely to record and the courier with nothing but your
address. With roles on one box, defence 3 is what stops the operator
watching the clock, and you can choose whether that operator is a concern.

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
- B9. Delivery: the clerk pushes, the courier holds
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
- B23. Deploying with Docker
- B24. OIDC and SSO
- Part C. The ten problems and their fixes

## B1. Threat model and what each party sees

Parties: the **home server** (front desk, clerk, backup store), the
**courier** (Envoy), **other servers** (hosting the people you talk to), and
the **network**.

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
        ▲                                            │  token issuer          │
        │ or arti (Tor) instead of an Envoy          └────────────────────────┘
        └──────────────────────────────────────────────────────▲
```

One binary, `sigil-server`, roles `home`, `envoy`, or `both`.

- **Envoy**: Oblivious HTTP style relay (RFC 9458). Clients seal each
  request to the home server's public key with HPKE (RFC 9180); the Envoy
  forwards ciphertext. It sees IPs and bag sizes. It holds push tokens and
  wake channels keyed by random handles it invents.
- **Home server**: the authoritative front desk for the names it hosts, a
  slot store, a blob store, a backup store, and a token issuer. It never
  accepts a direct client connection; everything arrives as a sealed bag.

**No federation.** `@alice:a.example` messaging `@bob:b.example` means
Alice's engine writes into a slot on whichever server hosts that
conversation, through Alice's Envoy. Servers never talk to each other about
users, rooms or messages. There is no server-to-server relationship at all.

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
  fixed-size envelopes, and a coarse expiry bucket. **No timestamps.**
- **The envelope is wrapped once more.** An MLS message carries `group_id`
  and `epoch` in the clear, which would let a server link every epoch of a
  group across rotations. So the MLS message is sealed under
  `env_key = MLS-Exporter("sigil/env/v1", group_id, 32)` with a fresh nonce
  before it becomes an envelope. The server, and the Envoy, see random
  bytes of a fixed size.

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

## B9. Delivery: the clerk pushes, the courier holds

The first draft woke the device and had it fetch. That produced two
correlations at the server: a read shortly after every write, and a burst of
subscriptions and fetches whenever a device reconnected, which clustered
every address that device cared about. Both are gone in this model.

1. **Subscribe once per address, for the life of the address.** The engine
   asks its Envoy to subscribe. The bag to the server contains `address` and
   a `wake_handle` the Envoy chose at random. The Envoy remembers
   `wake_handle → device`. The server records `address → [wake_handle…]`.
   The subscription is held by the Envoy on the device's behalf and is
   *never re-issued on reconnect*; it ends when the address rotates away or
   the device leaves the group.
2. **On a write, the server delivers the envelope itself** to every handle
   on that address: `deliver(wake_handle, envelope)`, constant size, no
   address. Nobody fetches. The server originates the traffic, so there is
   no read-after-write for it to observe.
3. **The Envoy queues per device.** Online: the envelope goes down the live
   socket at once. Offline: the Envoy holds it (it is doubly encrypted and
   carries no address) for up to 30 days and drains the queue when the
   device returns. A reconnect is a conversation between device and Envoy;
   the server is not involved and sees nothing.
4. **Backfill is the exception, not the rule.** A device only reads a slot
   by cursor (B5) when it is joining an address it did not subscribe to
   from the start (a newly linked device, a recovery) or when the Envoy's
   queue was lost. Backfills are spread over minutes with jitter and use the
   same padded bags as everything else.
5. **Push.** For a sleeping phone the Envoy sends a push through APNs or
   FCM, or over a self-hosted UnifiedPush channel on Android, carrying only
   "connect to me". The device connects and drains its queue. In the
   clocked tier (B13) pushes go out on a fixed schedule instead of on
   arrival, so Apple and Google see a heartbeat and not your rhythm.

The Envoy knows "device D has 213 subscriptions and a queue of 4". The
server knows "address A has three handles". Push tokens live only on the
Envoy. Handles are re-randomised when a subscription is re-created, which
happens on rotation, not on reconnect.

## B10. Abuse control without identity on the message path

Accounts exist (for names and recovery) but the message path never uses
them. Rate limiting is **Privacy Pass** (RFC 9576–9578):

- **Credential**: on registration, the client obtains a blind-issued
  credential by proving it owns an account. Blind, so the server cannot later
  recognise which account a credential belongs to.
- **Daily tokens**: presenting the credential (through the Envoy) yields a
  batch of blind-signed tokens, for example 2,000 a day. Every write, every
  subscription, every blob chunk spends one. Tokens
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

Requirement: a linked device, or username + password, and everything comes
back. Threat: whoever holds the backup guesses the password forever. Answer:
the password alone never opens anything; the second factor is either on a
device the user holds, inside hardware that enforces a guess limit, or on
paper.

**The backup.** The engine keeps an encrypted append-only backup on the home
server: identity keys, contacts, group states, message history, and media up
to a user-set cap (default 1 GiB, most recent first). It is encrypted under a
random `data_key`, stored under label `H(data_key, "label")` so the server
cannot tell which blob is whose, and uploaded as padded chunks through the
Envoy like any other blob. `data_key` is wrapped by `backup_key`.

**The key.**

```
recovery_key = 256 random bits, generated on the first device
pw_key       = Argon2id(password, salt)            // slow by design
backup_key   = HKDF(pw_key ‖ recovery_key, "backup")
```

The server stores the salt and the wrapped `data_key` by username. It never
holds `password`, and holds `recovery_key` only inside sealed hardware
(below), never on disk.

**Where the recovery key lives.** On every signed-in device, in the
platform keystore (Keychain, Android Keystore, Secret Service on Linux,
DPAPI on Windows). Optionally in the server's TPM. Optionally on paper.

### Path 1: a linked device

1. The new device generates an ephemeral X25519 + ML-KEM key pair and shows
   it as a QR code with a short random nonce.
2. An existing device scans it, runs a hybrid key agreement, and both sides
   derive `link_secret`.
3. Both display `emoji(HKDF(link_secret, "sas"))`, seven emoji from a fixed
   table of 64, the short-authentication-string idea Matrix and Signal use.
   The user confirms on the *existing* device, the one an attacker does not
   hold.
4. The existing device sends, encrypted under `link_secret`: the identity
   key, `recovery_key`, the group list with current epochs, and local
   history, through a one-shot slot at `H(link_secret, "slot")`.
5. The existing device issues an *Add* commit in every group for the new
   leaf, rotating every address.

A photographed, relayed or replaced QR code produces a different
`link_secret` on the attacker's side, so the emoji differ and the user
declines.

### Path 2: username + password against the server's TPM

When the server has a TPM 2.0 (or a USB security key, or a discrete TPM
board on a Pi), the client stores `recovery_key` in it at signup, sealed to
an authorisation value derived from the password:

```
auth = HKDF(Argon2id(password, salt), "tpm-auth")     // computed on the client
TPM2_Create(recovery_key, authValue = auth, DA-protected)
```

**The server is a pipe, not a participant.** The client does not send
`auth` to the server. It talks to the chip *through* the server, using the
TPM's own encrypted sessions:

1. The server exposes `tpm.relay`: raw TPM command bytes in, raw response
   bytes out. It is a dumb TCTI transport.
2. The client fetches the chip's endorsement key and its manufacturer
   certificate, checks the chain against embedded roots (Infineon, Nuvoton,
   STMicro, AMD, Intel; the same list Windows ships), and so knows a real
   chip is on the other end.
3. The client opens a **salted, encrypted HMAC session**
   (`TPM2_StartAuthSession` with the salt sealed to the chip's key, and the
   session's `encrypt` attribute set). Authorisation for `TPM2_Unseal` is an
   HMAC the client computes from `auth`; the unsealed `recovery_key` comes
   back parameter-encrypted under the session key. The server relays
   ciphertext both ways and holds neither the password-derived value nor
   the key at any point, in memory or otherwise.

The chip's **dictionary-attack lockout** (for example 8 failures, then a
10-minute recovery per failure) applies to every wrong HMAC, however the
server is configured. A disk image yields no key. A stolen machine yields a
lockout. Malware on the live server sees encrypted TPM traffic. That leaves
a hostile operator making guesses at the chip's rate through their own
relay, which is the residual, and the client says so and recommends the
paper code on servers the user does not run.

**One chip, one lockout.** TPM 2.0's lockout counter is per chip, not per
user, so wrong guesses on any account slow recovery for every account on
that server. Three defences: a per-username exponential backoff in the
server *before* anything reaches the chip; an optional OIDC gate (B24) so
only the account's SSO login can attempt at all; and, on public servers
with many users, the client recommending Path 3 as well. On a home server
with a handful of users the shared counter does not matter.

Servers without a chip say so in their card, and the client falls to
Path 3.

### Path 3: username + password + recovery code

At signup, when Path 2 is unavailable or the user opts in anyway, the app
shows `recovery_key` as a QR and a 55-character code to print or save to a
password manager, with a weekly reminder until done. Recovery is
`account.recover{username, password, recovery_key}`: derive `backup_key`,
fetch the salt and wrapped `data_key` by username, unwrap, fetch the backup
by label, replay it.

### Common tail

After any path the new device holds the identity key and issues *Remove*
commits for the lost device in every group.

**Password change**: re-derive `backup_key`, rewrap `data_key`, and, on
Path 2, reseal the TPM object with the new `auth`. A signed-in device can do
this without the old password because it holds `data_key`.

**Recovery key rotation**: any signed-in device generates a new
`recovery_key`, rewraps, reseals, pushes it to the other devices through the
self-group, and tells the user any printed code is now stale.

**Lost everything, no chip, no code.** The backup and the identity key are
unrecoverable, and the username is bound to the identity key. A self-hoster
releases it from the server's command line; a public server may release it
after a waiting period with operator approval.

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

**Shape.** Envelopes are padded to 1, 4 or 16 KiB; bags to the Envoy are
padded again to fixed sizes so the Envoy cannot tell a write from a
subscribe from a lookup.

**Timing.** Three tiers, chosen per user in settings, with the first two on
by default:

| Tier | What it does | What it costs |
|---|---|---|
| **no clocks** (always) | the server stores no timestamps anywhere: envelopes carry sequence numbers, expiry is a day bucket, logs carry counts only, and the log format is typed so a time cannot be written by accident | nothing |
| **push and hold** (always) | delivery is server-to-Envoy-to-device (B9); reconnects, offline catch-up and backfill never reach the server as a burst; the Envoy holds bags 0–2 s and forwards them shuffled | up to 2 s of latency |
| **clocked** (opt-in) | the Envoy sends the server a fixed number of bags per second, real or dummy; dummies are writes to throwaway addresses paid with blind tokens the Envoy holds, so the server cannot tell them apart; the device talks to the Envoy on a fixed cadence with a dummy bag when idle, and pushes go out on a fixed schedule | about 1 KiB per bag per second per Envoy (roughly 90 MB a day at one bag a second), some battery on mobile, and delivery waits for the next tick |

The clocked tier is the Loopix and Pond idea: a flat line hides everything,
and the rate is the only knob. It is what makes a *combined* courier and
clerk on one box unable to watch the clock. Tor via `arti` is a fourth,
independent switch that removes the IP from the Envoy's view as well.

**Slot creation timing.** Two objects are tied to a name: the requests slot
and the key-package shelf. When a request is accepted, nothing is written:
acceptance is local, and the first write to the conversation's own slot is
the recipient's first reply, minutes or days later. The client also never
creates a new address within a random 1–10 minutes of touching a
name-bound object, so "Bob got a request, and a slot appeared" is not a
pattern the server can read.

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
address away), delete the backup (the client knows the label),
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
| `backup.wrap` | `username, sig, salt, wrapped_data_key` | store the wrap by name |
| `tpm.info` | | endorsement key, certificate chain, capabilities |
| `tpm.relay` | `username, tpm_command_bytes` | relay one TPM command after per-user backoff and the optional OIDC gate |
| `room.*` | see Part C, problem 7 | open rooms: server-readable history, pagination, search, bans |
| `token.credential` | `account proof, blinded` | blind-issue credential |
| `token.issue` | `credential, blinded[]` | blind-sign the daily batch |

The Envoy speaks a plain control channel to the client for deliveries and
push registration, and receives `deliver(handle, envelope)` events from
servers on a long-lived stream, queuing them per device.

Every identifier the slot, blob and backup paths carry is a hash of a
secret or a random handle. Only `name.*`, `backup.wrap`, `tpm.*`
and `token.credential` name an account, and none touches a conversation.

## B18. The server: one binary

- **One static Rust binary**, `sigil-server`, roles `home`, `envoy`, `both`.
  No Postgres, no Redis, no auth service, no sync proxy, no reverse proxy
  required.
- **Embedded storage**: `redb` for names, slots, shelves, token
  double-spend sets; a content-addressed directory for blobs and backup
  chunks. Backup is copying two paths.
- **Transport**: `axum` + `rustls` with built-in ACME; `quinn` for the wake
  stream.
- **TPM**: `tss-esapi` against `/dev/tpmrm0`; optional, detected at start,
  advertised in the server's card so clients know which recovery paths
  exist.
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
      envoy.rs          bags (hpke), subscriptions, deliveries, queue drain
      tpm_client.rs     encrypted TPM sessions over tpm.relay
      requests.rs       the requests slot, accept/ignore/block
      tokens.rs         Privacy Pass credential and daily batches
      blobs.rs          chunking, encryption, upload/fetch
      backup.rs         continuous encrypted backup
      linking.rs        QR + emoji device linking
      recovery.rs       restore via TPM or recovery code
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
`ed25519-dalek`, `argon2`, `tss-esapi`,
`privacypass`, `arti-client`, `redb`, `rusqlite`, `axum`, `rustls`,
`rustls-acme`, `quinn`, later `str0m`.

## B20. Phases

**0. Specification.** Done for the derivation layer: see
[`docs/spec/sigil-protocol-v1.md`](spec/sigil-protocol-v1.md) and the
`sigil-protocol` crate in `protocol/`, whose tests verify the vectors.
Still to write: the bag operations and their layouts, blind tokens, the
Envoy control channel.

**1. Home server.** Names, slots, shelves, blobs, tokens; `redb`; ACME. A
command-line client exchanges padded envelopes through addresses from a
shared secret.

**2. Client core.** `transport::sigil`: identity, `openmls` on a hybrid
suite, exporter-derived slots, requests slot, local store. Two engines hold a
direct message by username. Omarchy frontend unchanged.

**3. Devices and wake.** Linking; the Envoy role with subscriptions, handles
and push; Android receives an empty push and fetches.

**4. Recovery.** Continuous backup; QR + emoji device linking; TPM-sealed
recovery key with attestation; the printed code as fallback; password
change. New phone: scan, or username + password, everything back.

**5. Groups and media.** Policy documents; add, remove, rename; blob
chunking; retention; group migration between hosts.

**6. Shape.** Padding audit, Envoy jitter, cover traffic, `arti`, the
paranoid settings page.

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
- **Recovery is a linked device, or username + password.** The second
  factor is never on the server's disk: it is on a device, sealed in the
  server's TPM behind a hardware guess limit, or on paper.
- **Convenience wins ties**; privacy hardening is opt-in.
- **Hybrid post-quantum from day one.**
- **One binary, three roles.**

Open, with the recommendation:

1. **Large groups.** Above roughly 200 devices one address gets busy.
   Recommendation: raise the self-update interval for large groups and
   revisit only if real usage demands.
3. **Deniability.** MLS messages are signed. Recommendation: accept for v1.
4. **Open registration policy on public servers.** Recommendation: invite
   codes by default, proof-of-work as the open alternative.

## B23. Deploying with Docker

The server is one static binary, so the image is a few megabytes on
`scratch` or distroless. A complete home deployment:

```yaml
services:
  sigil:
    image: ghcr.io/pellinore377/sigil-server:latest
    command: run --role both --config /data/sigil.toml
    ports:
      - "443:443/tcp"      # HTTPS bags, control channel
      - "443:443/udp"      # QUIC, wake stream
      - "80:80/tcp"        # ACME HTTP-01; drop if using the DNS challenge
    volumes:
      - sigil-data:/data   # names, slots, blobs, backups, certs: the whole backup
    devices:
      - /dev/tpmrm0:/dev/tpmrm0   # recovery Path 2; omit on a chip-less box
    restart: unless-stopped
volumes:
  sigil-data:
```

Notes:

- **Split roles** are two services from the same image, `--role home` and
  `--role envoy`, on the same host or different hosts. A friend's Envoy or
  a public one needs nothing from your compose file at all.
- **Reverse proxies** (Caddy, Traefik, nginx) are fine in front of either
  role. Bags are sealed end to end, so a proxy terminating TLS sees nothing
  extra. A proxy in front of the *home* role sees client IPs and must not
  log them, unless an Envoy sits in front of it, in which case it only sees
  the Envoy's address. Forward `X-Forwarded-For` nowhere.
- **The TPM device** passes through with `devices:`; the container needs no
  extra privileges. `/dev/tpmrm0` is the kernel's resource manager, which
  multiplexes safely alongside anything else on the host using the chip.
- **Backup** is `docker run --rm -v sigil-data:/data alpine tar c /data`.
  Restore is the reverse. The server runs while you copy.
- **UnifiedPush** on Android needs no extra service: the Envoy role speaks
  the UnifiedPush server side itself.
- **PocketID** or any other OIDC provider (B24) is one more service in the
  same file; the server only needs its issuer URL.

## B24. OIDC and SSO (PocketID and friends)

The engine already signs into Matrix with OIDC and a localhost redirect, so
the browser round-trip exists. What matters is *what* OIDC is allowed to
control, because the design depends on the message path having no login.

**OIDC gates the things that are already tied to a name:**

| Operation | With OIDC configured |
|---|---|
| `name.register` | requires a valid ID token from the server's issuer; the server maps `sub` to "may hold a name here"; replaces invite codes |
| `token.credential` | the daily blind-token credential is issued to a logged-in `sub`; tokens stay unlinkable afterwards |
| `tpm.relay`, `backup.wrap` (password change) | require a fresh ID token for the same `sub` that registered the name; this is the strongest guess limiter on Path 2 |
| admin API | operator's own `sub` or group claim |

Mechanics: authorisation code flow with PKCE from the engine, redirect to
`http://localhost:<port>/callback`, ID token presented inside a sealed bag;
the server validates signature, issuer, audience and expiry against the
issuer's JWKS and stores only `sub` → localpart. PocketID's passkey login
works unchanged; so does Authentik, Keycloak, Authelia, or Google.

**OIDC never touches:**

- **the identity key.** It is generated on the device. No login can produce
  or recover it, so the identity provider is never the thing a subpoena
  goes after.
- **slots, blobs, subscriptions, deliveries.** Those are paid with blind
  tokens and carry no account.
- **the backup password.** An identity provider issues assertions, not
  secrets. The password must be something no server holds.

**What the identity provider learns:** when a name was registered, when
tokens were drawn (daily), and when a recovery or password change was
attempted. On a self-hosted PocketID that is the operator watching
themselves. On a third-party provider it is a new party learning a coarse
rhythm, so SSO is per server and optional, and the client says which
provider a server uses before the user registers.

## B22. References

- MLS: RFC 9420 (protocol), RFC 9750 (architecture).
- HPKE: RFC 9180. Oblivious HTTP: RFC 9458.
- Privacy Pass: RFC 9576, 9577, 9578.
- Argon2id: RFC 9106. TPM 2.0 Library Specification, Part 1, "Dictionary
  Attack Protection". Short authentication strings: Matrix MSC4108 and
  Signal's device-link flow.
- ML-KEM: FIPS 203. X-Wing: Barbosa et al., 2024.
- Signal: PQXDH (2023), SPQR (2025), Sealed Sender (2018), *The Signal Private
  Group System and Anonymous Credentials* (2020).
- Piotrowska et al., *The Loopix Anonymity System* (2017); Langley, *Pond*
  (2012–2016), for fixed-cadence clients.
- TPM 2.0 Library Specification, Part 1, sections 19 (sessions) and 21
  (session-based parameter encryption); TCG *EK Credential Profile*.
- OpenID Connect Core 1.0; RFC 7636 (PKCE).
- UnifiedPush specification.
- Apple, *iCloud Private Relay Overview*.

# Part C: the ten problems and their fixes

Each entry: the problem as found in review, the fix now in the design, and
what remains.

**1. Reconnect bursts clustered a device's addresses.** Fixed by B9: the
server pushes deliveries, the Envoy queues them per device, subscriptions
are held by the Envoy for the life of an address and never re-issued on
reconnect. The server never sees a device come or go. Remaining: backfill
after a lost Envoy queue or on a newly linked device, spread over minutes.

**2. Small servers have small crowds.** Fixed in layers by A7 and B13: the
server stores no clocks, so a seizure yields no timeline; push-and-hold
removes every client-originated read; the clocked tier makes the
Envoy-to-server stream a flat line of indistinguishable bags, which is the
only known defence against an operator watching in real time. Remaining: a
combined-role operator who declines the clocked tier can still watch
timing, and a six-user server has six users. The design makes the trade
explicit and puts the knob in the user's hand.

**3. Name-bound objects leaked slot creation.** Fixed in B13: acceptance
writes nothing; the first write to a new slot is the first reply, at human
delay; and the client refuses to create any address within 1–10 random
minutes of touching a name-bound object. Remaining: key-package drain
counts and request counts per name, which say how many conversations
started, not with whom.

**4. One TPM lockout for everyone.** Fixed in B12 Path 2: per-username
backoff before the chip, an optional OIDC gate so only the account's SSO
can attempt, and the client recommending the paper code on large public
servers. Remaining: on a public server that runs neither, a determined
attacker can slow everyone's recovery; the operator's choice.

**5. A compromised live server could watch a recovery.** Fixed in B12
Path 2: the client runs an encrypted TPM session through the server as a
dumb relay, with the endorsement certificate checked against manufacturer
roots. The password-derived value and the key never exist on the server in
the clear. Remaining: the relay can be denied, not read; and a fake chip
would need a forged manufacturer certificate.

**6. Push leaks timing to Apple and Google.** Reduced in B9 and B13: on
Android, a self-hosted UnifiedPush channel through the Envoy bypasses
Google entirely; on iOS, APNs is mandatory, so the clocked tier sends
pushes on a fixed schedule. Remaining: in the instant tier on iOS, Apple
learns "a push at 9:14", as it does for Signal.

**7. Public communities did not fit.** Added as an explicit second mode,
**open rooms**: a room the host server can read. The room has a public
card, a server-held history with pagination and search, and server-enforced
bans. Members post under a per-room pseudonymous key by default and may
attach their username. Transport is still sealed bags, so outsiders and
the Envoy see nothing; the host server is a member. This is the Matrix
public-room experience without federation, and it is the only place the
server reads anything. Private groups remain blind. Remaining: an open room
is exactly as private as its host, and the client labels it so.

**8. Nobody could moderate what they cannot see.** Fixed at three levels.
Local: block lists synced across devices through the self-group; blocked
identities' Welcomes are dropped silently. Provable reports: every MLS
message is signed by a leaf key bound to the sender's identity, so a
recipient can forward a message plus its signature to the sender's home
server as proof, and that server can revoke the sender's credential (no
more tokens) or name. Open rooms: admins ban, the server enforces.
Remaining: private-group abuse is only ever reported by a participant, by
design.

**9. Cross-server spam.** Fixed in B10 and B7: a server issues request
tokens per *issuing server*, with a small daily quota for servers it has
not seen and a growing one for servers with a clean history; unknown
servers may be asked for proof-of-work per token; a server may run an
allow-list. Because a request costs a token from the *recipient's* server,
a hostile server minting credentials for its own users cannot buy more
than its quota. Remaining: quota tuning, which is operations, not design.

**10. Engineering risk.** Addressed by decisions rather than code: commit
conflicts are resolved by the slot's own sequence number (the first commit
at an epoch by sequence wins, and every member sees the same sequence);
group size is capped at 1,000 devices for v1; the hybrid post-quantum
suite is behind a feature flag so the client ships on X25519 if the MLS
suites are not final; `openmls` is in production at Wire and is the
reference implementation for the RFC. Remaining: time.

## Appendix C. Later, if ever: recovery without a device or a code

A user with one phone and no saved code has no path today. If that ever
matters, the recovery key can be split (Shamir, 2-of-3) across the home
server, a second server or a friend's app, and the user's password, with
each holder enforcing a guess limit through a threshold OPRF (RFC 9497;
Signal's SVR3). It is an addition on top of B12, not a change to it, and is
deliberately not in the plan because it needs more than one server to be
worth anything.
