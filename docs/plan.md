# Sigil: unified implementation plan

## 1. Outcome and project rules

Build a self-hostable E2EE messenger with a polished client, rich SigilText content, reliable recovery, integrated calling, and a first-class Admin management experience. Complete the agreed feature set on Android first, then implement Web → iOS → Linux → Windows → macOS.

The everyday experience is one account on one homeserver, with automatic communication across independent servers. Operators manage infrastructure redundancy; users do not manage multiple hosting identities.

Self-hosting must be UI-driven by default. A new operator should be able to install Sigil, open Admin, complete guided setup, configure the server, manage users/invitations/integrations, review health, and perform upgrades/backups/restores without hand-editing configuration files for normal operation.

### Development constraints

- Use Rust for the server, shared client logic, cryptographic protocols, SigilText, and suitable rendering/processing components.
- Prefer Slint for shared adaptive mobile, desktop, and web interfaces. Use narrow platform adapters where required.
- Prefer established C/C++ components when suitable Rust implementations do not meet requirements. No Python application services.
- Use idiomatic Rust, default rustfmt, Clippy, explicit error handling, and bounded resource usage. Justify and document necessary unsafe.
- Comments must be extremely terse and necessary. Keep one concise README, required notices/specifications, and one short current summary in docs/Status.md. Update the summary in place; no progress logs, historical ledgers, or duplicate status reports. Commit messages must be terse.
- Start in one private monorepo with independently buildable server, shared-core, client, and platform packages.
- Preserve MPL-2.0, GPLv3, AGPLv3, and proprietary client distribution options until a license is selected. Preserve an MIT-compatible server path. Reserve official branding separately.
- Review exact dependency versions, enabled features, native libraries, assets, and transitive licenses. Do not automatically incorporate libsignal or other strong-copyleft dependencies.

### Personal-data prohibition

Real conversations, usernames, domains, credentials, device identifiers, screenshots, and device-derived logs must never enter the project, fixtures, commits, documentation, or build artifacts. Use synthetic fixtures and reserved example domains. Keep necessary test-device configuration outside the project; collect only deliberately selected, non-identifying measurements.

## 2. Architecture and security boundaries

### Accounts, discovery, and devices

- One homeserver account, addressed as @user:server; usernames are unique within that server.
- Separate account addresses, display names, stable internal identity references, and client-controlled encryption keys.
- Support invitation-based enrollment and optional standard OIDC authentication, including Pocket ID. Neither OIDC nor server administrators can decrypt history or silently inherit an existing verified encryption identity.
- Allow exact-address discovery by default, with a user opt-out. No public browsable directory or partial-name search.
- Resolve ordinary mentions from known contacts and current conversation participants. Duplicate display names require disambiguation; copied names/images never transfer verification.
- Link devices using an authenticated QR exchange and transcript-bound emoji confirmation. Each device has independent credentials and session state.
- Provide device inspection, revocation, contact verification, key-change warnings, blocking, invitation revocation, and contact-request controls.
- Defer account migration. Preserve identity/address separation and versioned authenticated updates so migration can be added without rewriting conversation references.

### Messaging, groups, and federation

- Implement our own Rust protocol code following pinned Signal Triple Ratchet and associated session-establishment specifications, using established cryptographic primitives. Do not invent replacement cryptography.
- Use Signal’s private-group and Sender Keys designs as references for group messaging, administration, and device distribution.
- Treat their adaptation to federation as a security-design milestone. Do not claim pairwise Triple Ratchet guarantees automatically apply to groups.
- Establish authenticated group membership transitions, device changes, key rotation, and concurrent administrative-update handling before implementing shared group state.
- Federate authenticated, bounded encrypted deliveries. Users never need accounts on their contacts’ servers.
- Servers persist encrypted delivery/synchronization data and encrypted attachments; devices maintain encrypted local history and search indexes.
- Keep conversation semantics—names, replies, threads, group settings, receipts, and structured actions—inside encrypted content wherever the protocol permits.
- Version client-server, federation, encrypted-event, and backup interfaces independently. Define compatibility and downgrade rejection before freezing version 1.
- Implement retry-safe delivery, duplicate suppression, replay protection, offline queues, ordered local presentation, and reconciliation after interruption.

### Recovery and retained history

- Offer automatic encrypted history recovery during onboarding, protected by a separately generated recovery key.
- A fresh device can restore using account access plus recovery material, without the original phone.
- Restore conversation structure and text first; fetch media progressively. Show recovery progress, last successful checkpoint, and unprotected pending uploads.
- Never blindly restore and reuse old live ratchet state. Establish fresh device/session credentials.
- Retain local history until deletion or expiration. Retain remote encrypted recovery history and media according to disclosed account retention settings.
- Default account storage quota: 10 GiB, configurable by the operator. Warn before exhaustion and reject new storage cleanly; do not silently delete old history.
- Exclude view-once and disappearing content from recovery archives initially. Track deletions to prevent routine restoration from resurrecting removed content.
- Explain that recovery-key compromise exposes retained backup history, and recipient copies cannot be forcibly erased.

### Metadata objective

Long-term objective: eliminate or substantially reduce observable communication relationships, not merely encrypt message text.

Version 1 must document remaining account, discovery, routing, traffic, push, attachment-access, and call metadata. No “100% blind” claim.

Maintain a research milestone for PIR, traffic shaping, padding, and other metadata-protection techniques. Evaluate them against malicious servers with observation enabled. Adopt stronger mechanisms only when their complete guarantees, costs, and mobile behavior are demonstrated.

## 3. Client behavior and feature scope

### Conversations and organization

- Distinct direct conversations, named groups, and Note to Self.
- Groups support members/admins, invitations, removal, permission controls, and administrator changes.
- Replies, threads, reactions, edits, deletion, disappearing messages, voice messages, and delivery/read/failure states.
- Timeline timestamps, encryption/trust details, configurable typing indicators and contact-limited status/presence.
- Conversation/global search performed locally; conversation settings, shared message pins, and private conversation pins.
- Collections are off by default, enabled in settings. Their names and membership synchronize encrypted across linked devices.
- Long-press actions: pin/unpin, mark read/unread, snooze, and leave group; direct chats provide delete and block actions.
- Drafts synchronize encrypted. Preserve concurrent conflicting drafts rather than silently overwriting them.
- Snooze presets: one hour, eight hours, and until the next local morning. Incoming-call controls are separate.
- Read receipts and typing indicators start enabled and are independently configurable. No public last-seen directory.

### Configurable group history

Default new members to history from joining. Admins can enable sharing retained earlier history.

Implement earlier-history access as an authenticated, encrypted transfer from an authorized existing device, with pending/unavailable states when no supplying device is available. Do not distribute old live ratchet state. Exclude expired, view-once, and deleted content; disclosure cannot subsequently be revoked.

### SigilText

Implement the complete current SigilText specification through Android milestones:

- Markdown, modifiers, colors, gradients, reveal/redaction effects, and parameterized animations.
- Checklists, recurring lists, tasks, polls, reminders, notes, timers, and countdowns.
- Charts, diagrams, tables, recipes, mathematics, calculations, conversions, randomizers, display helpers, ASCII art, QR codes, and service-backed cards.
- Live formatted composition, context formatting, completion, graphical builders, round-trip editing, and integrated help.
- One canonical semantic model shared by typed syntax and graphical creation.
- NFC normalization and grapheme-safe effect ranges; redaction occurs before any transmitted representation, search text, or fallback is produced.
- Compact inline cards, expanded viewers, accessibility, theme safety, reduced motion, and deterministic animation replay.
- Notes remain a per-conversation view of notes, checklists, and reminders, separate from pinned messages and Note to Self.

Independent checklist-item edits merge; same-item conflicts resolve deterministically with attribution. Recurrence settings require authorized edits. Poll ballots are per identity and editable until closure.

Closed-result polls and task undo deadlines are initially application behavior, not guarantees against malicious modified clients. Do not claim hidden results are cryptographically inaccessible or client timestamps prove a deadline.

### Attachments, maps, and services

- Attachment panel includes camera, gallery, files, stickers, GIFs, emoji, polls, formatting, location, and categorized SigilText tools.
- Support arbitrary file transfer up to a configurable 1 GiB default, with encrypted resumable chunks, progress, cancellation, and retry.
- Every attachment gets a useful bubble; supported formats receive thumbnails and expanded in-app viewing.
- Cover the requested text, data, PDF, spreadsheet, document, presentation, STL/3MF, vCard, image, audio, video, and GIF families.
- Implement a format capability registry. Distinguish full rendering, simplified previews, and download-only fallback; never silently describe incomplete support as full fidelity.
- Prefer PDFium through Rust bindings for initial PDF evaluation; compare hayro where useful. Evaluate calamine for spreadsheet data and Symphonia for audio.
- DOCX/PPTX and uncommon codecs require explicit implementation feasibility checks. A parser alone does not satisfy the viewer requirement.
- Authenticate attachment data before processing. Use isolated preview workers without key access or arbitrary networking, bounded memory/time/decompression, and no macros or document scripts.
- Add local malware scanning only where operationally practical. Never upload files or hashes automatically for scanning; never label a file guaranteed safe.
- Use PMTiles with an evaluation of embedded martin-core. Serve styles, fonts, and sprites locally. No Postgres requirement for file-backed maps.
- Support one-time location, dropped pins, and live sharing for 15 minutes, one hour, or eight hours, with explicit stop controls.
- External-service features require explicit provider configuration and clear query disclosure. Received cards render from encrypted snapshots without repeating recipient-side queries.
- Support self-hosted endpoints. Do not require a GIPHY integration incompatible with the desired delivery/privacy model.

### Calls and notifications

- Distinct audio/video entry buttons backed by one call session capable of adding video.
- Add people to a call without adding them to a conversation.
- Include group calls, screen sharing, device/audio routing, reconnects, and background incoming-call handling.
- Compare str0m and webrtc-rs for the Rust forwarding service; use established client media processing and hardware codecs where needed.
- Encrypt media end to end through the forwarding service. Transport encryption alone is insufficient.
- Send FCM only generic wake-up data without message, sender, or conversation content. Construct notifications locally after fetching and decrypting events.
- Offer UnifiedPush support; account for distributor setup and platform delivery limitations.

### Admin management

The Admin UI is the integrated operator-facing management surface for Sigil. It is part of the product, not an afterthought.

- First-run setup wizard: server name/domain, administrator account, storage paths/quotas, HTTPS/reverse-proxy checks, push options, OIDC, external/self-hosted providers, and optional maps/media settings.
- Prefer safe defaults and automatic validation. Normal setup must not require editing TOML/YAML/JSON by hand.
- Dashboard: service health, version, federation reachability, storage usage, backup status, queue/failure summaries, push/provider status, and upgrade availability.
- User/admin controls: invitations, account status, role/permission management, device/session visibility, blocking/abuse controls, and configurable registration policy.
- Federation controls: allow/deny policy where supported, peer diagnostics, retry/error visibility, and clear distinction between local faults and remote-server faults.
- Storage/recovery controls: quotas, retention policy, backup destination/status, restore workflow, media usage, and warnings before exhaustion.
- Integration controls: OIDC, FCM, UnifiedPush guidance, service-backed card providers, maps, and other optional integrations from one UI.
- Maintenance: guided upgrades, migration checks, backup-before-upgrade, rollback guidance where feasible, logs/diagnostics export with secrets and personal data redacted by default.
- Security-sensitive changes require explicit confirmation and explain their effect before applying.
- Expose advanced/raw configuration only behind an expert view. UI changes and file/API configuration must round-trip to one canonical server configuration model.
- The Admin UI must work well on desktop and mobile web so a small self-hoster can administer Sigil without SSH for routine tasks.

## 4. Implementation milestones and decision gates

| Milestone | Deliverable and completion condition |
|---|---|
| A. UI feasibility and concepts | Run the agreed two-hour first-pass Slint/Compose comparison, counting setup. Use identical synthetic content and release builds on Android. Check styled editing, emoji/RTL/IME behavior, interactive cards, gestures, accessibility, and a 1,000-message timeline. Also prototype the Admin setup flow as a web surface. Record surface-integration checks that cannot fit as unresolved. |
| B. Visual selection | Produce three Android concepts: Material 3 Expressive, a restrained minimal direction, and a distinct content-focused direction. Each shows inbox, rich conversation, attachment creation, and calling; demonstrate optional collections. User selects the direction before production UI styling. |
| C. Dependency and protocol foundations | Pin feasible, license-compatible components; define wire/event/recovery contracts and the security model. Complete group/federation analysis, cryptographic vectors, and core misuse tests. No real-message pilot before this gate. |
| D. Reliable Android messaging | Enrollment/login, discovery, device linking, DMs, groups, Note to Self, cross-server delivery, offline synchronization, notifications, encrypted local storage, and recovery from a lost-phone scenario. |
| E. Conversation and SigilText experience | Complete conversation controls and all SigilText constructs, builders, renderers, Notes, search, organization, shared-state behavior, and accessibility. |
| F. Files, maps, and service cards | Implement the requested attachment matrix, secure expanded viewers, resumable media, PMTiles maps, location sharing, and configured external/self-hosted providers. Unsupported required formats remain blockers requiring a scope decision. |
| G. Calling | Complete audio/video transitions, independent participant invitations, group calls, screen sharing, reconnects, encrypted forwarding, and network-condition testing. |
| H. Android acceptance and deployment | Full feature acceptance, recovery and fault testing, resource/performance checks, dependency review, independent security assessment, and a single-container installation/upgrade/restore test. Admin must complete fresh setup, routine configuration, user/integration management, health review, upgrade, backup, and restore without hand-editing configuration for the normal path. |
| I. Remaining clients | Implement Web, iOS, Linux, Windows, and macOS in that order, reusing shared logic and UI where validated. Each platform passes its own input, accessibility, security, and media checks. |

### Selection rules

- Prefer Slint when essential Android behavior passes and platform integration has a demonstrated feasible path. An unresolved critical check is not a pass. A switch to native UI requires presenting the concrete blocker and obtaining the user’s decision.
- Prefer str0m for the forwarding server if it passes the shared interoperability and reliability tests; select webrtc-rs if it resolves demonstrated str0m integration deficiencies. If both fail, stop at the media gate.
- Prefer embedded Martin functionality when bounded serving and integration tests pass. Evaluate packaging an internal tile-serving process in the same container only if embedding fails.
- Do not start independent cryptography, Office-rendering, codec, or mapping-engine rewrites to bypass a failed dependency evaluation without a separate decision.

## 5. Validation, deployment, and future work

### Initial performance benchmark

Use synthetic workloads on a reference 4-core/8-GiB server, with 50 enrolled users and 20 connected devices. Measure both same-server and cross-server delivery; record network conditions and test-device characteristics without storing identifying details.

Targets under a healthy, controlled network:

- Local send feedback within 100 ms.
- Online recipient-visible text delivery below 500 ms p95, with approximately 50-ms client-to-host RTTs and 50-ms inter-host RTT.
- Call media established within two seconds p95 after acceptance.
- Validate groups of 256 members and calls of eight participants.
- Responsive search and virtualized scrolling with 100,000 synthetic messages.
- Text delivery retains priority during bulk transfers. Report idle/busy battery, memory, and bandwidth measurements; do not invent battery guarantees before device measurements.

### Security and failure tests

- Cryptographic vectors, malformed inputs, replay/reordering, tampering, crash/rollback recovery, revoked devices, username reassignment, and attempted identity substitution.
- Parser/protocol fuzzing, oversized structured cards, decompression bombs, hostile documents, worker crashes, and unsafe external URLs.
- Concurrent group changes, checklist edits, poll closure, offline history sharing, expiration, and multi-device conflicts.
- Fresh-device restoration without the original phone, incorrect recovery material, incomplete backups, deleted-content handling, and storage exhaustion.
- Interrupted uploads, process/container restarts, federation outages, packet loss, changing networks, denied permissions, and delayed pushes.
- Accessibility, large text, RTL, keyboard/gesture conflicts, reduced motion, and screenshot comparisons using synthetic content only.
- Metadata inventory based on observable traffic and server behavior, not absence of logs.
- Independent review before presenting Sigil as production-grade secure messaging.

### Deployment

Ship one Docker image and Compose example suitable for Dockge, with persistent data storage, optional read-only maps, health checks, migration handling, and operator backup/restore instructions. OIDC and external providers remain optional integrations. Document required HTTPS/media connectivity and support reverse-proxy/VPS deployments without embedding personal configuration.

Normal deployment and administration is driven through Admin after the container starts. The documented happy path is: start container → open setup URL → complete guided configuration → operate from the admin UI. Hand-editing configuration remains an expert/debug path, not the expected installation experience.

### Beyond 1.0

Preserve versioned content and authenticated-action interfaces for turn-based games, collaborative content, and optional on-device reply suggestions/transcription. Do not ship a downloaded executable-plugin system initially.

Keep metadata-resistant transport and authenticated account migration as explicit future work. Neither is required for the initial account experience, and neither is represented as already solved.
