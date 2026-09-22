#!/usr/bin/env bash
# Synthetic camera tracks only. No capture devices, accounts, or audio.
# Open the printed URL; ?recovery drops ten receive frames every 600, ?burst releases 150 ms receive bursts, ?plain bypasses transforms, ?revoke checks revocation and worker expiry.
set -euo pipefail
cd "$(dirname "$0")/../.."
lab=$(mktemp -d)
trap 'rm -rf "$lab"' EXIT
CFLAGS_wasm32_unknown_unknown=-std=gnu2x cargo build --locked --release -p sigil-browser --features video-acceptance --target wasm32-unknown-unknown
mkdir -p "$lab/web"
wasm-bindgen "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release/sigil_browser.wasm" --target web --out-dir "$lab/web"
cat > "$lab/index.html" <<'HTML'
<!doctype html><meta charset="utf-8"><title>Silent video acceptance</title><style>body{font:16px sans-serif;background:#17212b;color:#eee}pre{white-space:pre-wrap}</style><p>Loading silent synthetic video test…</p><script type="module">import init,{video_test_start} from '/web/sigil_browser.js';await init();await video_test_start(1920,1080,60);</script>
HTML
cat > "$lab/web/sigil-media-worker.mjs" <<'JS'
import init,{media_worker_start} from './sigil_browser.js';self.onmessage=async({data})=>{self.onmessage=null;await init({module_or_path:data.module});media_worker_start(data.port,data.gate);};
JS
cat > "$lab/web/sigil-video-control.mjs" <<'JS'
import init,{video_test_control_start} from './sigil_browser.js';self.onmessage=async({data})=>{self.onmessage=null;await init({module_or_path:data.module});video_test_control_start(data.port,data.gate,data.fixture,data.revoke);};
JS
rustc --edition=2024 browser/tests/video/serve.rs -o "$lab/serve"
"$lab/serve" "$lab"
