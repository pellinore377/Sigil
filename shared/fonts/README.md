# Bundled fonts

Sigil ships its own fonts rather than asking the system for them. It has to:
the icons are private-use codepoints, and a font that is not present renders
them as blank boxes. That is not only a problem on macOS, Windows, Android, iOS
and Web — Sigil was already relying on fontconfig happening to resolve
`monospace` to a Nerd Font on this machine, and the family it named in two
places (`CaskaydiaMono Nerd Font`) was not installed at all.

| file | family | role | licence | notice |
|---|---|---|---|---|
| `GoogleSansFlex.ttf` | `Google Sans Flex` | UI text, every weight | **OFL-1.1** | `LICENSE-OFL-1.1.txt` |
| `GoogleSansCode.ttf` | `Google Sans Code` | code blocks, formatted content | **OFL-1.1** | `LICENSE-OFL-1.1.txt` |
| `MaterialSymbolsRounded.ttf` | `Material Symbols Rounded` | outlined icons | Apache-2.0 | `LICENSE-Apache-2.0.txt` |
| `MaterialSymbolsRounded-Filled.ttf` | `Material Symbols Rounded Filled` | filled icons | Apache-2.0 | `LICENSE-Apache-2.0.txt` |
| `NotoColorEmoji.ttf` | `Noto Color Emoji` | emoji | **OFL-1.1** | `LICENSE-OFL-1.1.txt` |

Google Sans is the face the platform messenger is set in, which is the whole
reason it is here: Roboto shipped until Google open-sourced the family, and
every screen was a half-step off the reference because of it. **Google Sans
Flex** is the single family that replaced both Google Sans and Google Sans
Text — one variable font covering Thin through Black — and **Google Sans Code**
is the monospace companion.

## One file per family, not one per weight

Both are **variable** fonts, so 400 / 500 / 700 all come out of one file:
nothing asks for `Roboto-Medium.ttf` any more, it asks for `font-weight: 500`
and the shaper drives the `wght` axis. The axes each file actually carries:

| file | axes |
|---|---|
| `GoogleSansFlex.ttf` | `wght` 1–1000 (def 400), `opsz` 6–144 (def 18), `wdth` 25–151, `GRAD` 0–100, `ROND` 0–100, `slnt` −10–0 |
| `GoogleSansCode.ttf` | `wght` 300–800 (def 400), `MONO` 0–1 (def 1) |

Both files are the **canonical variable releases, shipped as they are** —
`GoogleSansFlex[GRAD,ROND,opsz,slnt,wdth,wght].ttf` and
`GoogleSansCode[MONO,wght].ttf`, renamed and otherwise untouched. Nothing is
instanced to a static weight anywhere in the build; a weight is a `wght` value
at render time. (The release also carries an `android/` build of Flex with the
same six axes and the same glyphs, only repackaged. We ship the canonical one.)

Flex defaults to `opsz` 18, which is the body size, so nothing has to set it.

`Face::parse` alone is **not** enough to read these. See "gvar" below.

## gvar: `ttf-parser` needs the `gvar-alloc` feature

Sigil rasterises glyphs itself in two places — `slint/src/fx.rs` (the glow
halo behind an effect message) and `core/src/maps/labels.rs` (map labels) —
both with `ttf-parser`. Google Sans Flex has more than 32 variation tuples per
glyph, and `ttf-parser` keeps tuples in a fixed 32-slot **stack** array unless
the `gvar-alloc` feature is on; over that it gives up and `outline_glyph`
returns `None`. Not an error, not a panic — every glyph silently comes back
empty. Both crates therefore depend on:

    ttf-parser = { version = "0.25", features = ["gvar-alloc"] }

With it, outlines work and the weight really moves: `n` at `wght` 400 is 878
units of ink and 1122 of advance, at 700 it is 1014 and 1245.

## Coverage

Google Sans Flex and Google Sans Code are **Latin** (~540 and ~675 mapped
codepoints): Latin-1, Latin Extended-A, Vietnamese, the common punctuation and
currency, and the arrows and box drawing Code needs. Roboto carried Greek and
Cyrillic; these do not. Slint falls back to a system face for UI text in a
script the bundled family lacks, but Sigil's own two rasterisers have no
fallback — `core/src/maps/labels.rs` drops a label whose name it cannot spell,
so a Cyrillic street name now goes unlettered rather than being drawn.

## Licences

The licences are **not** uniform, and an early version of this file wrongly
claimed Apache-2.0 for all of them. Both Google Sans families are under the SIL
Open Font License; the shipped binaries say so themselves in name ID 14
(`https://openfontlicense.org`), which is the authoritative check:

    strings GoogleSansFlex.ttf | grep openfontlicense

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
  OFL's Reserved Font Name clause governs. Neither family declares a reserved
  name, so a subset may keep the family name, but the notice must still ship.

## Size

Material Symbols ships as two **static instances** rather than the 30 MB
variable font: the FILL axis is what separates outlined from filled, and Google
serves a static instance per axis value. They register under different family
names (`Material Symbols Rounded` and `Material Symbols Rounded Filled`), so
choosing one is `font.family`, with nothing to configure and no variable-axis
support required. Both carry identical codepoints, so one icon table serves
them both.

The two text files are 4.45 MB against Roboto's four static faces at 0.44 MB —
a **+4.0 MB** swap, essentially all of it Flex's `gvar` (3.6 MB of its 4.18).
That is the price of one file that does every weight. Cutting the axes we never
touch (`wdth`, `GRAD`, `ROND`, `slnt`, and pinning `opsz`) would take most of
it back, but that is instancing, and the point of shipping the variable file is
not to.

Nothing here is subsetted. Cutting the icons to the ~85 actually used would
take that pair under 100 KB, which will matter for the Web target — but it
would also mean a new icon silently rendering as nothing until someone
regenerates the subset, so it is deliberately left for when there is a build
step to hang it on.

Sources:
- Google Sans Flex 4.007 — <https://github.com/googlefonts/googlesans-flex>
- Google Sans Code 7.001 — <https://github.com/googlefonts/googlesans-code>
- Material Symbols Rounded — Google Fonts, browsable at
  <https://fonts.google.com/icons> (set Style to "Rounded" to match)
- Noto Color Emoji — <https://github.com/googlefonts/noto-emoji>

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
