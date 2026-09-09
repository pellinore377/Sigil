# Current work

Android implementation remains active against the twelve mockups. Inbox/search/notes, collections, timeline receipts/menus/swipes, account appearance, private chat themes, attachments, interactive cards and native audio/video/screen calls are connected. Signed contact requests preserve drafts until explicit device verification. Profile names/photos use separate account/origin sharing permissions; they are server-visible metadata, not encryption identity. Photo upload/retry/removal, encrypted caching and block/restore revocation are implemented. Android 11 keyboard positioning now preserves the header and composer.

Device inventory/revocation, QR linking with consent on both devices, recovery setup/retention, storage counts, call history/redial, private notifications, conversation clearing/group departure and encrypted device-local wallpapers are connected. Sign-out confirms local loss, revokes this device, then asks Android to clear app data/keys and close the app. Unconfirmed revocation keeps data with retry or explicit local-only removal. Pending sign-out survives restart and pauses background work. Production accounts remain preserved; physical tests use synthetic content in the isolated acceptance app. The standard APK and native libraries are rebuilt and installed with the existing account preserved.

Android profile settings support SSO linking to the existing account, resumable browser callbacks, and configuration-bound acknowledgment before provider retirement. Foreground checks surface pending retirement notices without automatically acknowledging them.

Android’s file worker also runs archive backup/import, retained-history cleanup, journal erasure and media recovery. Recovery-key import authenticates the manifest before atomically storing the key and import state. A protected restore dialog requires explicit acceptance of an unanchored backup. Saved history is browsable without live contact/group state; unresolved conversations have no send/call controls. Restoring history never restores live identity/session keys or peer approval.

Lost-device sign-in offers SSO or an administrator recovery invitation with explicit replacement consent. Successful recovery preserves the account, revokes previous devices and offers history import. Returning to sign-in choices refuses to discard live credentials, local messaging keys or an uncertain enrollment.

Group audio uses a featured speaker, live waveform and smaller participant row; video retains the grid with activity indicators and direct camera/share controls. Android bars follow the app theme. Icons and avatar initials keep their proportions at enlarged text sizes; voice controls wrap and scroll.

Android UnifiedPush registration, proof handling, retries and disabling are connected to Rust's encrypted push state and background worker. Notifications settings lists installed distributors and offers periodic sync as a fallback. Physical tests exercise real connector IPC with a synthetic distributor, reject altered ciphertext and confirm the server channel before disabling it. External-provider delivery and an Android FCM adapter remain open.

Canonical rich text reaches Android with UTF-16 presentation ranges, preserving inline styles, code, links and card labels without reparsing the body. Spoilers stay out of rendered text and link semantics until revealed; scratch gestures have an accessible reveal action. Named colors adapt to bubble contrast. Authored SigilText motion currently renders statically.

Remaining UI work: forwarding, accessibility and performance. Optical linking between two physical devices and broader audio routing/battery acceptance remain open. Desktop messaging has no connected storage adapter. Admin appearance remains browser-local; native account appearance syncs privately. Wallpapers are currently device-local.

The UI contract is in [Design.md](Design.md). Exact message details start hidden; only the final outgoing timeline message shows its receipt/avatar stack. Reaction/pin corners mirror by direction. Small typing stacks lift the latest timeline. Swipe inward replies; outward opens a thread. Long-press/right-click centers the bubble between reactions and actions without moving receipts. Keyboard/attachment/voice switching preserves composer height. Direct-call termination ends both sides; group departure preserves remaining participants. Default neutral Newsreader, optional Google Sans Flex, quick reversible transitions and reduced motion remain required. [plan.md](plan.md) is unchanged.

Backend/shared Rust items #1–#12 are implemented. The container includes the Compose/Wasm setup wizard and Admin dashboard, Argon2id passwords, OIDC linking, account/group administration and separate user-password policy. Advanced maintenance/service controls remain APIs. Public GHCR `latest` and revision tags support deployment. Opt-in bounded `SIGIL_LOG_CIPHERTEXT` logging excludes plaintext, credentials and private keys.

## Validation

Server schema **32**, native **75**, attachment cache **5**. Release RTC workspace before rich-text projection: **836 passed, zero failed, thirteen ignored**; updated client/text subset **390 passed, seven ignored**. Ignored entries are parent-invoked crash helpers, a fixture generator and separately invoked acceptance/load tests. Commands are in the README. Shared UI **24 tests** pass; Clippy and Android build checks pass. Physical rich-text **2 tests** cover canonical code, link taps, emoji spoilers and scratch gestures without accidental reply actions. Physical layout **3 tests** cover both fonts/modes, 200% text, status-bar contrast, composer actions and four call layouts. These use synthetic view state; actual media transport is checked separately.

Physical content **5 tests** pass, including encrypted attachment playback/seeking, authenticated maps, wallpapers, photos and archive publication through Android’s worker. After phone sign-out, a separate replacement client restores the saved test message from that backup. Messaging/menu/recovery/sign-out/account-access **13 tests** pass, including the keyboard/header regression and saved-history rendering. Sign-out acceptance confirms server revocation, app-data/wrapping-key removal, job cancellation and fresh enrollment state. Native tests cover wrong recovery keys, atomic import rollback/restart, retained history without live keys, unauthorized sign-out retries, account-preserving SSO callbacks and stale retirement acknowledgments. Replacement/enrollment **18 tests** pass after the workspace run, including explicit consent, old-device revocation and refusing unsafe cancellation. QR rendering/decoding and consent gating **2 tests** pass. Four physical call modes pass: audio, camera, screen and three-person continuation after creator handoff. Call-only authorization cannot grant ordinary messaging or profile access.

Two-server acceptance passes through signed contact requests, authorized profile photos, messaging, group/history/file transfer, outages/restart and encrypted calls. Container schema **31→32** upgrade/downgrade rejection, restart/retries, offline/guided backup/import/restore, revocation, recovery ciphertext, attachment lifecycle and push reset pass. Pending encrypted-upload compatibility acceptance remains open.

Browser checks cover onboarding, private-provider OIDC discovery, callback binding, account-preserving linking, username selection, native password fields, Enter navigation, clipboard/context menus and appearance. The operator confirms native PocketID sign-in; synthetic Auth Tab tests confirm return/dismissal. Other-browser callback fallback remains unverified.

## Security and acceptance limits

[Audit.md](Audit.md) records eight-area Codex review and four independent Claude batches. Confirmed restore-journal, quota, maintenance-artifact and federation-counter defects were fixed. That independent audit predates browser/mobile additions, which have self-review and regression tests. No security certification is claimed.

Independent HMAC/HKDF expectations cover 36 initial Triple Ratchet packets; OpenSSL/libsodium PQXDH fixtures pass. Four ASan fuzz targets completed approximately 17.9 million executions without a product crash. These do not prove protocol composition, later-epoch/post-compromise security, RaptorQ healing bounds or target-machine constant-time behavior. Gaps beyond the skipped-key budget can stall a session; automatic reset is refused. Mailbox cursors expose aggregate activity; group credential issuance trusts the homeserver.

Three dependency advisories and one build-tool maintenance warning remain documented in [Security.md](Security.md); `cargo audit` is not clean. Erasure/replay tests cover superseded checkpoints, messages, history fragments, caches, migration, cleanup and restart; physical storage limitations remain explicit. Live push, alarms, audio latency/routing, accessibility, battery and UI performance still need client/hardware acceptance. User passwords are administrator-provisioned; self-service password changes remain UI work.

## Backend measurements

Synthetic HTTPS with 50 ms request RTT; established sessions:

| Workload | Result |
| --- | --- |
| 4 CPUs/8 GiB, 50 accounts, 20 devices, messages alongside 160 MiB uploads | Durable feedback 2.3 ms p95; recipient decryption 119 ms p95 |
| 100,000 encrypted messages | 64-candidate search page 16.2 ms p95; full scan 17.1 s |
| Two servers, additional 50 ms inter-server RTT | Recipient decryption 164 ms p95 |
