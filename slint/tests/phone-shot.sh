#!/usr/bin/env bash
# The app on a phone-shaped virtual screen: the real desktop window through
# the same windowing backend the Android build uses for scale and resize,
# at a phone's density, captured after it settles. Catches layout that
# runs off the screen before an APK does. Needs Xvfb, xwd, ImageMagick
# and slint/target/debug/sigil-slint.
#   phone-shot.sh <out.png> [scale=2.625] [WxH=1100x2300] [seconds=10]
set -euo pipefail
OUT=${1:?out.png}; SCALE=${2:-2.625}; GEOM=${3:-1100x2300}; WAIT=${4:-10}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
W=$(mktemp -d); mkdir -p "$W/state" "$W/cache"
trap 'rm -rf "$W"' EXIT
SIGIL_SLINT_SHARED=1 XDG_STATE_HOME=$W/state XDG_CACHE_HOME=$W/cache HOME=$W \
WINIT_X11_SCALE_FACTOR=$SCALE LIBGL_ALWAYS_SOFTWARE=1 \
  xvfb-run -a -s "-screen 0 ${GEOM}x24" bash -c "
    '$ROOT/slint/target/debug/sigil-slint' >'$W/app.log' 2>&1 & APP=\$!
    sleep $WAIT
    xwd -root -silent | convert xwd:- '$OUT'
    kill \$APP" || { echo "FAIL: app"; cat "$W/app.log" | tail -30; exit 1; }
grep -q panicked "$W/app.log" && { echo "FAIL: panic"; grep -A3 panicked "$W/app.log"; exit 1; }
echo "phone-shot ok: $OUT"
