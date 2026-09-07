# Backend deployment checks and operator procedure

This is a synthetic-data deployment candidate, not a completed messaging release. Production UI is paused. The independent protocol/security audit follows backend readiness; it has not happened yet.

Server schema 17 adds disabled-by-default federation discovery, authenticated admission and durable inbound/outbound mailbox delivery to durable push and opaque attachment storage; native schema 52 adds durable structured actions, authenticated cards and canonical SigilText delivery and recovery alongside push and media retention. See [push acceptance](Push.md), [attachment acceptance](Attachments.md) and [group acceptance](Groups.md). Server restore cancels unfinished uploads and blocks reads of restored published files until explicit owner reconciliation against a trusted expected root. Native transfer scheduling pauses those restored files; automatic republication and complete media-retention propagation remain unfinished. A server snapshot alone cannot prove deletion freshness.

## Reproduce acceptance

From the repository root, with Docker, Bash, curl, jq, OpenSSL and standard Unix utilities installed:

```sh
docker build -t sigil-backend:dev .
bash server/tests/container.sh
```

The script creates disposable anonymous volumes, uses only synthetic accounts and opaque test payloads, binds an ephemeral loopback port, and removes its resources on exit. It checks non-root execution, a read-only root filesystem, dropped capabilities, the deployment resource limits, configuration readiness, persistent credentials and exact retries after SIGKILL/container replacement, offline backup locking, read-only backup restore, credential revocation, and preserved recovery ciphertext. It does not test upstream Signal interoperability, actual home hardware, sustained load, TLS termination or an upgrade from a previously released image.

The Rust process test separately checks acknowledged writes across SIGKILL and CLI backup/restore. Database files must be regular private files (0600); the data directory must be private (0700). Existing shared permissions fail closed rather than being silently changed.

## Local installation

```sh
docker compose up -d --build
curl --fail http://127.0.0.1:8080/healthz
```

The supplied Compose file targets Linux amd64. Persistent storage is the `sigil-data` named volume, mounted at `/var/lib/sigil`; the database and installation Admin credential live under `data/`. Do not use `docker compose down -v` on an installation you intend to keep.

`/healthz` is liveness; `/readyz` returns 503 until the homeserver name has been configured through the Admin API. A 200 readiness response means the implemented server configuration is ready, not that every planned messaging capability is available. The homeserver name becomes immutable after configuration. Current API usage is described in the repository README; a finished Admin setup interface is still pending.

HTTP is published only on `127.0.0.1:8080`. Remote clients require a trusted HTTPS reverse proxy with a valid certificate. Keep the database and Admin credential out of the proxy's filesystem. Configure and test the actual hostname, certificate chain and proxy before a home-server pilot; this repository does not yet provide that environment-specific configuration.

## Offline backup

Run from the original Compose project directory. Choose a new backup filename each time; backup deliberately refuses to overwrite an existing file.

```sh
umask 077
backup_name="sigil-$(date -u +%Y%m%dT%H%M%SZ).db"
docker compose stop sigil
docker compose run --rm --no-deps sigil backup "/var/lib/sigil/$backup_name"
docker compose cp "sigil:/var/lib/sigil/$backup_name" "./$backup_name"
chmod 600 "./$backup_name"
docker compose start sigil
```

Check each command's success before proceeding. Move the exported backup to private storage outside the repository and onto a separate failure domain. The copy inside the Docker volume is staging, not protection from disk loss; remove that staged copy after verifying the exported backup. Backups contain account/routing metadata, credential hashes and ciphertext, but exclude the installation Admin credential and client-held keys. Backups are standalone SQLite files using DELETE journal mode, so restoring from a read-only mount does not require creating WAL sidecars.

## Restore into a fresh installation

Keep the original volume intact. Stop the original service before starting a replacement on the same port. In the commands below, `sigil-restored` must be a **new, unused** Compose project name, and `backup_file` must point to the private exported backup.

```sh
backup_file=/absolute/path/to/private/server-backup.db
docker compose stop sigil
docker compose -p sigil-restored build sigil
docker compose -p sigil-restored run --rm --no-deps -T --entrypoint sh sigil \
  -c 'umask 077; set -C; cat > /var/lib/sigil/restore.db' < "$backup_file"
docker compose -p sigil-restored run --rm --no-deps sigil restore /var/lib/sigil/restore.db
docker compose -p sigil-restored up -d sigil
curl --fail http://127.0.0.1:8080/readyz
```

Stop if any step fails. Restore validates the supported schema, rejects triggers/views, stages the destination and refuses an existing database or Admin token. It creates a new Admin credential, revokes old device credentials and invitations, clears prekeys and queued delivery, and retains encrypted history-recovery objects with an explicit restored-checkpoint flag. Account reauthorization can issue a new device credential; it does not restore live cryptographic sessions or peer verification.

**The native history importer refuses operator-restored checkpoints.** A surviving archive can authorize repair when the server's restored head matches its trusted checkpoint, its exact pending successor, or an ancestor proven by its bounded ledger of 64 authenticated manifests. Older-backup repair retains the newest local anchor and publishes above it; local edits and tombstones are preserved. Native APIs support an atomic history-only handoff into a freshly reauthorized client for the same account before repair. Unproven/conflicting backups, missing anchors, pending imports and user-facing recovery orchestration remain unresolved. Preserved ciphertext is not yet a complete end-user disaster-recovery flow. A server backup is never a backup of live client ratchet state. See `Recovery.md` for the repair contract.

## Upgrade and remaining gates

Before upgrading, stop the service, take and export an offline backup, and retain the old image identifier. New binaries migrate supported server schemas transactionally and reject future schemas. Rehearse upgrades and recovery using disposable copies; do not run an older binary against a migrated database. Restoring an older server snapshot follows the credential-revoking procedure above, not a transparent rollback.

Operational acceptance is only one part of backend readiness. Shared session/device implementation acceptance is complete; see [SessionsDevices.md](SessionsDevices.md). Remaining backend gates include durable client key-erasure policy, recovery reconciliation and retention lifecycle, full encrypted-event/group/federation contracts, and the remaining server services. Platform lifecycle integration, actual home hardware, HTTPS proxy behavior, load/resource budgets and release-to-release upgrades also require validation. Independent protocol/security review follows the backend implementation gates. See `plan.md`, `TripleRatchet.md`, `Recovery.md`, and `GroupsFederation.md` for the current boundaries.
