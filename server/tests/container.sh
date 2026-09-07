#!/usr/bin/env bash
# Synthetic deployment acceptance. Run from any directory after building the image.
set -euo pipefail
umask 077
image=${1:-sigil-backend:dev}
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
docker image inspect "$image" >/dev/null
scratch=$(mktemp -d /tmp/sigil-container-test.XXXXXX)
container=
original=
restored=
cleanup() {
    if [[ -n "$container" ]]; then docker rm -f "$container" >/dev/null 2>&1 || true; fi
    for volume in "$original" "$restored"; do
        if [[ -n "$volume" ]]; then docker volume rm "$volume" >/dev/null 2>&1 || true; fi
    done
    rm -rf -- "$scratch"
}
trap cleanup EXIT
original=$(docker volume create)
restored=$(docker volume create)
runtime=(--read-only --cap-drop ALL --security-opt no-new-privileges:true --memory 256m --cpus 2 --pids-limit 128)
start() {
    container=$(docker create "${runtime[@]}" --mount "source=$1,target=/var/lib/sigil" -p 127.0.0.1::8080 "$image")
    docker start "$container" >/dev/null
    endpoint="http://$(docker port "$container" 8080/tcp)"
    local deadline=$((SECONDS + 10))
    until curl --noproxy '*' --connect-timeout 2 --max-time 2 -fsS "$endpoint/healthz" -o /dev/null 2>/dev/null; do
        if ((SECONDS >= deadline)); then echo 'Container did not become live.' >&2; exit 1; fi
        sleep 0.1
    done
    docker exec "$container" sigil-server healthcheck
    test "$(docker exec "$container" id -u)" = 65532
}
stop() {
    docker stop --time 10 "$container" >/dev/null
    test "$(docker inspect --format '{{.State.ExitCode}}' "$container")" = 0
    docker rm "$container" >/dev/null
    container=
}
headers() {
    printf 'header = "Authorization: Bearer %s"\n' "$(cat "$scratch/$1")" > "$scratch/$1.conf"
}
request() {
    local expected=$1 auth=$2 method=$3 path=$4 status
    local options=(--noproxy '*' --connect-timeout 2 --max-time 8 -sS -X "$method")
    if [[ "$auth" != - ]]; then options+=(-K "$scratch/$auth.conf"); fi
    if (($# >= 5)); then options+=(-H "Content-Type: ${6:-application/json}" --data-binary "@$scratch/$5"); fi
    status=$(curl "${options[@]}" "$endpoint$path" -o "$scratch/response.json" -w '%{http_code}')
    if [[ "$status" != "$expected" ]]; then
        printf '%s %s: expected %s, received %s\n' "$method" "$path" "$expected" "$status" >&2
        exit 1
    fi
}
start "$original"
docker exec "$container" cat /var/lib/sigil/data/admin.token > "$scratch/admin"
headers admin
request 503 - GET /readyz
printf '%s' '{"expected_revision":0,"settings":{"server_name":"chat.example"}}' > "$scratch/config.json"
request 200 admin PUT /admin/v0/configuration config.json
request 200 - GET /readyz
request 200 admin GET /admin/v0/federation/status
jq -e '.enabled==false and .peers==0 and .egress.reserved_bytes==0 and .ingress.reserved_bytes==0' "$scratch/response.json" >/dev/null
printf '%s' '{"expected_revision":0,"unified_push":true,"contact":"mailto:operator@example.com","exceptions":[],"rotate_vapid":false,"fcm":{"action":"disable"}}' > "$scratch/push-config.json"
request 200 admin PUT /admin/v0/push push-config.json
cp "$scratch/response.json" "$scratch/push-configuration.json"
printf '%s' '{"username":"synthetic","expires_in_seconds":600}' > "$scratch/invite.json"
request 201 admin POST /admin/v0/invitations invite.json
cp "$scratch/response.json" "$scratch/invitation.json"
openssl rand -hex 32 > "$scratch/device"
headers device
jq -n --slurpfile invitation "$scratch/invitation.json" --rawfile credential "$scratch/device" \
    '{invitation:$invitation[0].secret,device_credential:($credential|rtrimstr("\n")),device_label:"Synthetic deployment"}' > "$scratch/enroll.json"
request 201 - POST /client/v0/enroll enroll.json
cp "$scratch/response.json" "$scratch/session.json"
device=$(jq -r .device_id "$scratch/session.json")
account=$(jq -r .account_id "$scratch/session.json")
# A synthetic private endpoint is rejected by egress; no external provider receives this.
jq -n --slurpfile config "$scratch/push-configuration.json" \
    '{expected_revision:0,target:{provider:"unified_push",endpoint:"https://127.0.0.1:9/synthetic",public_key:"BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",auth_secret:"BTBZMqHH6r4Tts7J_aSIgg",vapid_key:$config[0].vapid_public_key}}' > "$scratch/push-register.json"
request 200 device PUT /client/v0/push push-register.json
cp "$scratch/response.json" "$scratch/push-pending.json"
jq -n --arg recipient "$device" --argjson expiry "$(( $(date +%s) + 3600 ))" \
    '{recipient_device:$recipient,message_id:("12"*32),expires_at:$expiry,payload:("ab"*16)}' > "$scratch/message.json"
request 202 device POST /client/v0/messages message.json
cp "$scratch/response.json" "$scratch/receipt.json"
# The server treats this random test object as opaque bytes, not validated E2EE.
openssl rand 36 > "$scratch/object.bin"
object=$(sha256sum "$scratch/object.bin" | cut -d ' ' -f1)
od -An -v -tx1 "$scratch/object.bin" | tr -d ' \n' > "$scratch/object.hex"
jq -n --rawfile ciphertext "$scratch/object.hex" '{ciphertext:$ciphertext}' > "$scratch/object.json"
request 204 device PUT "/client/v0/recovery/objects/$object" object.json
jq -n --arg manifest "$object" '{expected_generation:0,expected_manifest:null,manifest:$manifest}' > "$scratch/head.json"
request 200 device PUT /client/v0/recovery/head head.json
cp "$scratch/response.json" "$scratch/published.json"
# Fixed synthetic encrypted empty-file vector; the access token is disposable.
attachment_fixture="$script_dir/../../crypto/tests/vectors/attachments.json"
attachment=$(jq -r '.[0].descriptor[16:80]' "$attachment_fixture")
attachment_root=$(jq -r '.[0].root' "$attachment_fixture")
attachment_hex=$(jq -r '.[0].chunks[0].ciphertext' "$attachment_fixture")
[[ "$attachment_hex" =~ ^([0-9a-f]{2})+$ ]]
for ((attachment_offset=0; attachment_offset<${#attachment_hex}; attachment_offset+=2)); do
    printf '%b' "\\x${attachment_hex:attachment_offset:2}"
done > "$scratch/attachment.bin"
openssl rand -hex 32 > "$scratch/attachment-access"
jq -n --rawfile access "$scratch/attachment-access" '{plaintext_bytes:0,access_token:($access|rtrimstr("\n")),expires_at:null}' > "$scratch/attachment.json"
request 200 device PUT "/client/v0/attachments/$attachment" attachment.json
request 204 device PUT "/client/v0/attachments/$attachment/chunks/0" attachment.bin application/octet-stream
jq -n --arg root "$attachment_root" '{root:$root,acknowledge_restored_checkpoint:false}' > "$scratch/attachment-publish.json"
request 200 device POST "/client/v0/attachments/$attachment/publish" attachment-publish.json
cp "$scratch/device.conf" "$scratch/device-download.conf"
printf 'header = "sigil-attachment-access: %s"\n' "$(cat "$scratch/attachment-access")" >> "$scratch/device-download.conf"
request 200 device-download GET "/client/v0/attachments/$attachment/chunks/0"
cmp "$scratch/response.json" "$scratch/attachment.bin"
docker kill "$container" >/dev/null
docker rm "$container" >/dev/null
container=
start "$original"
request 200 device GET /client/v0/push
cmp "$scratch/response.json" "$scratch/push-pending.json"
request 200 device GET /client/v0/session
cmp "$scratch/response.json" "$scratch/session.json"
request 202 device POST /client/v0/messages message.json
cmp "$scratch/response.json" "$scratch/receipt.json"
request 200 device GET /client/v0/mailbox
jq -e 'length==1 and .[0].payload==("ab"*16)' "$scratch/response.json" >/dev/null
request 200 device GET /client/v0/recovery/head
cmp "$scratch/response.json" "$scratch/published.json"
request 200 device-download GET "/client/v0/attachments/$attachment/chunks/0"
cmp "$scratch/response.json" "$scratch/attachment.bin"
if docker run --rm "${runtime[@]}" --mount "source=$original,target=/var/lib/sigil" "$image" backup /var/lib/sigil/backup.db > "$scratch/locked-backup.log" 2>&1; then
    echo 'Backup unexpectedly bypassed the running server lock.' >&2; exit 1
fi
stop
docker run --rm "${runtime[@]}" --mount "source=$original,target=/var/lib/sigil" "$image" backup /var/lib/sigil/backup.db > "$scratch/backup.log"
docker run --rm "${runtime[@]}" --mount "source=$original,target=/backup,readonly" --mount "source=$restored,target=/var/lib/sigil" "$image" restore /backup/backup.db > "$scratch/restore.log"
start "$restored"
request 200 - GET /readyz
request 401 admin GET /admin/v0/configuration
request 401 device GET /client/v0/session
docker exec "$container" cat /var/lib/sigil/data/admin.token > "$scratch/admin"
headers admin
request 200 admin GET /admin/v0/push
jq -e '.revision==2 and .unified_push==false and .fcm_project_id==null and .vapid_public_key==null' "$scratch/response.json" >/dev/null
request 200 admin GET /admin/v0/federation/status
jq -e '.enabled==false and .peers==0 and .nonces.reserved_bytes==0' "$scratch/response.json" >/dev/null
request 200 admin GET /admin/v0/configuration
jq -e '.revision==1 and .settings.server_name=="chat.example"' "$scratch/response.json" >/dev/null
printf '%s' '{"expires_in_seconds":600}' > "$scratch/reauth-request.json"
request 201 admin POST "/admin/v0/accounts/$account/reauthorization-invitations" reauth-request.json
cp "$scratch/response.json" "$scratch/invitation.json"
openssl rand -hex 32 > "$scratch/replacement"
headers replacement
jq -n --slurpfile invitation "$scratch/invitation.json" --rawfile credential "$scratch/replacement" \
    '{invitation:$invitation[0].secret,device_credential:($credential|rtrimstr("\n")),device_label:"Synthetic restored deployment"}' > "$scratch/reauth.json"
request 201 - POST /client/v0/reauthorize reauth.json
jq -e --arg account "$account" --arg prior "$device" '.account_id==$account and .device_id!=$prior' "$scratch/response.json" >/dev/null
request 200 replacement GET /client/v0/mailbox
jq -e 'length==0' "$scratch/response.json" >/dev/null
request 200 replacement GET /client/v0/mailbox/senders
jq -e 'length==0' "$scratch/response.json" >/dev/null
request 200 replacement GET /client/v0/recovery/head
jq -e --arg manifest "$object" '.generation==1 and .manifest==$manifest and .restored_checkpoint==true' "$scratch/response.json" >/dev/null
request 200 replacement GET "/client/v0/recovery/objects/$object"
jq -e --slurpfile expected "$scratch/object.json" '.==$expected[0]' "$scratch/response.json" >/dev/null
cp "$scratch/replacement.conf" "$scratch/replacement-download.conf"
printf 'header = "sigil-attachment-access: %s"\n' "$(cat "$scratch/attachment-access")" >> "$scratch/replacement-download.conf"
request 404 replacement-download GET "/client/v0/attachments/$attachment/chunks/0"
request 409 replacement POST "/client/v0/attachments/$attachment/publish" attachment-publish.json
jq -n --arg root "$attachment_root" '{root:$root,acknowledge_restored_checkpoint:true}' > "$scratch/attachment-repair.json"
request 200 replacement POST "/client/v0/attachments/$attachment/publish" attachment-repair.json
request 200 replacement-download GET "/client/v0/attachments/$attachment/chunks/0"
cmp "$scratch/response.json" "$scratch/attachment.bin"
request 204 replacement DELETE "/client/v0/attachments/$attachment"
request 404 replacement-download GET "/client/v0/attachments/$attachment/chunks/0"
stop
echo 'Container acceptance passed: restart, retries, resource limits, backup locking, restore revocation, recovery ciphertext, attachment access/repair/deletion and push restart/restore reset.'
