# Sigil

Self-hostable encrypted messaging under development. Rust owns backend/domain logic; Compose is the provisional UI framework. Use synthetic data until backend completion and security review.

[Plan](docs/plan.md) · [Current status](docs/Status.md) · [Security boundaries](docs/Security.md) · [Design](docs/Design.md) · [SigilText](docs/SigilText.md) · [Maps/providers](docs/Integrations.md) · [Calling](docs/Calls.md)

## Build and check

Use Cargo.lock and the toolchain pinned in Dockerfile.

```sh
cargo build --locked --release -p sigil-server
cargo test --locked --release --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
docker build -t sigil-backend:dev .
bash server/tests/container.sh
bash client/tests/federation.sh
bash calls/tests/relay.sh
bash client/tests/load.sh
```

Eleven tests are excluded from the ordinary workspace run: three parent-invoked crash helpers, a fixture generator, the disk test below, two-server acceptance, attachment format acceptance, two TURN fixtures and two controlled load tests. `client/tests/federation.sh` runs federation with synthetic DNS/TLS inside an isolated Linux container. `calls/tests/relay.sh` needs Docker, OpenSSL and jq; it runs pinned synthetic Coturn on loopback ports 39781–39813 with UDP/TCP/TLS. Its TCP/TLS bridge is test-only; platform media adapters remain separate. Run the large disk test separately:

```sh
cargo test --locked --release -p sigil-client one_gib_file_stages_restarts_and_releases_its_independent_cache_budget -- --ignored
```

Tag the previous image before rebuilding to test an actual upgrade: `docker tag sigil-backend:dev sigil-backend:previous`. Then pass both images to `bash server/tests/container.sh sigil-backend:dev sigil-backend:previous`; this also checks downgrade rejection. Load tests enforce 4 CPUs/8 GiB, 50 accounts/20 active devices, simulated request latency, competing encrypted uploads and 100,000 retained messages. They do not measure platform UI or physical network behavior.

Fuzzing uses `cargo-fuzz` and nightly Rust; corpus/artifacts stay outside version control:

```sh
cargo run --locked --manifest-path fuzz/Cargo.toml --example seeds
cargo +nightly-2026-09-06 fuzz build
for target in content wire checkpoints federation; do
  cargo +nightly-2026-09-06 fuzz run "$target" -- -max_total_time=180 -timeout=5 -rss_limit_mb=2048
done
cargo audit
```

Dependency advisories and mitigations are documented in [Security.md](docs/Security.md#metadata-and-dependencies); the audit is not advisory-free.

## Run and configure

Attachment acceptance: install Bubblewrap, FFmpeg, LibreOffice and `heif-enc`, then run `bash media/tests/runtime.sh`. It downloads checksum-pinned PDFium without V8/XFA into a temporary directory. Set `SIGIL_TEST_OFFICE_LIBRARIES` if LibreOffice is outside `/usr/lib`; `SIGIL_TEST_PDFIUM` selects an existing library. Preview isolation currently targets Linux; other platform adapters remain client acceptance work.

```sh
docker compose up --build -d
```

Compose binds loopback port 8080 and retains data in the `sigil-data` volume. Remote access requires a trusted HTTPS reverse proxy. The Admin UI is unfinished.

For a native process, set `SIGIL_DATA_DIR` to a private directory outside the repository and run `target/release/sigil-server serve`. `SIGIL_LISTEN` defaults to `127.0.0.1:8080`. Directories/files require permissions 0700/0600.

Admin requests use `Authorization: Bearer <admin.token contents>`. Keep this installation token private. `GET/PUT /admin/v0/configuration` uses revision checks; initial configuration can be:

```json
{"expected_revision":0,"settings":{"server_name":"chat.example","default_quota_bytes":10737418240,"max_attachment_bytes":1073741824}}
```

The homeserver name becomes immutable. `/healthz` reports liveness; `/readyz` reports configured availability, not product completion. `/versions` distinguishes opaque storage APIs from complete messaging support.

Account roles, OIDC, discovery, configuration and guided maintenance APIs are
described in [Administration.md](docs/Administration.md). The Admin UI is pending.

## Maintenance

The Admin maintenance API supports live snapshots and restore staging. Stop the service before offline maintenance. Native commands use `SIGIL_DATA_DIR`:

```sh
target/release/sigil-server backup /private/backup/server.db
target/release/sigil-server rotate-admin-token
SIGIL_DATA_DIR=/private/new-installation target/release/sigil-server restore /private/backup/server.db
```

With Compose: stop `sigil`, run `docker compose run --rm --no-deps sigil backup /var/lib/sigil/backup.db`, export with `docker compose cp sigil:/var/lib/sigil/backup.db /private/backup/server.db`, then restart. Use a new private backup filename each time and store the export outside the repository on separate storage.

Restore into a new installation, retaining the original volume. Restore revokes device credentials/invitations, clears prekeys and queued delivery, and creates a new Admin token. Retained recovery ciphertext and attachments require trusted reconciliation; see [Recovery.md](docs/Recovery.md). Backups include server-side keys and metadata, but no client-held keys.

Before upgrading, export a stopped-service backup, retain the image identifier, and rehearse restore on disposable storage. Older binaries must not open migrated databases. Never restore a live client database as history recovery. SQLite deletion is not physical key erasure.

Dependency notices are in [licenses/](licenses/). Project licensing remains unselected; no Signal implementation code is permitted.
