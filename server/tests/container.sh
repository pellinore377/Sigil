#!/usr/bin/env bash
# Synthetic deployment acceptance. Run from any directory after building the image.
set -euo pipefail
umask 077
image=${1:-sigil-backend:dev}
previous=${2:-$image}
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
    container=$(docker create "${runtime[@]}" --mount "source=$1,target=/var/lib/sigil" -p 127.0.0.1::8080 "${2:-$image}")
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
start "$original" "$previous"
docker exec "$container" cat /var/lib/sigil/data/admin.token > "$scratch/admin"
headers admin
request 200 admin GET /admin/v0/diagnostics
previous_schema=$(jq -er '.schema' "$scratch/response.json")
request 503 - GET /readyz
printf '%s' '{"expected_revision":0,"settings":{"server_name":"chat.example"}}' > "$scratch/config.json"
request 200 admin PUT /admin/v0/configuration config.json
request 200 - GET /readyz
request 200 admin GET /admin/v0/calls
jq -e '.settings==null and .has_turn_secret==false' "$scratch/response.json" >/dev/null
printf '%s' '{"expected_revision":0,"settings":{"bind":"0.0.0.0:34780","advertised":"127.0.0.1:34780","max_calls":1,"turn_urls":[]},"turn_secret":{"action":"clear"}}' > "$scratch/calls.json"
request 200 admin PUT /admin/v0/calls calls.json
request 200 admin GET /admin/v0/calls/status
jq -e '.ready==true and .calls==0 and .participants==0 and .dropped_packets==0' "$scratch/response.json" >/dev/null
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
request 200 device GET /client/v0/calls
jq -e '.enabled==true and .max_participants==8' "$scratch/response.json" >/dev/null
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
request 200 admin GET /admin/v0/diagnostics
current_schema=$(jq -er '.schema' "$scratch/response.json")
test "$current_schema" -ge "$previous_schema"
request 200 - GET /versions
jq -e '.contact_requests==[0,1] and .contact_directory==[0]' "$scratch/response.json" >/dev/null
printf '%s' '{"username":"synthetic"}' > "$scratch/directory.json"
request 401 - POST /client/v0/contact-directory directory.json
request 200 device POST /client/v0/contact-directory directory.json
jq -e '.account.address=="@synthetic:chat.example" and .bindings==[] and .links==[]' "$scratch/response.json" >/dev/null
request 200 device GET /client/v0/contact-requests
jq -e '.requests==[] and .next==null' "$scratch/response.json" >/dev/null
request 200 device GET /client/v0/contact-requests/policy
jq -e '.enabled==true' "$scratch/response.json" >/dev/null
printf '%s' '{}' > "$scratch/invalid-contact.json"
request 422 device POST /client/v0/contact-requests invalid-contact.json
request 200 - GET /client/v0/login
jq -e '.server_name=="chat.example" and .password==false and .sso==false' "$scratch/response.json" >/dev/null
printf '%s' '{"revision":0,"enabled":true}' > "$scratch/password-policy.json"
request 200 admin PUT /admin/v0/password-login password-policy.json
request 200 - GET /client/v0/login
jq -e '.password==true' "$scratch/response.json" >/dev/null
printf '%s' '{"password":"a synthetic long container password"}' > "$scratch/password.json"
request 200 admin PUT "/admin/v0/accounts/$(jq -r .account_id "$scratch/session.json")/password" password.json
jq -n '{username:"synthetic",password:"a synthetic long container password",device_credential:("ac"*32),device_label:"Synthetic phone"}' > "$scratch/password-login.json"
request 428 - POST /client/v0/login/password password-login.json
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
if ((previous_schema < current_schema)); then
    if docker run --rm "${runtime[@]}" --mount "source=$original,target=/var/lib/sigil" "$previous" backup /var/lib/sigil/downgrade.db > "$scratch/downgrade.log" 2>&1; then
        echo 'An older binary unexpectedly opened the migrated database.' >&2; exit 1
    fi
    rg -q 'cannot open compatible server storage' "$scratch/downgrade.log"
fi
docker run --rm "${runtime[@]}" --mount "source=$original,target=/var/lib/sigil" "$image" backup /var/lib/sigil/backup.db > "$scratch/backup.log"
docker run --rm "${runtime[@]}" --mount "source=$original,target=/backup,readonly" --mount "source=$restored,target=/var/lib/sigil" "$image" restore /backup/backup.db > "$scratch/restore.log"
start "$restored"
request 200 - GET /client/v0/login
jq -e '.password==false' "$scratch/response.json" >/dev/null
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
maintenance() {
    request 200 admin POST /admin/v0/maintenance/prepare operation.json
    operation_id=$(jq -er '.id' "$scratch/response.json")
    jq -e '.state=="confirmation_required" and (.effect|length)>0' "$scratch/response.json" >/dev/null
    printf '%s' '{"confirm":true}' > "$scratch/confirm.json"
    request 200 admin POST "/admin/v0/maintenance/actions/$operation_id" confirm.json
    local deadline=$((SECONDS + 30))
    while true; do
        request 200 admin GET "/admin/v0/maintenance/actions/$operation_id"
        if jq -e '.state=="complete"' "$scratch/response.json" >/dev/null; then break; fi
        if jq -e '.state=="failed"' "$scratch/response.json" >/dev/null || ((SECONDS >= deadline)); then
            echo 'Guided maintenance failed or timed out.' >&2; exit 1
        fi
        sleep 0.2
    done
}
request 200 admin GET /admin/v0/setup
request 200 admin GET /admin/v0/diagnostics
jq -e '.redacted==true and .schema==34' "$scratch/response.json" >/dev/null
printf '%s' '{"kind":"backup"}' > "$scratch/operation.json"
maintenance
backup_id=$(jq -er '.result.file' "$scratch/response.json")
request 200 admin GET "/admin/v0/maintenance/files/$backup_id/0"
cp "$scratch/response.json" "$scratch/guided.db"
backup_size=$(stat -c %s "$scratch/guided.db")
test "$backup_size" -gt 8192
test "$backup_size" -lt 4194304
backup_hash=$(sha256sum "$scratch/guided.db" | cut -d ' ' -f 1)
jq -n --argjson bytes "$backup_size" --arg sha256 "$backup_hash" '{bytes:$bytes,sha256:$sha256}' > "$scratch/upload.json"
request 200 admin POST /admin/v0/maintenance/files upload.json
upload_id=$(jq -er '.' "$scratch/response.json")
request 200 admin PUT "/admin/v0/maintenance/files/$upload_id/0" guided.db application/octet-stream
request 200 admin GET "/admin/v0/maintenance/files/$upload_id"
jq -e --argjson bytes "$backup_size" '.received==$bytes' "$scratch/response.json" >/dev/null
jq -n --arg file "$upload_id" '{kind:"import",file:$file}' > "$scratch/operation.json"
maintenance
jq -n --arg file "$upload_id" '{kind:"restore",file:$file}' > "$scratch/operation.json"
maintenance
jq -e '.result.restart_required==true' "$scratch/response.json" >/dev/null
request 200 replacement GET /client/v0/session
stop
start "$restored"
request 200 - GET /readyz
request 401 replacement GET /client/v0/session
request 200 admin GET /admin/v0/configuration
request 200 admin GET /admin/v0/maintenance/actions
jq -e 'length==0' "$scratch/response.json" >/dev/null
stop
echo 'Container acceptance passed: restart, retries, resource limits, offline and guided backup/import/restore, revocation, recovery ciphertext, attachment lifecycle and push reset.'
