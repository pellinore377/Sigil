# SigilText

**The rich text and structured content syntax for Sigil.**

Design principle: *intuitive over terse.* Markdown is unintuitive; SigilText should read naturally to someone who has never seen it before.

---

## Contents

- **Part I — Implemented** — core grammar, Markdown, colors, modifiers, animations
- **Part II — Specified, Not Yet Built** — checklists onward
- **Part III — Cross-cutting Constraints**

> **Note:** the line between Parts I and II is drawn at checklists. Some modifiers in Part I were specified after the initial implementation (`underline`, `mark`, `mono`, `small`/`big`, `redact`, `scratch`, and the `sparkle`/`glitch`/`blur`/`flip`/`barrel` animations) and may still be pending. Verify against the codebase.

---

# PART I — IMPLEMENTED

## 1. Core grammar

Everything follows one shape:

```
modifier::content;
```

Modifiers stack, separated by `::`, and are **order-independent**:

```
shake::bold::red::text;
```

The terminator is an unescaped `;`.

### Grammar categories

| Category | Shape | Examples |
|---|---|---|
| Inline modifiers | `modifier::text;` | `red::`, `shake::`, `bold::`, `mark::` |
| Single-line blocks | `keyword::content;` | `note::`, `timer::`, `calc::`, `qr::` |
| Multi-line blocks | `keyword::[opts::]Title` + items + `;` | `checklist::`, `poll::`, `chart::` |
| Standard Markdown | unchanged | `**bold**`, `` `code` ``, `- item`, `> quote` |

---

## 2. Standard Markdown

Parsed via `pulldown-cmark`. Spec-compliant Matrix HTML; renders correctly in Element and other clients.

| Feature | Syntax | HTML output |
|---|---|---|
| Bold | `**text**` | `<strong>` |
| Italic | `*text*` or `_text_` | `<em>` |
| Bold + italic | `***text***` | `<strong><em>` |
| Strikethrough | `~~text~~` | `<del>` |
| Inline code | `` `text` `` | `<code>` |
| Code block | ` ```lang ` … ` ``` ` | `<pre><code class="language-…">` |
| Blockquote | `> text` | `<blockquote>` |
| Unordered list | `- item` | `<ul><li>` |
| Ordered list | `1. item` | `<ol><li>` |
| Checklist item | `- [ ]` / `- [x]` | task list item |
| Link | `[text](url)` or bare URL | `<a href>` |
| Heading | `# text` | `<h1>`–`<h6>` (consider starting at h3) |
| Spoiler | `\|\|text\|\|` | `<span data-mx-spoiler>` |
| Line break | Shift+Enter | `<br>` |

**Code is backticks only.** No `code::` modifier — a `;` terminator collides with `};` in most languages. Headings remain Markdown-only, as they are block-level.

---

## 3. Colors

Nine hues × three brightness levels, plus `rainbow`. Each hue is defined for a **dark ground** and a **light ground**; the engine ships both and the frontend draws with the one matching its background, so a colour is never unreadable and never differs between clients.

| Base | Variants |
|---|---|
| `red` | `red1` `red2` `red3` |
| `orange` | `orange1` `orange2` `orange3` |
| `yellow` | `yellow1` `yellow2` `yellow3` |
| `green` | `green1` `green2` `green3` |
| `cyan` | `cyan1` `cyan2` `cyan3` |
| `blue` | `blue1` `blue2` `blue3` |
| `purple` | `purple1` `purple2` `purple3` |
| `pink` | `pink1` `pink2` `pink3` |
| `gray` | `gray1` `gray2` `gray3` |
| `rainbow` | auto-distributes across characters |

`1` is lightest, `3` is darkest. Bare name aliases the mid variant (`red` = `red2`).

### Reference values

Canonical, per ground. A client must not substitute its own, or the same message
renders differently per platform. `1` is lightest, `3` is darkest.

| Hue | dark `1` | dark `2` | dark `3` | light `1` | light `2` | light `3` |
|---|---|---|---|---|---|---|
| `red` | `#FF544E` | `#D94742` | `#95312E` | `#D3342F` | `#9C2723` | `#6C1B18` |
| `orange` | `#FF9740` | `#E08538` | `#9B5B27` | `#DA7623` | `#A2581A` | `#6F3C12` |
| `yellow` | `#FFD949` | `#DEBD40` | `#99822C` | `#D8B42C` | `#A08520` | `#6E5C16` |
| `green` | `#8AF87C` | `#66B85C` | `#467F3F` | `#5AB24F` | `#42843A` | `#2E5B28` |
| `cyan` | `#65F8FF` | `#4DBDC2` | `#358286` | `#3DB7BC` | `#2D878C` | `#1F5D60` |
| `blue` | `#62A3FF` | `#548CDB` | `#3A6197` | `#427FD5` | `#315E9E` | `#22416D` |
| `purple` | `#BB89FF` | `#9970D1` | `#6A4D90` | `#8E62CB` | `#694897` | `#493268` |
| `pink` | `#FF82C5` | `#E073AD` | `#9B4F78` | `#DA63A3` | `#A24978` | `#6F3253` |
| `gray` | `#CFCFD5` | `#99999E` | `#6A6A6D` | `#94949A` | `#6E6E72` | `#4C4C4F` |

Derivation, so the table can be regenerated rather than trusted: the `2` column
is the base hue. Level `1` multiplies the HSV value by **1.35**, level `3`
divides it by **1.45**, both clamped to `[0, 1]` with hue and saturation
preserved. The light-ground set is the dark-ground base with its value scaled by
**0.72** and saturation by **1.12** before the level is applied.

`rainbow` cycles hue across the run at fixed **saturation 0.62** and
**lightness 0.62**, HSL, position `0.0 → 0.999` over the span's characters.

Bare `mark` with no colour of its own highlights with `yellow2`.

Gradient stops are resolved individually; interpolation between them is linear
in RGB and happens per-frame in the frontend, since it depends on each
character's position within the run.

Each span carries `rgb: { "dark": "#RRGGBB", "light": "#RRGGBB" }` — gradients
carry an array of such pairs, and `mark` carries `markRgb` in the same shape. A
frontend selects by the ground it is drawing on and never resolves a name
itself. That is what keeps `red::text;` identical across clients while staying
legible on either background.

### Gradients

Hyphen-separated color list, distributed across the content's characters:

```
red1-blue3::text;
red-yellow-green::text;
```

**The hyphen means gradient separator only.** No hyphenated color names may ever be added — use numeric suffixes instead.

---

## 4. Text modifiers

| Modifier | Effect | Output |
|---|---|---|
| `bold` | bold | `<strong>` |
| `italic` | italic | `<em>` |
| `strike` | strikethrough | `<del>` |
| `underline` | underline | `<u>` |
| `mono` | monospace styling (no code semantics) | Sigil effect |
| `mark` | highlight / background color | `<span data-mx-bg-color>` |
| `spoiler` | tap to reveal | `<span data-mx-spoiler>` |
| `redact` | permanently blacked out, not revealable | Sigil effect |
| `scratch` | animated static; swipe to reveal (invisible-ink style) | Sigil effect |
| `small1` `small2` `small3` | reduce size (floor ~0.7×) | Sigil effect |
| `big1` `big2` `big3` | increase size (ceiling ~1.6×) | Sigil effect |

Bare `small` / `big` alias to step 2. **Sizes must be clamped** — reject or saturate beyond step 3 to prevent timeline layout breakage.

`mark` composes with colors: `mark::yellow::text;`

**Reveal mechanics differ:** `spoiler` = tap, `blur` = hover, `scratch` = swipe progressively, `redact` = never.

**`underline` emits `<u>`**, which is outside the Matrix HTML whitelist. Element renders it; strict clients may strip it. Acceptable degradation.

---

## 5. Animations

**Last one wins** if multiple are given.

| Modifier | Effect |
|---|---|
| `shake` | horizontal jitter |
| `wave` | letters ripple up and down |
| `pulse` | opacity/scale oscillation |
| `glow` | animated outer glow |
| `typewriter` | reveals one character at a time |
| `sparkle` | particle overlay |
| `glitch` | RGB-split distortion |
| `blur` | blurred until hover |
| `flip` | rotated 180° |
| `barrel` | barrel roll |

### Reference timings

The engine ships these; a frontend fetches them once and drives its own
animation system with the numbers rather than choosing any. Over the socket:
`sigiltext.motion`. Linked in-process: `sigiltext_motion()`. Source:
`core/src/timeline/motion.rs`.

Per-character stagger is `index * 90 ms`, reduced per effect by the modulo
shown — that stagger *is* the effect, and a client using a different one
produces visibly different motion from identical text.

| Animation | Steps (ms) | Motion | Easing | Stagger |
|---|---|---|---|---|
| `shake` | 80, 80, 80 | x ±0.8 px | linear | `% 160` |
| `wave` | 520, 520, 260 | y ±1.8 px | in-out sine | none |
| `pulse` | 500, 500 | scale 1.0 ↔ 1.18 | in-out quad | `% 300` |
| `glow` | 1600 | alpha 0 ↔ 1 | in-out quad | none |
| `sparkle` | 900 | 3 particles, scale 1.1↔1.5, alpha 0.05↔0.45 | in-out sine | none |
| `glitch` | 90, 70, 110, 260 | x −2 / +1.5 px | linear | stride 53, `% 400` |
| `typewriter` | 620 | reveal per character | out cubic | none |
| `flip` | — | 180°, run reversed | — | none |
| `barrel` | 1200 | 360°, continuous | linear | none |
| `blur` | 900 | radius → 6 px | in-out quad | none |

`glitch` splits into two chromatic-aberration copies at `#FF2640` (leading) and
`#26F2FF` (trailing), each at alpha 0.7 and offset oppositely.

Sizes: `small3`..`big3` map to **0.7, 0.8, 0.9, 1.0, 1.2, 1.4, 1.6** by step
`-3..=3`, clamped.

**Easings are cubic Bézier control points, not names.** `InOutSine` in Qt,
`FastOutSlowInEasing` in Compose and `ease-in-out` in CSS are three different
curves; control points are the only encoding all three read identically. The
engine emits them in CSS `cubic-bezier(x1, y1, x2, y2)` order:

| Name | Control points |
|---|---|
| linear | `0, 0, 1, 1` |
| in-out sine | `0.37, 0, 0.63, 1` |
| in-out quad | `0.45, 0, 0.55, 1` |
| out cubic | `0.33, 1, 0.68, 1` |

Particle systems (`sparkle`) and shader effects (`glitch`, `blur`) genuinely
differ between toolkits. Expect the same family, not identical pixels; sharing
the parameters is what stops them diverging further.

All motion stops under a reduced-motion setting and when the span is off-screen.

---

## 6. Escapes

| Sequence | Renders as |
|---|---|
| `\;` | literal `;` inside a span |
| `\\` | literal `\` |
| `\*` `\_` `\~` `\|` | literal Markdown delimiter |
| `\red::` | literal `red::` (escaped opener) |

```
shake::red1-blue3::text\; lorem ipsum;
```

---

## 7. Parsing rules

1. Run Markdown first via `pulldown-cmark`.
2. Apply spoiler and modifier passes **only to `Text` events** in the event stream — never `Code` or `CodeBlock`. This makes `` `red::foo;` `` literal automatically.
3. Classify each `::`-separated token before the content: animation set → animation; color set → color; hyphenated all-colors → gradient; otherwise **not a modifier** — render literally.
4. Unterminated span (`red::text` with no `;`) → style to end of line.
5. Empty content (`red::;`) → no-op.
6. Sanitize output HTML before rendering received messages.
7. **Single colons inside a segment are legal** — `remind::9:30am::text;` splits on `::` only. Too many segments → not a valid modifier → render literally.

---

## 8. Event representation

Standard fields carry Markdown-derived HTML so other clients render normally. Sigil-only effects go in a custom field:

```json
{
  "msgtype": "m.text",
  "body": "text lorem ipsum",
  "format": "org.matrix.custom.html",
  "formatted_body": "text lorem ipsum",
  "com.sigil.text_effects": [
    {
      "start": 0,
      "end": 16,
      "color": {"type": "gradient", "stops": ["red1", "blue3"]},
      "animation": "shake"
    }
  ]
}
```

```rust
struct TextEffect {
    start: usize,
    end: usize,
    color: Option<ColorSpec>,      // Solid(String) | Gradient(Vec<String>) | Rainbow
    animation: Option<Animation>,
}
```

Parser emits this; the UI renders from it. Adding a modifier type later is an enum change, not a pipeline change.

---

## 9. Contacts

### `@::` composer trigger

Typing `@::` opens an inline picker from the homeserver user directory (`GET /_matrix/client/v3/user_directory/search`).

**Distinct from mentions.** Bare `@` inserts a mention pill; `@::` attaches a shareable contact card.

**Directory scope:** Synapse returns only users the homeserver knows — local users plus remote users sharing a room.

```json
{
  "msgtype": "m.text",
  "body": "Contact: Alice (@alice:example.org)",
  "com.sigil.contact": {
    "type": "matrix",
    "user_id": "@alice:example.org",
    "display_name": "Alice",
    "avatar_url": "mxc://example.org/abc123"
  }
}
```

`body` must be human-readable and contain the MXID, since non-Sigil clients display it verbatim.

### vCard support

Receiving a `.vcf` renders a contact card. Exporting generates a vCard with the MXID in both `X-MATRIX-ID` and `NOTE`, so it survives import into address books that drop unknown fields.

---

## 10. UI affordances

| Feature | Interaction |
|---|---|
| Bold / Italic / Underline / Strike | Ctrl+B / Ctrl+I / Ctrl+U / Ctrl+Shift+X |
| Inline code | Ctrl+Shift+M |
| Mentions | `@` → autocomplete |
| Contact share | `@::` → directory picker |
| Rooms | `#` → autocomplete |
| Emoji | `:` → picker |
| Color / animation | select text → context menu |
| Paste rich text | convert HTML → Markdown |

**Live composer preview** — render formatting in the composer rather than showing raw syntax. The single biggest UX difference between "supports Markdown" and "feels good."

---

# PART II — SPECIFIED, NOT YET BUILT

Everything below extends the grammar above. **Block constructs** span multiple lines, take a title line and item lines, and terminate with `;`.

**All constructs work in any room** — do not gate structured content behind a room type. A checklist in a chat and a checklist in a notes room behave identically.

---

## 11. Checklists

```
checklist::Title
-x- Item 1
- Item 2
- Item 3;
```

- Title is everything on the same line as `checklist::`
- Items begin on subsequent lines with `- `
- `-x- ` marks an item pre-checked
- Items support inline modifiers: `- red::urgent;`
- **Adding, removing, and reordering after creation is UI only**, not syntax

### List types

| Type | Syntax | Check behavior |
|---|---|---|
| Standard | `checklist::Title` | Freely checkable and uncheckable. Items removed manually. |
| Recurring | `checklist::recurr::weekly::Title` | Items **cannot** be manually unchecked — only the scheduled reset unchecks them. |
| Task | `checklist::task::Title` | Irreversible after a 30-second undo window. Records who completed it and when. |

### Recurring lists

```
checklist::recurr::weekly::Groceries
-r- Milk
-r- Eggs
- Bananas;
```

- `-r- ` marks a **persistent** item — unchecks on reset, stays on the list
- Plain `- ` items are **one-offs** — deleted on reset
- Intervals: `weekly`, `monthly`, `yearly`

**Reset timing:** 12:01 AM local time on the target day, so the list is ready before the user wakes up.

- `weekly` — same weekday as creation
- `monthly` — same day-of-month as creation
- `yearly` — same month and day as creation

**Month-end clamping:** clamp to the **last day of the target month**, not a fixed value. Created on the 31st → Jan 31, Feb 28 (29 in leap years), Mar 31. It must snap back to the intended day whenever the month allows.

**DST:** compute reset times in the user's local timezone, not UTC, so the 12:01 AM guarantee holds across transitions.

### Tasks

```
checklist::task::Move-in punch list
- Patch drywall
- Replace outlet cover;
```

- Tapping opens a confirmation dialog: **Complete** / **Cancel**
- On confirm: `Completed by <name> · <time>` with an **Undo** affordance
- Undo available for **30 seconds**, then permanent
- **Only the completing user may undo** — avoids cross-device races
- Implementation: hold the completion event locally during the window, or send and redact on undo. Prefer holding for two-person lists; prefer send-and-redact if delayed shared visibility matters.
- Render attribution compactly (checkmark + small avatar), full detail on tap

---

## 12. Polls (syntax path)

A fast path producing the **same event as the existing poll attachment builder**. Do not build a parallel implementation.

```
poll::closed::multi2::Question
- Option 1
- Option 2
- Option 3;
```

| Segment | Values | Default |
|---|---|---|
| Disclosure | `open` (results visible before voting) / `closed` (hidden until you vote) | `open` |
| Selection | `multi` (unlimited) or `multi2`, `multi3`… (capped) | single |

Both optional — `poll::Question` with items is valid.

Maps to `org.matrix.msc3381.poll.start`. `open`/`closed` correspond to the spec's disclosed/undisclosed kinds. **Element renders these natively**, so emit spec-compliant events.

---

## 13. Reminders

```
remind::07/05/27 9:30am::Call a plumber;
remind::tomorrow 16:45::Take out trash;
remind::next week::Research marine biology;
```

### Date handling — critical

Accept natural, locale-familiar input, but **resolve to an absolute timestamp at compose time and transmit that**. Never send the raw string for the recipient to reinterpret — `07/05/27` means different dates in different locales, and both parties must see the same moment.

- Parse ambiguous numeric formats using the **sender's** locale
- Store an unambiguous timestamp in the event
- Each client **displays** it in the recipient's preferred format

**Confirm the parse in the composer** — show "July 5, 2027 at 9:30 AM" before sending, so users catch misinterpretation and typos.

Accept relative forms (`tomorrow`, `next week`, `friday`) alongside absolute dates. Where no time is given, default to **9:00 AM**.

### Notification

Reminders in a shared room notify **all members**. Each client schedules its own platform-local notification (Android AlarmManager, iOS local notifications, desktop equivalent) on seeing the event. Matrix has no server-side scheduling.

Fired reminders remain in the Notes tab as history.

---

## 14. Notes

```
note::Be happy!;
```

A single-line message flagged for the Notes tab. Users should also be able to **promote an existing message to a note via UI** — likely more common than the syntax.

---

## 15. Timers

```
timer::1 hour 45 min;
```

- **Store an absolute end timestamp**, not a duration — duration plus received-time drifts between clients; absolute `ends_at` keeps countdowns synchronized ("sync your watches")
- Accept flexible input: `1 hour 45 min`, `1h45m`, `90 minutes`, `1.5 hours`, `45s`
- **Confirm the parse in the composer** — show `1:45:00` before send
- Renders as a live countdown; transitions to an **ended-state bubble** on expiry, same pattern as a closed poll
- Each client schedules a local notification for `ends_at`
- **Timers do not appear in the Notes tab**

---

## 16. Notes tab

A per-room tab alongside Pinned. **Computed from the timeline** — not room state, and **not auto-pinned**. Auto-pinning would pollute `m.room.pinned_events`, which is room-wide state visible to all clients.

**Contents:** notes, checklists, reminders. Not timers, not polls.

**Sections:**
- **Active** — open checklists, upcoming reminders
- **Past** — fired reminders, completed lists. Collapsed by default.

Reminder history is retained deliberately. Without sectioning, a year of fired reminders makes the tab useless for finding current items.

---

## 17. Charts

Uses `=` for values, since `::` is the modifier separator and would collide with inline modifiers in item labels.

```
chart::pie::Title
- Category 1 = 30%
- Category 2 = 20%
- Category 3 = 50%;
```

| Type | Notes |
|---|---|
| `pie` | Percentages or raw values; normalize if they don't sum to 100 |
| `donut` | As pie, with a hollow center |
| `bar` | Raw values |
| `line` | Ordered series |
| `area` | As line, filled beneath |
| `scatter` | Numeric x = y pairs |

Item labels support inline modifiers. Values accept percentages or plain numbers.

**Anything with axes and values belongs under `chart::`.** Adding a new visualization should be a renderer change, not a grammar change.

---

## 18. Diagrams

Mermaid is a JS library with no complete Rust implementation, so SigilText defines its own syntax.

### Flowchart

```
diagram::flow::Deploy Process
- Start -> Build
- Build -> Test
- Test -> Deploy [yes]
- Test -> Build [no];
```

`->` defines an edge; bracketed text is an edge label. Node shapes inferred, or declared: `{Decision}` diamond, `[Process]` rectangle, `(Start)` rounded.

### Sequence

```
diagram::sequence::Call Flow
- Sigil -> Synapse: send message
- Synapse -> LiveKit: create room
- LiveKit --> Sigil: room ready;
```

`->` solid, `-->` dashed (typically a response). Text after `:` is the message label.

### Timeline

```
diagram::timeline::Project
- 2026-03 = Started
- 2026-06 = Beta
- 2026-09 = Launch;
```

### Mind map

```
diagram::mindmap::Sigil
- Sigil -> Messaging
- Sigil -> Media
- Messaging -> SigilText
- Media -> Voice memos;
```

Radial layout from the root node.

### Org chart

```
diagram::org::Team
- Alice -> Bob
- Alice -> Carol
- Bob -> Dave;
```

Strict top-down hierarchy. Reject cycles.

### State machine

```
diagram::state::Call
- Idle -> Ringing [invite]
- Ringing -> Connected [answer]
- Ringing -> Idle [decline]
- Connected -> Idle [hangup];
```

Rounded state nodes, labelled transitions.

**Anything with nodes and relationships belongs under `diagram::`.** All types share the same `->` edge syntax; only the layout algorithm differs. A simple layered (Sugiyama-style) approach is adequate for chat-sized diagrams — don't over-engineer.

---

## 19. Tables

Far friendlier than hand-aligning Markdown pipes.

```
table::Name | Role
- Alice | Manager
- Bob | Engineer;
```

- Title line defines column headers, separated by `|`
- Each row uses the same separator
- Rows with fewer cells pad with empty; more cells truncate or wrap — pick one and document it
- **Emit a real HTML `<table>`** so Element renders it too

---

## 20. Recipes

```
recipe::Carbonara
serves::4
time::25 min
ingredients:
- 200g guanciale
- 4 egg yolks
- 100g pecorino
steps:
- Render the guanciale over medium heat
- Whisk yolks and cheese
- Combine off heat, using pasta water to loosen;
```

- `serves::` and `time::` are optional metadata lines
- `ingredients:` and `steps:` are section markers (single colon, own line)
- Renders as a card: metadata header, ingredient list, numbered steps
- **Ingredients individually checkable while cooking** — same interaction as a standard checklist, not persisted

---

## 21. Math

```
math::E = mc^2;
```

Block form:

```
math::block
\int_0^\infty e^{-x^2} dx = \frac{\sqrt{\pi}}{2};
```

Emit `<span data-mx-maths="...">` with a plaintext fallback in the element body, so **Element renders it natively** rather than showing raw LaTeX.

Rendering options: `katex` bindings, or MathJax via a small JS runtime. **Verify viability in the target toolkit before committing** — same rendering-vs-parsing gap that ruled out Mermaid.

---

## 22. Countdowns

```
countdown::2027-07-05::Launch;
ago::2019-03-14::Project started;
```

- `countdown::` — days remaining until a future date, live-updating
- `ago::` — elapsed time since a past date
- Both use the same date parsing and compose-time resolution as `remind::`
- Distinct from `timer::`, which is minutes-scale and notifies

---

## 23. Calculation & conversion

Render as a result chip that **preserves the original expression**.

| Syntax | Renders |
|---|---|
| `calc::17 * 34;` | `17 × 34 = 578` |
| `convert::20C;` | `20°C = 68°F` |
| `convert::5 miles;` | `5 miles = 8.05 km` |

- `calc` via `meval` or equivalent — **arithmetic only**, never arbitrary code evaluation
- `convert` auto-detects direction: metric input yields imperial and vice versa
- Cover temperature, distance, weight, volume, speed
- Computed at render time on each client; no service dependency

---

## 24. Randomizers

```
roll::2d6;
roll::2d6, 1d20, 2d12;
pick::pizza, tacos, thai;
pick::number::1-100;
pick::food;
pick::coin;
```

**`roll::`** — standard dice notation. Multiple groups comma-separated; show each group's result and a combined total. **Animate the dice in** on render.

**`pick::`** — three modes:

| Form | Behavior |
|---|---|
| `pick::a, b, c;` | Chooses from explicit options |
| `pick::number::1-100;` | Random integer in range |
| `pick::<category>;` | Chooses from a built-in list |

Built-in categories: `food`, `movie`, `book`, `coin`. Each needs a curated list of a few dozen entries — **this is content to maintain**, not just logic. Allow user-defined categories in settings.

`pick::coin;` replaces a standalone coin-flip keyword, since `flip::` is the upside-down animation.

---

## 25. Display helpers

| Syntax | Renders |
|---|---|
| `swatch::#ff5733;` | Filled color chip beside the value |
| `swatch::rgb(255,87,51);` | Same |
| `swatch::rgba(255,87,51,0.5);` | Chip over a checkerboard so alpha is visible |
| `swatch::hsl(9,100%,60%);` | Same |
| `kbd::Ctrl+Shift+P;` | Rendered key caps — split on `+`, each a bordered rounded rect |
| `rate::4/5;` | Star rating |
| `progress::75;` | Progress bar |
| `quote::Marcus Aurelius::You have power over your mind;` | Formatted pull-quote with attribution |

`quote::` optionally takes a source: `quote::Marcus Aurelius::Meditations::text;`. Distinct from Markdown `>`, which is for quoting conversation.

---

## 26. ASCII art

```
art::
    /\_/\
   ( o.o )
    > ^ <
;
```

- Locks monospace, **preserves every space**, disables wrapping and reflow
- **No inline modifier parsing inside** — content is literal
- Horizontal scroll if wider than the bubble; wrapping would destroy the art
- Terminator is `;` alone on its own line

Effectively a code block without highlighting or a language label.

---

## 27. QR codes

```
qr::https://example.org;
qr::wifi::MyNetwork::password123;
qr::contact::@alice:example.org;
qr::text::anything at all;
```

Typed forms exist because the raw wifi payload (`WIFI:S:name;T:WPA;P:pass;;`) is full of semicolons and would need escaping throughout — construct it internally instead.

- `qrcode` crate generates the matrix
- **Medium error correction** default — far more tolerant of poor scanning conditions than Low
- **Preserve the quiet zone.** QR requires white margin; don't crop tight to the bubble
- **Force a light background even in dark theme.** Most scanners expect dark modules on light; white-on-dark frequently fails. This is the single most common QR rendering mistake.

---

## 28. Service-backed constructs

All three require a backend. **Self-host each** for consistency with the privacy architecture. Configure endpoints in settings; **degrade gracefully to literal text when unreachable** — never hang.

### Translation

```
translate::es::Where is the library?;
translate::auto::¿Dónde está la biblioteca?;
```

Renders original and translation together with the target language labelled. `auto` means "detect source, translate to my locale."

**Backend:** LibreTranslate — self-hostable via Docker.

### Definitions

```
define::petrichor;
```

Renders word, pronunciation, part of speech, and definition as a card.

**Backend:** a trimmed English Wiktionary extract (a few hundred MB) hosted locally, preferred over a third-party API.

### Weather

```
weather::Springfield;
weather::Springfield::forecast;
```

Current conditions or a multi-day forecast card.

**Backend:** Open-Meteo — free, no API key, no tracking, publishes a self-hostable Docker image. Running your own means location queries stay on your infrastructure, which matters given the architecture exists to avoid leaking location.

---

## 29. Self-documentation

```
sigil::
sigil::colors;
sigil::animations;
```

Renders an inline SigilText reference card. **With this much surface, discoverability is the real problem** — this puts the cheat sheet where people already are instead of only in a PDF.

- Bare `sigil::` → category index
- `sigil::<category>;` → that section
- Categories: `colors`, `animations`, `text`, `lists`, `utility`, `charts`

---

# PART III — CROSS-CUTTING CONSTRAINTS

## Architecture

**Everything platform-independent lives in the Rust core.** The test: *would you write it identically in QML, Kotlin, and Swift?* If yes, it belongs in Rust.

In core:
- SigilText parsing → fully-resolved render tree
- Structured content parsing (checklists, polls, charts, diagrams)
- Business logic (recurrence math, date resolution, timer state)
- Animation **specifications** (not execution)

In each UI:
- Drawing
- Platform integrations (notifications, pickers, camera)
- Animation **execution**, driven by core's parameters

If you find yourself writing SigilText logic in Kotlin, that's a signal it belongs in core.

## Animation consistency

Animations will drift between platforms unless parameterized centrally. Specify in core:

```rust
struct AnimationSpec {
    kind: Animation,
    duration_ms: u32,
    easing: CubicBezier,   // explicit control points, not named constants
    params: AnimationParams,
}

// shake: amplitude_px, cycles
// wave: amplitude_px, wavelength_chars, phase_offset_per_char
// pulse: min_scale, max_scale
// sparkle: particle_count, lifetime_ms, spread_px, gravity
```

Named easing constants differ between toolkits — Compose's `FastOutSlowInEasing`, QML's `Easing.InOutQuad`, and CSS's `ease-in-out` are all different curves. Use explicit control points.

**Accept that particles and shaders will look similar rather than identical** across platforms. Keep parameters shared so they stay in the same family.

**Safeguard:** maintain a test room with one message per effect. Screenshot on each platform and compare. Drift rots silently otherwise.

## General constraints

- **Respect reduce-motion.** Honor the system preference; provide a Sigil-level toggle. Animated text is hostile to motion-sensitive users.
- **Performance.** Pause animations for messages scrolled out of view. Per-character animated items across a long timeline will otherwise burn CPU.
- **Lazy rendering.** Don't parse or render heavy constructs (diagrams, QR, large art) until visible.
- **Graceful degradation.** Always populate `body` and `formatted_body` so non-Sigil clients show readable text.
- **Theme safety.** Never emit raw hex from color names — resolve at render time against the active theme.
- **Service failures never hang.** Translation, definitions, and weather fall back to literal text with a clear error state.

---

## Test cases

### Inline modifiers
- Adjacent spans: `red::A;blue::B;`
- Unterminated: `red::text`
- Empty: `red::;`
- Inside code span: `` `red::foo;` ``
- Inside fenced block
- Non-color word: `std::vector`
- Escaped opener: `\red::text;`
- Escaped terminator: `red::a\;b;`
- Modifier order: `shake::red::x;` vs `red::shake::x;`
- Gradient with brightness: `red1-blue3::x;`
- Multiple animations: `shake::wave::x;` (last wins)
- Rainbow on single char: `rainbow::A;`
- Conflicting sizes: `big3::small1::x;` (last wins)
- Out-of-range: `small4::text;` (not a modifier)
- Three-modifier stack: `underline::bold::red::text;`
- `mark::yellow::text;` and bare `mark::text;`

### Checklists & tasks
- Standard list, check and uncheck freely
- `-x-` pre-checked on creation
- Recurring list — manual uncheck blocked
- Weekly created Friday → resets following Friday, 12:01 AM
- Monthly created on the 31st → Feb 28, Mar 31 (clamp then snap back)
- Monthly on the 31st in a leap year → Feb 29
- Reset across a DST boundary still fires 12:01 AM local
- `-r-` items survive reset unchecked; plain items removed
- Item with inline modifier: `- red::urgent;`
- Item with literal semicolon: `- buy milk\; and eggs`
- Task: tap → dialog; Cancel → no change; Complete → attribution + undo
- Undo within 30s → reopens; after 30s → not offered
- Undo by a different user → not offered
- Two devices same user, complete then undo → consistent

### Polls
- `poll::Question` with no option segments
- `poll::closed::Question`
- `poll::multi2::Question`
- Event matches attachment-builder output
- Element renders it natively

### Reminders, countdowns, timers
- `remind::07/05/27 9:30am::text;` → US locale → July 5
- Same string, UK sender → May 7; recipient sees sender's resolved date
- `remind::tomorrow::text;` → defaults 9:00 AM
- `remind::tomorrow 16:45::text;` → 24h parses
- Composer shows resolved date before send
- Fired reminder appears in Notes → Past
- `countdown::` future date; `ago::` past date
- `timer::1 hour 45 min;`, `1h45m`, `90 minutes` all equivalent
- Two clients show identical remaining time
- Expired timer → ended-state bubble
- Timer absent from Notes tab

### Charts, diagrams, tables, recipes
- Pie/donut values not summing to 100 → normalize
- Scatter with numeric x = y pairs
- Area and line render the same data differently
- Item label with inline modifier: `- red::urgent; = 40%`
- Item label containing `=` → define first-`=`-wins or escape
- Flow with a cycle
- Flow with edge labels
- Sequence with `->` and `-->`
- Timeline with out-of-order dates
- Mindmap with multi-level nesting
- Org chart containing a cycle → reject with a clear error
- State machine with self-transition (`- Idle -> Idle`)
- Single-node diagram
- Malformed edge (`- A B`) → literal
- Table row with fewer cells than headers
- Table emits valid HTML Element renders
- Recipe missing optional `serves::` / `time::`
- Recipe ingredients checkable, not persisted
- Recipe step with literal semicolon

### Utility
- `calc::17 * 34;`, `calc::(5+3)/2;`
- `calc::` invalid expression → literal, no crash
- `convert::20C;` → °F; `convert::68F;` → °C
- `convert::` unknown unit → literal
- `roll::2d6;` → 2–12
- `roll::2d6, 1d20;` → per-group plus total
- `roll::0d6;`, `roll::1d0;` → literal
- `pick::` single option
- `pick::number::100-1;` → reversed range handled
- `pick::unknowncategory;` → literal
- All four `swatch::` formats; invalid color → literal
- `rate::7/5;` → clamp or reject
- `progress::150;` → clamp to 100
- `kbd::` single key, no `+`
- `math::` inline and block; invalid LaTeX → literal
- `math::` renders in Element via `data-mx-maths`
- Art: whitespace preserved exactly
- Art: `red::text;` inside stays literal
- Art wider than bubble → horizontal scroll, no wrap
- QR: all four typed forms
- QR: wifi password with special characters
- **QR: verify scannability in dark theme specifically**
- QR: very long input still scannable

### Service-backed
- Each with service reachable
- Each with service **unreachable** → literal fallback, clear error state, **no hang**
- `translate::auto::` detection
- `define::` word with multiple senses
- `define::` nonexistent word
- `weather::` ambiguous place name

### General
- Every construct inside a code span or fenced block → literal
- Every construct in a normal chat room → identical to notes room
- Unterminated block (no `;`) → defined behavior
