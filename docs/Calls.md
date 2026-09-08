# Calling contract

Calls are independent of conversations and support eight participants, each with audio, camera and screen tracks. One creator owns membership and closes the call when leaving. Joining requires an encrypted invitation and explicit acceptance. Invitations ring for at most 60 seconds; call lifetime is at most 24 hours. Leaving/declining is terminal for that call. An active participant can reconnect after transport or process failure.

## Authentication and keys

The creator signs a monotonically revisioned roster of ephemeral participant signing keys. Each key has a device-signed attestation scoped to the call. Encrypted native controls travel through existing verified pairwise Triple Ratchet sessions. Participants independently verify device attestations, creator state signatures and participant readiness/key-share signatures. Known device replacements or blocked peers cannot inherit call authority. Participants without independently verified contact bindings remain visibly unverified.

Readiness binds a durable sequence, receiver challenge and enabled tracks. A fresh native media handle requires a new challenge; the creator includes it in signed state before fresh keys can be used. Membership/readiness changes invalidate prior media state. The creator relays individually signed sender key shares through encrypted controls. Persistent records/jobs use the native storage key; ordinary message history retains authenticated control markers, not media seeds. End/expiry removes active secrets from live storage; [physical erasure limits](Security.md#erasure) still apply.

Encoded frames use [RFC 9605 SFrame](https://www.rfc-editor.org/rfc/rfc9605.html), AES-256-GCM/HKDF-SHA-512, with a full authentication tag. AAD binds call, roster, sender, fresh incarnation, media kind, timestamp and keyframe flag. Each frame additionally carries a sender signature: shared SFrame AEAD keys alone do not authenticate individual senders against other recipients (§§6.1.1, 7.2). A receiver verifies the signature and AEAD before advancing its 128-frame replay window. Sender handles cannot be restored from old keys; renewal occurs after 600 seconds, 1 GiB or the frame-counter limit.

Frames are at most 1 MiB. RTP packetization splits ciphertext into 1,024-byte fragments with a version, ciphertext digest, total length, index and count. Camera/screen payloads have a VP8 payload descriptor. Assembly tolerates duplicates/reordering, expires incomplete frames after two seconds, and caps pending work at 16 frames/8 MiB. Only complete authenticated plaintext may reach a decoder.

## Signaling and forwarding

`PUT /client/v0/calls` publishes the creator-signed roster using local device credentials; `GET` reports availability. `POST /calls/v0/connect` accepts a participant-signed current-roster SDP/layout and durable connection sequence. `POST /calls/v0/relay` accepts a current participant proof timestamped within 60 seconds. These public proof routes reject account credentials, queries and browser origins. Remote clients use authenticated federation lookup services `call_connect`/`call_relay`, without account/device identifiers in the remote envelope. Federation nonce admission precedes media admission; retries use fresh federation nonces and the durable connection sequence.

str0m 0.23.1 forwards encrypted Opus/VP8 RTP over ICE-lite/DTLS-SRTP. Signed SDP binds transport fingerprints, upload MIDs and every requested downstream track. Responses map downstream SSRCs to participant/media kind. The forwarder preserves output sequence progression when a sender reconnects, bounds per-track queues to 64 KiB/128 packets, and limits participant ingress to 1 MiB/second. RTX caching and keyframe feedback are bounded. Membership changes disconnect all old transports; clients negotiate against the new roster. Server restart rejects an old connection sequence without live transport state: create a fresh peer connection and increment the durable sequence.

The server sees creator account/device, call times, ephemeral roster, SDP, addresses, codecs, track mappings and traffic sizes/timing. TURN sees relay credentials, addresses and traffic. Other participants receive device attestations. Media content/keys remain client-held; this does not hide call metadata or prevent an authorized recipient recording plaintext.

## Deployment and adapters

Calling is disabled by default. `GET/PUT /admin/v0/calls` uses `expected_revision`, nullable `settings`, and `turn_secret` actions `keep`, `clear`, or `set` with `value`. Settings contain `bind`, `advertised`, `max_calls` (1–8), and up to four `turn_urls`. Changing configuration closes existing calls. `/admin/v0/calls/status` reports readiness, live counts and dropped packets.

For example, bind `0.0.0.0:34780`, advertise the server's reachable address/port, and start with `max_calls: 1`. `docker compose -f compose.yaml -f compose.calls.yaml up --build -d` publishes that UDP port. Forward it through the firewall/NAT; HTTPS signaling still needs the reverse proxy. A UDP forwarding socket must remain reachable from TURN.

Optional Coturn uses a separate shared REST secret of 32–256 ASCII characters. Configure explicit URLs such as `turn:turn.example:3478?transport=udp` and `turns:turn.example:5349?transport=tcp`. Use a valid TLS certificate and restrictive relay network policies. Credentials expire within ten minutes or call expiry; request them before SDP gathering and refresh through a fresh connection before expiry. Disabling a call stops SFU access; an already allocated TURN relay can last until its own expiry. Native validation permits the same 60-second signaling clock skew.

Platform adapters supply established codec/capture processing, pacing, echo cancellation, audio routing, authenticated-frame decoding, network-change callbacks and background incoming-call presentation. Use `set_call_tracks` for audio/video/screen transitions; never send captured media before explicit acceptance. UI, hardware codecs and home-network latency remain device acceptance work. Local synthetic RTP timing is not a production p95 claim.
