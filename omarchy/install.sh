#!/usr/bin/env bash
#
# Installs Sigil — a native Matrix client — with its Omarchy shell frontend.
#
#   1. Bundles the fonts, builds the engine (Rust) and the video QML plugin
#      (C++) via bin/sigil-setup.
#   2. Registers/enables the plugin in the shell.
#   3. Prints the theme layer-rule and keybinding snippets (use --apply to
#      write them into the Frosted Glass theme + bindings.lua).
set -euo pipefail
PLUGIN_ID="pellinore.sigil"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"   # the omarchy/ frontend
REPO="$(cd "$HERE/.." && pwd)"
say() { printf '%s\n' "$*"; }

"$HERE/bin/sigil-setup"

OMARCHY="omarchy"; command -v omarchy >/dev/null 2>&1 || OMARCHY="/usr/share/omarchy/bin/omarchy"
DEST="$HOME/.config/omarchy/plugins/$PLUGIN_ID"
# Symlink rather than copy so edits in the checkout are live. The shell follows
# symlinked plugins; only omarchy-plugin-validate refuses them, so validate
# "$HERE" directly instead of the link.
if [ ! -e "$DEST" ]; then
  say "Linking the shell frontend…"
  ln -sfn "$HERE" "$DEST"
fi
"$OMARCHY" plugin enable "$PLUGIN_ID" right >/dev/null 2>&1 || true

RULE='hl.layer_rule({ match = { namespace = "^(omarchy-sigil|omarchy-sigil-call)$" }, blur = true, ignore_alpha = 0.45, blur_popups = true })'
BIND1='o.bind("SUPER + ALT + M", "Sigil", "omarchy-shell -q shell toggle pellinore.sigil")'
BIND2='o.bind("SUPER + ALT + SHIFT + M", "Sigil: answer or hang up call", "omarchy-shell -q sigil callToggle")'
if [ "${1:-}" = "--apply" ]; then
  B="$HOME/.config/hypr/bindings.lua"
  grep -q "pellinore.sigil" "$B" 2>/dev/null || printf '\n-- Sigil (pellinore.sigil, Matrix client)\n%s\n%s\n' "$BIND1" "$BIND2" >> "$B"
  T="$HOME/.config/omarchy/themes/$(cat "$HOME/.config/omarchy/current/theme.name" 2>/dev/null || echo frosted-glass)/hyprland.lua"
  if [ -f "$T" ] && ! grep -q "omarchy-sigil" "$T"; then printf '\n-- Sigil: frost the chat window and the call banner/pill.\n%s\n' "$RULE" >> "$T"; fi
  hyprctl reload >/dev/null 2>&1 || true; "$OMARCHY" theme refresh >/dev/null 2>&1 || true
  say "Applied theme rule and keybindings."
else
  say ""
  say "Add to ~/.config/hypr/bindings.lua:"; say "  $BIND1"; say "  $BIND2"
  say "Add to your theme's hyprland.lua (or extend the existing overlay blur rule) and run 'omarchy theme refresh':"
  say "  $RULE"
fi
say "Sigil installed. Toggle it with SUPER+ALT+M and sign in."
