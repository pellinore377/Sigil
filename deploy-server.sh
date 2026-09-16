#!/usr/bin/env bash
# Build the server image on the machine that runs it, then restart the stack there.
# Avoids a hosted runner: the build host keeps its own layer cache and never moves
# the image across the network.
#
#   ./deploy-server.sh              build, deploy, wait for health
#   ./deploy-server.sh --skip-tests skip the server test run
#
# Configure with an untracked deploy-server.conf beside this script, or the
# environment:
#   SIGIL_BUILD_HOST  ssh destination that builds and runs the image (required)
#   SIGIL_BUILD_DIR   directory on that host to sync sources into
#   SIGIL_STACK_DIR   directory on that host holding the compose file
#   SIGIL_SERVICE     compose service name
#   SIGIL_IMAGE       image tag to build
set -euo pipefail

source=$(cd "$(dirname "$0")" && pwd)
# shellcheck disable=SC1091
[ -f "$source/deploy-server.conf" ] && . "$source/deploy-server.conf"

host=${SIGIL_BUILD_HOST:?set SIGIL_BUILD_HOST, or create deploy-server.conf}
remote=${SIGIL_BUILD_DIR:-sigil-build}
stack=${SIGIL_STACK_DIR:?set SIGIL_STACK_DIR, or create deploy-server.conf}
service=${SIGIL_SERVICE:-sigil}
image=${SIGIL_IMAGE:-ghcr.io/pellinore377/sigil:latest}

if [[ ${1:-} != --skip-tests ]]; then
  printf 'Running server tests\n'
  cargo test -q -p sigil-server
fi

printf 'Copying sources to the build host\n'
rsync -a --delete --delete-excluded \
  --exclude '.git/' --exclude 'target/' --exclude 'build/' --exclude '.gradle/' \
  --exclude '*/target/' --exclude 'app/build/' \
  "$source/" "$host:$remote/"

printf 'Building %s\n' "$image"
ssh "$host" "cd '$remote' && DOCKER_BUILDKIT=1 docker build --tag '$image' ."

printf 'Restarting the stack\n'
ssh "$host" "cd '$stack' && docker compose up -d"

printf 'Waiting for health\n'
ssh "$host" "
  cd '$stack'
  container=\$(docker compose ps -q '$service')
  [ -n \"\$container\" ] || { echo 'service is not running'; exit 1; }
  for attempt in \$(seq 1 60); do
    state=\$(docker inspect \"\$container\" --format '{{.State.Health.Status}}' 2>/dev/null || echo missing)
    [ \"\$state\" = healthy ] && { echo healthy; exit 0; }
    [ \"\$state\" = unhealthy ] && { docker logs --tail 20 \"\$container\"; exit 1; }
    sleep 2
  done
  echo 'timed out waiting for health'; docker logs --tail 20 \"\$container\"; exit 1
"
printf 'Deployed %s\n' "$image"
