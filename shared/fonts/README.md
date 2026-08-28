# Bundled fonts

Sigil ships its own fonts rather than asking the system for them. It has to:
the icons are private-use codepoints, and a font that is not present renders
them as blank boxes. That is not only a problem on macOS, Windows, Android, iOS
and Web — Sigil was already relying on fontconfig happening to resolve
`monospace` to a Nerd Font on this machine, and the family it named in two
places (`CaskaydiaMono Nerd Font`) was not installed at all.

| file | role | licence | notice |
|---|---|---|---|
| `Roboto-Regular.ttf` `Roboto-Medium.ttf` `Roboto-Bold.ttf` | UI text | **OFL-1.1** | `LICENSE-OFL-1.1.txt` |
| `RobotoMono-Regular.ttf` | code blocks, formatted content | **OFL-1.1** | `LICENSE-OFL-1.1.txt` |
| `MaterialSymbolsRounded.ttf` | outlined icons | Apache-2.0 | `LICENSE-Apache-2.0.txt` |
| `MaterialSymbolsRounded-Filled.ttf` | filled icons | Apache-2.0 | `LICENSE-Apache-2.0.txt` |

The licences are **not** uniform, and an earlier version of this file wrongly
claimed Apache-2.0 for all six. Google relicensed the Roboto family to the SIL
Open Font License; the shipped binaries say so themselves in name ID 14
(`https://openfontlicense.org`), which is the authoritative check:

    otfinfo -i Roboto-Regular.ttf | grep -i license
    # or: strings Roboto-Regular.ttf | grep openfontlicense

Both licences permit bundling in an application, including a paid or
app-store-distributed one. The obligations that actually bind us:

- **Ship the licence text.** OFL-1.1 requires the copyright notice and licence
  travel with the font, which is why `LICENSE-OFL-1.1.txt` sits next to the
  files rather than only being referenced here. Apache-2.0 requires its notice
  the same way. Any packaging step — an `.app` bundle, an `.apk`, a WASM
  payload — has to carry both files, not just the `.ttf`s.
- **Never sell the fonts on their own.** OFL forbids selling the font files by
  themselves; bundled in Sigil is fine.
- **Renaming on modification.** If the fonts are ever subsetted (see below),
  OFL's Reserved Font Name clause governs. Roboto declares no reserved name, so
  a subset may keep the family name, but the notice must still ship.

Roboto and Material Symbols are designed together, which is why the icons sit
correctly on the text baseline.

Total ~3.4 MB. Material Symbols ships as two **static instances** rather than
the 30 MB variable font: the FILL axis is what separates outlined from filled,
and Google serves a static instance per axis value. They register under
different family names (`Material Symbols Rounded` and
`Material Symbols Rounded Filled`), so choosing one is `font.family`, with
nothing to configure and no variable-axis support required.

Both carry identical codepoints, so one icon table serves them both.

Not subsetted. Cutting them to the ~85 icons actually used would take the pair
under 100 KB, which will matter for the Web target — but it would also mean a
new icon silently rendering as nothing until someone regenerates the subset, so
it is deliberately left for when there is a build step to hang it on.

Sources:
- Roboto / Roboto Mono — Google Fonts (`fonts.googleapis.com/css2`)
- Material Symbols Rounded — Google Fonts, browsable at
  <https://fonts.google.com/icons> (set Style to "Rounded" to match)

Use the **canonical** codepoints from that repo's `.codepoints` file — the ones
fonts.google.com/icons shows. Material Symbols reaches most glyphs from several
codepoints (aliases kept for legacy Material Icons compatibility): `mood`
answers to U+E24E, U+E420, U+E7F2 and U+EA22, all drawing the same glyph.
Reading one out of the font's own cmap therefore "works" but yields an
arbitrary alias that matches no documentation. Take the canonical value and
verify it exists in the bundled `.ttf`.

Rounded and Outlined share codepoints exactly, so changing style is a file swap
plus the family name in `components/Fonts.qml` — no codepoint work.

Icons are referenced by name through the `Icons` singleton, never as literals.
Its codepoints live in `shared/icons.json`; `shared/icongen` generates the QML.
See `docs/portability.md`.
