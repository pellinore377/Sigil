# SigilText

**The rich text and structured content syntax for Sigil.**

Design principle: *intuitive over terse.* SigilText should make advanced rich and structured content understandable without requiring users to memorize punctuation-heavy syntax. Standard Markdown remains supported for familiar text formatting.

---

## Contents

- **Part I — Core Language** — grammar, Markdown, colors, modifiers, animations, identity, composer behavior
- **Part II — Structured Content & Utilities** — lists, polls, time, data, diagrams, utilities, service-backed cards, help
- **Part III — Cross-cutting Constraints** — consistency, accessibility, performance, degradation, test coverage

---

# PART I — CORE LANGUAGE

## 1. Core grammar

All SigilText constructs begin from the same `keyword::...` grammar. Inline constructs use the basic shape below and terminate with `;`; structured constructs extend the same grammar across multiple lines.

```
modifier::content;
```

Modifiers stack, separated by `::`. Modifiers from different categories are **order-independent**:

```
shake::bold::red::text;
red::shake::bold::text;
```

Both examples are equivalent because animation, emphasis, and color are different categories. Within an **exclusive category**, the last modifier wins. For example, multiple animations or multiple size modifiers are resolved left-to-right with the final value taking precedence.

The terminator is an unescaped `;`.

For ordinary inline content, `;` ends the inline construct. Inside a structured block item, inline modifiers are line-scoped instead, so they end at the item boundary and the final unescaped `;` remains available to terminate the enclosing block.

### Grammar categories

| Category | Shape | Examples |
|---|---|---|
| Inline modifiers | `modifier::text;` | `red::`, `shake::`, `bold::`, `mark::` |
| Single-line blocks | `keyword::content;` | `note::`, `timer::`, `calc::`, `qr::` |
| Multi-line blocks | `keyword::[opts::]Title` + items + `;` | `checklist::`, `poll::`, `chart::` |
| Standard Markdown | unchanged | `**bold**`, `` `code` ``, `- item`, `> quote` |

---

## 2. Standard Markdown

Parsed with a CommonMark-compatible Markdown parser. Emit sanitized rich-text HTML plus a readable plaintext fallback so rendering degrades gracefully in clients that do not understand Sigil-specific features.

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
| Spoiler | `\|\|text\|\|` | Sigil spoiler span |
| Line break | Shift+Enter | `<br>` |

**Code is backticks only.** No `code::` modifier — a `;` terminator collides with `};` in most languages. Headings remain Markdown-only, as they are block-level.

---

## 3. Colors

Nine hues × three brightness levels, plus `rainbow`. All resolve to **theme-aware** values — map to ANSI 0–15 or the active theme. **Never emit raw hex from a color name**, or text becomes unreadable on themes you didn't test.

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
| `mark` | highlight / background color | Sigil highlight span |
| `spoiler` | tap to reveal | Sigil spoiler span |
| `redact` | permanently redacted; original content is not exposed in rendered or fallback output | Sigil effect |
| `scratch` | animated static; swipe to reveal (invisible-ink style) | Sigil effect |
| `small1` `small2` `small3` | reduce size (floor ~0.7×) | Sigil effect |
| `big1` `big2` `big3` | increase size (ceiling ~1.6×) | Sigil effect |

Bare `small` / `big` alias to step 2. Only steps 1–3 are valid. Out-of-range forms such as `small4` or `big9` are not recognized as modifiers and render literally; do not clamp or saturate them.

`mark` composes with colors: `mark::yellow::text;`

**Reveal mechanics differ:** `spoiler` = tap, `scratch` = swipe progressively, `redact` = never. On touch, `scratch` reveals by finger swipe; on desktop, use click-drag/pointer-drag across the span with an equivalent keyboard-accessible **Reveal** action so the effect is never pointer-only.

`redact` is a semantic privacy transform, not merely a visual black bar. The original redacted substring must not appear in plaintext fallback, generic rich-text fallback, accessibility labels, copy/export output, notifications, search indexing, or unsupported-client rendering. Fallbacks substitute `[REDACTED]` (or an equivalent localized marker).

**Serialization rule:** redaction is applied before the canonical event body, rich-text fallback, effect ranges, search text, or any secondary representation is serialized. The original redacted substring is never included anywhere in the transmitted event payload. Effect ranges are calculated against the already-redacted canonical body.

**`underline` emits `<u>`** in rich-text output. Clients that do not support underline may strip it; the plaintext fallback remains readable.

---

## 5. Animations

**Last one wins** if multiple are given. Animations must be designed for both touch and pointer-based interfaces; no effect may depend on hover to make sense or become usable.

| Modifier | Effect |
|---|---|
| `shake` | short horizontal jitter, as if the text itself is trembling |
| `wave` | letters ripple vertically in sequence across the text |
| `pulse` | a soft heartbeat-like motion: the text briefly compresses, expands past its resting size, and settles back with spring easing; avoid opacity cycling so it does not read like `glow` |
| `glow` | light blooms outward from the glyphs and fades back while the text itself remains geometrically still |
| `typewriter` | reveals one character at a time |
| `sparkle` | brief particle sparkle around the text |
| `glitch` | the glyphs themselves momentarily corrupt: individual characters flicker, jump by a few pixels, or briefly substitute glitch symbols before snapping back to the original text; do not use an RGB-channel overlay as the primary effect |
| `scatter` | characters burst slightly away from their normal positions in different directions, then spring back into their exact original layout |
| `flip` | renders the text as genuinely upside-down text rather than merely rotating the normal line box; glyph orientation and baseline should read naturally as upside-down writing, with implementation chosen per platform to preserve legibility |
| `barrel` | the text gives a quick upward bounce, tucks into a forward somersault/front flip, then lands and settles; characters should move as a coordinated word or span rather than continuously spinning in place |

### Playback classes

Sigil distinguishes two animation classes:

- **Content animations** — authored or semantically meaningful motion such as `shake`, `wave`, `glitch`, `scatter`, `barrel`, dice rolls, coin flips, and chart reveal animations. These follow the replay rules below.
- **UI transitions** — incidental interface feedback such as checkbox ticks, progress fills, timer-expiry transitions, poll-bar growth, card entrance motion, and weather decoration. These are not exposed through **Replay animation** and may run only when their related UI state changes.

### Playback and replay

Content animations are **one-shot by default**, not perpetual. They must not restart merely because a message is recomposed, scrolled off-screen and back on-screen, or otherwise re-rendered by the UI.

- **New messages:** autoplay the animation once when the message is first presented as new content.
- **History:** old messages should normally render in their settled/final state rather than replaying automatically while the user scrolls through history.
- **Replay:** every message containing one or more replayable content animations exposes a **Replay animation** action. On desktop, surface it with the normal message-hover/focus actions and context menu. On mobile, expose it from the tap/long-press message actions.
- **Optional direct replay:** if an animated span has no conflicting interaction such as a link, spoiler, or selectable control, a simple tap/click on the span may replay it. This is a convenience only; the message action remains the reliable cross-platform path.
- **Replay scope:** replay only the animations in the selected message, from their initial state, without re-triggering unrelated message effects or notifications.
- **Reduce motion:** when reduced-motion is enabled, do not autoplay. The replay action may remain available, but should either use a reduced-motion variant or clearly respect the user's animation preference.

This makes animations expressive when a message arrives without turning the timeline into a permanently moving surface, while still letting the recipient intentionally experience the effect again.

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

Parsing happens in layers so structured blocks and inline modifiers do not fight over the same punctuation.

1. **Detect structured constructs from raw source first.** Recognize block openers such as `checklist::`, `poll::`, `chart::`, `diagram::`, `table::`, `recipe::`, `math::block`, and `art::` before Markdown processing.
2. Inside a recognized structured block, the active construct defines which row/item markers are valid. Ordinary item rows use `- `; constructs may define additional explicit markers such as checklist `-x- ` and `-r- `. Because the parser has already classified the enclosing block before Markdown runs, these markers are interpreted as part of the SigilText block grammar rather than ordinary Markdown list syntax.
3. **Inline modifiers inside structured item lines are line-scoped and do not take their own unescaped `;`.** They end automatically at the item boundary. A literal semicolon inside item content must be escaped as `\;`.
4. A structured block terminates when its **final block line** ends with the block's only unescaped `;`. Because item-level inline modifiers are line-scoped, that semicolon cannot be mistaken for an inner modifier terminator.
5. After structural boundaries are known, run Markdown through the CommonMark-compatible parser on ordinary text and on the content portions of structured items.
6. Apply spoiler and inline-modifier passes **only to `Text` events** — never `Code` or `CodeBlock`. This makes `` `red::foo;` `` literal automatically.
7. Inline-modifier token classification applies only after the parser has decided that a segment is an inline modifier expression: animation set → animation; color set → color; hyphenated all-colors → gradient; text-style set → style; size set → size; otherwise render literally.
8. Unterminated inline span (`red::text` with no `;`) → style to end of the current logical line/item.
9. Empty inline content (`red::;`) → no-op.
10. **Single colons inside a segment are legal** — `remind::9:30am::text;` splits on `::` only. Structured constructs define their own valid segment counts and option positions.
11. An unterminated structured block (missing its final unescaped `;`) remains literal/incomplete and does **not** create an interactive structured event. In the composer, show a non-blocking incomplete-construct hint and offer completion.
12. Sanitize rich-text output before rendering received messages.

This keeps block termination unambiguous while preserving fast inline syntax inside checklist items, poll options, chart labels, and other structured content.


### Structured constructs

Structured constructs share the `keyword::...` opener family and an unescaped final `;` terminator, but each construct defines the body grammar that best fits its content.

Row-oriented constructs such as checklists, polls, charts, tables, and diagrams use construct-defined item/row markers. Ordinary rows use `- `, while a construct may define additional markers such as checklist `-x- ` and `-r- `. Other constructs may use specialized bodies, such as sectioned recipe content, literal ASCII art, or block math.

Domain-specific operators such as `=`, `|`, `->`, `-->`, and `:` are defined only by the constructs that use them. They are not global SigilText operators.

Composer completion and graphical builders should expose the expected body shape for the active construct so users do not need to memorize each construct's syntax.

---

## 8. Event representation

Standard fields carry Markdown-derived HTML plus plaintext so generic clients render readable content. Sigil-specific styling is represented as resolved spans rather than baking every possible modifier into one fixed schema.

```json
{
  "type": "text",
  "body": "text lorem ipsum",
  "format": "text/html",
  "formatted_body": "text lorem ipsum",
  "sigil": {
    "text_spans": [
      {
        "start": 0,
        "end": 16,
        "effects": [
          {"kind": "color", "value": {"type": "gradient", "stops": ["red1", "blue3"]}},
          {"kind": "animation", "value": "shake"}
        ]
      }
    ]
  }
}
```

```text
TextSpan
- start: integer
- end: integer
- effects: ordered list of resolved effect descriptors

`start` and `end` index **Unicode extended grapheme clusters** in the canonical normalized plaintext body, using half-open ranges `[start, end)`. Before effect ranges are calculated, text is normalized to Unicode NFC. Every client must apply the same normalization and grapheme-segmentation rules so emoji, combining marks, flags, skin-tone sequences, and other multi-codepoint user-perceived characters are never split by an effect boundary.

EffectDescriptor
- kind: color | background | emphasis | decoration | size | animation | reveal | monospace | ...
- value: kind-specific resolved value
```

The parser resolves conflicts before serialization. Effects from different categories coexist. If multiple values from the same exclusive category were authored, only the winning resolved value needs to be stored.

This representation intentionally stays extensible: adding a future modifier generally adds a new effect kind/value, not a new top-level protocol field or a parallel rendering pipeline.

---

## 9. Contacts and identity references

SigilText defines how identities are referenced inside authored and serialized content. It does not define account hosting, server topology, migration, federation, or transport architecture.

### `@` mentions and `@::` contact cards

Typing bare `@` opens mention autocomplete.

A normal mention may be authored using either:

- a friendly local/display form such as `@user`, when that name resolves unambiguously in the current context; or
- a full address such as `@user:server.example`, when the server-qualified form is needed for lookup or disambiguation.

Typing `@::` opens a contact picker and inserts a shareable contact-card construct rather than an inline mention.

### Friendly names and full addresses

The user-facing form should prefer the shortest unambiguous identity reference.

- `@user` is the preferred display form when it resolves uniquely.
- `@user:server.example` is the explicit full address and may be shown when first contacting someone, resolving ambiguity, or exposing identity details.
- The server component is part of the full address. SigilText does not reinterpret it as a separate routing hint or define how the server reaches the user.
- A client may visually collapse a known full address to `@user` after resolution, but must retain the stable resolved identity reference in the serialized event.

### Resolution and disambiguation

Mention resolution is a client/composer concern, but the serialized result must be stable.

- If a friendly `@user` maps to exactly one known identity in context, it may resolve directly.
- If more than one identity could match, the composer must require disambiguation before binding the mention.
- Disambiguation UI may show display name, avatar, full `@user:server` address, verification/trust state, and other local context useful to the user.
- Once selected, the mention is serialized against a stable identity reference so later display-name or friendly-name changes do not retarget old messages.
- If a client cannot resolve an authored friendly name before send, it must not guess which identity was intended.

The exact stable identity identifier is owned by Sigil's identity/protocol layer rather than SigilText. SigilText only requires that the serialized reference be stable and unambiguous.

### Serialized mention representation

Plaintext fallback should remain human-readable, while Sigil metadata carries the stable resolved identity reference.

Representative shape:

```json
{
  "type": "text",
  "body": "Hey @user",
  "sigil": {
    "mentions": [
      {
        "start": 4,
        "end": 9,
        "user_id": "<stable-user-id>",
        "display": "@user",
        "address": "@user:server.example"
      }
    ]
  }
}
```

`user_id` is the stable identity reference supplied by the surrounding protocol. `display` is the authored/friendly presentation. `address` is the full address when known and useful for reconstruction or identity details.

Mention ranges use the same canonical text normalization and indexing rules defined for other SigilText spans.

### Contact-card event shape

A contact card shares an identity as structured content rather than merely mentioning it.

Representative shape:

```json
{
  "type": "text",
  "body": "Contact: Example User (@user:server.example)",
  "sigil": {
    "contact": {
      "user_id": "<stable-user-id>",
      "handle": "@user",
      "address": "@user:server.example",
      "display_name": "Example User",
      "avatar_url": "https://example.org/avatar/abc123"
    }
  }
}
```

Fields:

- `user_id` — stable identity reference supplied by the identity/protocol layer
- `handle` — friendly short form, when available
- `address` — full `@user:server` address
- `display_name` — current human-readable display name
- `avatar_url` — optional avatar reference

The plaintext `body` must remain useful in clients that do not understand Sigil contact cards.

### Contact-card presentation

Render a contact card with:

- avatar;
- display name;
- friendly handle when available;
- full `@user:server` address in details or directly when disambiguation matters;
- one primary action such as **Message** or **Add contact**.

Stable internal identifiers should remain behind an identity/details view rather than dominating the normal card.

On mobile, the default card should stay compact. Verification, copy-address, copy-identity, and other secondary actions belong in expanded details or the context menu.

### vCard representation

Receiving a `.vcf` may render through the same contact-card presentation when the file contains recognizable contact data.

When exporting a Sigil identity to vCard:

- place the display name and other conventional contact fields in standard vCard properties;
- include the full `@user:server` address in a visible/custom field suitable for round-trip recovery;
- include the stable Sigil identity reference in a Sigil-specific extension field;
- optionally duplicate critical Sigil identity information into `NOTE` so it survives address-book software that strips unknown extension fields.

vCard is a representation/export format only. It does not define Sigil identity semantics or server architecture.

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

### Power-user composer behavior

The composer should behave like a lightweight IDE for SigilText:

- **Prefix completion:** typing `rem` can suggest `remind::`; typing `chart::` can suggest chart types.
- **Ghost text:** show the expected next segment or syntax shape without forcing a popup.
- **Keyboard-first control:** Up/Down navigates suggestions; Tab or Enter accepts; Escape dismisses.
- **Context-aware hints:** after a construct opener, show only options valid at that position.
- **Live parse confirmation:** reminders, timers, dates, and other interpreted input show the resolved value before send.
- **Forgiving incomplete input:** when the grammar already defines an obvious end-of-line interpretation, do not punish a missing final `;`.
- **Re-entry editing:** placing the cursor inside formatted content should expose the underlying effect(s) in a compact, editable form rather than making the user reconstruct syntax manually.
- **Contextual errors:** invalid segments should explain what was expected and offer the relevant `help::` topic.

The intended learning curve is: context menus for beginners, autocomplete that teaches the syntax for intermediate users, and raw SigilText at full typing speed for power users. UI actions and typed syntax should resolve to the same underlying constructs rather than becoming separate formatting systems.


### Graphical creation rule

**Every sendable SigilText construct must have a graphical creation path.** Raw syntax is a fast path, never the only path. Graphical creation and typed SigilText must resolve to the same canonical event/construct rather than producing separate implementations.

Simple text styling remains available directly from selection/context formatting controls. Rich or structured constructs live in the composer **Attachment / Create** panel.

### Categorized attachment panel

The attachment panel must not present every construct as a flat grid. The first level shows a small set of categories; opening one category reveals its tools. Search sits at the top and can jump directly to any construct by name, synonym, or example intent.

Suggested top-level categories:

| Category | Graphical creation paths |
|---|---|
| **Plan & Organize** | Checklist, recurring checklist, task list, note, reminder, timer, countdown / elapsed-time |
| **Ask & Decide** | Poll, dice roll, random pick, number pick, coin flip, rating |
| **Data & Visualize** | Chart, diagram, table, progress, calculation, conversion, math |
| **Share & Encode** | Contact card, QR code, quote, keyboard shortcut, color swatch, ASCII art |
| **Reference & Services** | Translation, definition, weather, recipe |
| **Help** | Interactive Help browser / cheat sheets |

Formatting features such as bold, italic, underline, strike, monospace, colors, gradients, highlighting, spoilers/reveal effects, sizes, and animations live in the formatting/context UI rather than occupying attachment-panel slots.

#### Panel behavior

- **Search first:** typing `weather`, `dice`, `translate`, `flowchart`, `convert`, etc. filters across all categories.
- **Recent/Favorites:** optionally surface a small row of recently used or user-pinned tools above the categories without duplicating the full catalog.
- **One editor model:** selecting a graphical tool opens a focused editor/builder with live preview and the same validation rules as typed SigilText.
- **Round-trip editing:** a structured message created graphically can be reopened graphically; a message authored with syntax opens the same editor populated from the parsed event.
- **Mobile:** categories open as drill-down sheets/pages with large targets; do not render a dense desktop-style mega-menu.
- **Desktop:** categories may open as a two-pane panel (categories left, tools/results right) with full keyboard navigation.
- **No dead ends:** if a construct has advanced options not shown in the first graphical screen, expose them under **More options** rather than requiring the user to switch to raw syntax.
- **Help integration:** every builder has a compact **Syntax** / **Help** affordance that can show the equivalent SigilText for learning, without forcing syntax into the main workflow.

This creates three equivalent entry paths: **format/context UI**, **categorized attachment builders**, and **typed SigilText**.

---

# PART II — STRUCTURED CONTENT & UTILITIES

Everything below extends the grammar above. Part II contains both additional single-line constructs and structured multi-line blocks using the same `keyword::...` family. **Structured block constructs**, where used, span multiple lines and terminate when the final block line ends with an unescaped `;`.

**All constructs work in any room** — do not gate structured content behind a room type. A checklist in a chat and a checklist in a notes room behave identically.

---

## 11. Checklists

```
checklist::Things
- red::shake::URGENT ITEM
- Item 2
- Item 3;
```

- Title is everything on the same line as `checklist::`
- Items begin on subsequent lines with `- `.
- `-x- ` marks an item pre-checked.
- Inline modifiers inside an item are line-scoped and **must not use their own unescaped `;`**: `- red::shake::URGENT ITEM`
- The final item's trailing unescaped `;` terminates the checklist block.
- A literal semicolon inside an item must be escaped as `\;`.
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

**Reset timing:** 12:01 AM in the checklist's **authoritative recurrence timezone** on the target day. That timezone is captured when the recurring list is created and stored with the event/state so every participant computes the same reset instant.

Clients display the recurrence in the viewer's locale/timezone, but the reset calculation always uses the stored recurrence timezone unless an authorized editor explicitly changes the recurrence settings.

**Recurrence authority:** the checklist creator is the initial recurrence owner. The event/state stores the canonical creator identity plus a recurrence-permission policy. By default, only the creator may change interval, recurrence timezone, or reset policy. A room-level permission model may delegate that capability to additional members, but every accepted recurrence edit must be attributable to a canonical identity and produce one authoritative new recurrence state for all clients.

- `weekly` — same weekday as creation
- `monthly` — same day-of-month as creation
- `yearly` — same month and day as creation. A Feb 29 yearly recurrence fires Feb 28 in non-leap years, then returns to Feb 29 in leap years.

**Month-end clamping:** clamp to the **last day of the target month**, not a fixed value. Created on the 31st → Jan 31, Feb 28 (29 in leap years), Mar 31. It must snap back to the intended day whenever the month allows.

**DST:** target 12:01 AM in the stored authoritative recurrence timezone. If a transition removes that local time, shift forward by the gap; if it repeats, use the earlier instant. Every client uses the stored timezone-data version and the same resolution rule.

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
- Render attribution compactly (checkmark + small avatar), full detail on tap

### Checklist presentation

Render checklists as compact interactive cards rather than plain bullet lists:

- Header shows the title, completed/total count, and a thin progress indicator.
- Checked items visually recede but remain readable; do not remove them immediately from standard lists.
- On mobile, the entire checkbox row is a generous touch target. On desktop, support checkbox click plus keyboard focus/Space.
- Long lists initially show a useful viewport with **Show all** rather than creating a message bubble several screens tall.
- Reordering/editing opens a focused list editor; ordinary timeline viewing stays compact.
- Recurring lists show the recurrence rule in subdued metadata (`Weekly · resets Friday`).
- Task lists distinguish irreversible completion from ordinary checklists with a small task/status affordance rather than relying on color alone.
- Progress changes should animate subtly (check stroke/progress fill) but never loop.


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

Both optional — `poll::Question` with items is valid. Poll option segments are **order-independent** before the question, so `poll::closed::multi2::Question` and `poll::multi2::closed::Question` are equivalent. Repeating the same option category uses last-one-wins and should trigger a composer warning.

Emit Sigil's canonical poll event shape so the syntax path and graphical poll builder produce exactly the same underlying event. `open` and `closed` control whether results are visible before voting.

### Poll presentation

- Render each option as a full-width selectable row with a radio/check affordance appropriate to single or multi-select mode.
- After voting, animate result bars once from zero to their stored percentage/count; do not continually animate them.
- Show both percentage and vote count where space permits. On narrow mobile layouts, keep the percentage visible and move detailed counts into tap-expanded metadata.
- `closed` polls hide result bars until the viewer is entitled to see them; do not leak relative widths through placeholders or accessibility labels.
- Show the viewer's selected choices persistently.
- Long option labels wrap naturally; many-option polls collapse after a sensible threshold with **Show all options**.
- Closed/ended polls become read-only result cards with the winning option(s) visually emphasized without relying solely on color.


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

Reminders in a shared room notify **all members**. Each client schedules its own platform-local notification (Android AlarmManager, iOS local notifications, desktop equivalent) after receiving the reminder event.

Fired reminders remain in the Notes tab as history.

### Reminder presentation

- Render as a compact reminder card with a clock/bell icon, reminder text, and the resolved local date/time.
- Show relative context such as `Tomorrow · 9:30 AM` when helpful, but always keep the absolute date available on tap/expand.
- Upcoming reminders may show a quiet relative countdown (`in 3h`) that updates at coarse intervals; do not create a constantly ticking seconds display.
- Fired reminders transition to a subdued **Completed/Fired** state rather than disappearing.
- Tapping the time opens details and notification state; long-press/context menu offers actions such as copy, add to calendar, or disable local notification where supported.


---

## 14. Notes

```
note::Be happy!;
```

A single-line message flagged for the Notes tab. Users should also be able to **promote an existing message to a note via UI** — likely more common than the syntax.

### Note presentation

Notes should look intentionally different from ordinary chat without becoming visually loud: a small note glyph/accent, slightly distinct card treatment, and a clear **Note** label in expanded details. Promoted notes preserve the original message content and authorship rather than duplicating editable text.


---

## 15. Timers

```
timer::1 hour 45 min;
```

- Store both an absolute `started_at` and absolute `ends_at`. `ends_at` is authoritative for synchronization; `started_at` defines original duration/progress and prevents clients from deriving progress from receive time.
- Accept flexible input: `1 hour 45 min`, `1h45m`, `90 minutes`, `1.5 hours`, `45s`
- Duration must be finite and strictly greater than zero. `started_at` must be earlier than `ends_at`; zero-length, negative, NaN, or overflowed timers are invalid and remain literal with a composer warning.
- **Confirm the parse in the composer** — show `1:45:00` before send
- Renders as a live countdown; transitions to an **ended-state bubble** on expiry, same pattern as a closed poll
- Each client schedules a local notification for `ends_at`
- **Timers do not appear in the Notes tab**

### Timer presentation

- Primary visual is a large, glanceable remaining time with a progress ring or horizontal progress track derived from `(now - started_at) / (ends_at - started_at)` and clamped to `0–100%`.
- For durations over an hour, avoid needless seconds by default; reveal finer precision near the end or on interaction.
- The ring/track moves continuously only while visible. Off-screen timers should update from absolute time when brought back into view, not simulate missed frames.
- On expiry, perform one short completion animation and settle into an **Ended** card showing when it ended.
- Tapping the timer opens details such as start time, end time, and notification state.
- Reduced-motion mode updates the numeric value/progress without sweeping or pulsing effects.


---

## 16. Notes tab

A per-room tab alongside Pinned. **Computed from the timeline** rather than global room state, and **not auto-pinned**. Notes should remain a derived view of message events rather than modifying shared pin state.

**Contents:** notes, checklists, reminders. Not timers, not polls.

**Sections:**
- **Active** — open checklists, upcoming reminders
- **Past** — fired reminders, completed lists. Collapsed by default.

Reminder history is retained deliberately. Without sectioning, a year of fired reminders makes the tab useless for finding current items.

### Notes-tab presentation

Use a searchable, filterable list rather than reproducing timeline bubbles verbatim. Each row should show type icon, title/first line, relevant status/date, and source-room position. Filters for **All / Notes / Lists / Reminders** become useful once a room accumulates many items.


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

Validation:
- `NaN`, infinities, malformed numerics, and empty values are invalid.
- Pie/donut values must be finite and non-negative; all-zero pie/donut data is invalid because it cannot be normalized meaningfully.
- Bar/line/area/scatter values may be negative when the chart type can represent them.
- Scatter rows require finite numeric x and y values.


**Row split rule:** the **last unescaped `=` outside code spans** separates the label from the value. Earlier literal equals signs may be escaped as `\=` when needed. A row with no valid value separator is malformed and remains literal with a composer hint.

**Anything with axes and values belongs under `chart::`.** Adding a new visualization should be a renderer change, not a grammar change.

### Chart presentation and interaction

Charts render as responsive timeline cards with the title above the visualization and a compact legend only when needed.

- **Pie / donut:** tap or click a slice to reveal label, raw value, and percentage. Prefer direct labels for very small category counts; otherwise use a legend.
- **Bar:** tapping/hovering a bar reveals the exact value. Horizontal bars are preferred automatically when category labels are long or numerous.
- **Line / area:** show a touch-friendly crosshair/nearest-point tooltip. Do not require precise pointer hover; dragging across the plot should scrub values on mobile.
- **Scatter:** tap selects the nearest point and pins its values until dismissed.
- Animate a chart in once when newly received (bars grow, line draws, pie settles), following global reduced-motion/replay rules. Replay is visual only and does not change data.
- On narrow screens, axes and labels simplify before data is removed. Rotate labels only as a last resort.
- Large datasets are downsampled for the inline preview while preserving extrema and overall shape; **Open chart** presents the full interactive dataset.
- The expanded chart viewer supports pinch/scroll zoom where meaningful, pan, legend toggles, and copy/export of the underlying data when allowed.
- Never encode a series only by color: use labels, patterns/markers, or direct selection cues for accessibility.


---

## 18. Diagrams

SigilText defines its own diagram syntax rather than depending on Mermaid-specific source syntax.

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
- Client -> Server: send message
- Server -> Media Service: create room
- Media Service --> Client: room ready;
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
- Lead -> Member A
- Lead -> Member B
- Member A -> Member C;
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

Across diagram types, node identity is based on the normalized node label unless a future explicit node-id syntax is introduced. Reusing the exact same normalized node label refers to the same node rather than creating an ambiguous duplicate. Empty node names and malformed edges are invalid.

**Anything with nodes and relationships belongs under `diagram::`.** Relationship-based diagram types share the `->` / `-->` edge family where applicable; timeline uses its own date/value row syntax with `=`. Layout and validation rules then vary by diagram type. A simple layered (Sugiyama-style) approach is adequate for chat-sized relationship diagrams — don't over-engineer.

### Diagram presentation and interaction

- Inline diagrams render to fit the bubble width with a minimum readable text size. If fitting would make labels illegible, show a cropped/overview preview with **Open diagram** instead of shrinking indefinitely.
- Expanded view supports pinch-to-zoom on touch, wheel/trackpad zoom on desktop, pan, reset-to-fit, and tap/click-to-focus nodes.
- Focusing a node highlights its directly connected edges/nodes and dims unrelated structure.
- Flow/state diagrams may animate edge traversal only as a one-shot presentation effect; the static topology must remain fully understandable without animation.
- Sequence diagrams horizontally scroll when participants exceed the available width rather than crushing lifelines together.
- Timelines horizontally scroll or compress date spacing intelligently while keeping event labels legible.
- Mind maps may collapse branches interactively in expanded view.
- Org charts should expose a compact subtree focus action for large hierarchies.
- Complexity limits are mandatory. Above a node/edge threshold, inline rendering becomes a lightweight overview card and full rendering is deferred until opened.


---

## 19. Tables

Far friendlier than hand-aligning Markdown pipes.

```
table::Name | Role
- Person A | Designer
- Person B | Engineer;
```

- Title line defines column headers, separated by `|`
- Each row uses the same separator
- Rows with fewer cells pad missing trailing cells with empty values. Rows with more cells than the header count are malformed: keep the source visible in the composer, show a clear warning, and do not silently truncate data.
- Table headers must contain at least one non-empty column name. Duplicate header labels are allowed but should be disambiguated in accessibility/export metadata by column position.
- A completely empty row is invalid; explicit empty cells inside an otherwise non-empty row are allowed.
- **Emit a real HTML `<table>`** in the rich-text fallback so generic clients can render it when supported

### Table presentation

- Use a sticky header row in expanded view.
- Inline cards show a limited number of rows and columns before offering **Open table**.
- On mobile, preserve column structure with horizontal scrolling; never stack cells into ambiguous key/value blobs unless the table has exactly two columns and the user chooses that view.
- Long cell text wraps to a bounded number of lines inline and expands on tap.
- Numeric columns align consistently and may be sorted in expanded view; sorting is a local viewing operation and does not mutate the shared event.
- Support copy cell, copy row, and copy entire table from context actions.


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
- `ingredients:` and `steps:` are section markers (single colon, own line). Their entries use ordinary `- ` item lines inside the recipe block.
- Section order is fixed: optional metadata lines first, then `ingredients:`, then `steps:`. Missing `ingredients:` or `steps:` makes the recipe malformed. Repeating a section marker is invalid rather than merged implicitly.
- Renders as a card: metadata header, ingredient list, numbered steps
- **Ingredients individually checkable while cooking** — same interaction as a standard checklist, not persisted

### Recipe presentation and cooking mode

The timeline card shows title, servings, total time, a short ingredient preview, and the first few steps. **Open recipe** enters a dedicated cooking view:

- Ingredients are individually checkable locally and checked ingredients visually recede.
- Steps become large, high-contrast cards navigable with swipe/Next/Previous.
- Keep the current step and ingredient list one tap away rather than forcing repeated scrolling.
- Optional **Keep screen awake** can be offered while cooking, controlled locally and clearly indicated.
- If servings are present, allow local serving scaling and recalculate numeric ingredient quantities where safely parseable; preserve the original recipe values as authoritative.
- Embedded timers in step text may offer a local **Start timer** action, but do not infer/send new timers automatically.


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

Emit a Sigil math span/card plus a plaintext fallback containing the original expression so clients without math rendering still show readable content.

Math rendering requires a renderer capable of producing readable inline and block notation; parsing support alone is insufficient. The specific rendering library is a platform choice and is not part of the SigilText language specification.

### Math presentation

Inline math should align naturally with surrounding text. Block math centers in a horizontally scrollable card if it exceeds the bubble width. Tap/click opens an expanded renderer with **Copy expression** and **Copy rendered text** actions. Never scale equations down until symbols become unreadable.


---

## 22. Countdowns

```
countdown::2027-07-05::Wedding;
ago::2019-03-14::Project started;
```

- `countdown::` — days remaining until a future date, live-updating
- `ago::` — elapsed time since a past date
- Both use the same date parsing and compose-time resolution as `remind::`
- Distinct from `timer::`, which is minutes-scale and notifies

### Countdown / elapsed-time presentation

- Render the principal value prominently (`143 days`, `6 years`) with the label beneath or beside it.
- Choose granularity appropriate to the distance: days for distant countdowns, hours/minutes only when near enough to matter.
- Do not continuously animate. Update at the natural unit boundary.
- When a countdown reaches zero, transition once into a **Today** / reached state. After the target date passes, automatically render elapsed-time semantics (`1 day ago`, `2 days ago`, …) while preserving the original construct/event. Never show negative countdown numbers.
- Tap reveals the exact target/origin timestamp and timezone interpretation.


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
- Computed locally with no service dependency. Calculation semantics, unit constants, rounding mode, and display precision are defined centrally so every client produces the same canonical result. Clients may reveal additional precision locally, but the canonical displayed result must match.

### Calculation / conversion presentation

- Render as a compact two-line result chip: original expression/input first, emphasized result second.
- Preserve enough significant digits for correctness but avoid noisy floating-point tails; expanded details can show full precision.
- Tapping a conversion may offer **Swap units** locally and copy either side without modifying the sent event.
- Invalid expressions stay visibly literal with a concise inline parse hint in the composer rather than producing an error card after send.


---

## 24. Randomizers

Randomizers should feel like visible events in the timeline, not instant plaintext answers. The result is chosen once when the message is created, stored in the event, and then animated locally for presentation. Replaying the animation must never generate a new result.

```
roll::2d6;
roll::2d6, 1d20, 2d12;
pick::pizza, tacos, thai;
pick::number::1-100;
pick::food;
pick::flip;
```

### `roll::`

Standard dice notation. Multiple groups are comma-separated.

`roll::2d6;` should render **two fully 3D d6 dice** in the message bubble. When the message first appears, both dice tumble through three-dimensional space, rotate on multiple axes, bounce, collide lightly with the visual floor/bounds, and settle on the exact stored faces.

The dice must read as actual volumetric objects rather than flat squares with rotating pips. Use perspective, lighting/shading, visible edges, and face transitions so a d6 looks like a cube and polyhedral dice look like their real corresponding forms.

Where practical, supported die types should render as recognizable 3D polyhedra:

- `d4` — tetrahedron
- `d6` — cube
- `d8` — octahedron
- `d10` — pentagonal trapezohedron
- `d12` — dodecahedron
- `d20` — icosahedron

Other die sizes may use a stylized fallback if a faithful polyhedron would be impractical, but the UI should still make the die type obvious.

Rules:

- Choose the random result once at send time and store each die face/result explicitly.
- Animation is presentation only; replaying it shows the same stored result.
- Dice should tumble, translate, bounce, and settle naturally rather than merely spinning a flat icon.
- Multiple dice may have small staggered launch velocities and rotations so rolls feel physical without becoming chaotic.
- A subtle shared surface shadow can sell depth, but animation must remain readable in both light and dark themes.
- Respect the global one-shot animation/replay rules and reduced-motion preference.
- Very large valid rolls that remain within configured hard limits should collapse to a compact representation instead of rendering hundreds of animated dice. Show a small representative group plus numeric results. Enforce configurable hard limits on dice count and sides before allocation/rendering; notation exceeding a hard limit is invalid rather than compacted.
- After settling, show each group's subtotal and a combined total when multiple groups are present.

#### Randomizer trust model

Randomizers are **casual client-generated randomness by default**, not cryptographic proof of fairness. The sender's client generates the result once before send and includes the resolved result in the event.

- The UI must not imply that ordinary `roll::` or `pick::` results are independently verifiable.
- For casual chat, this model is sufficient and keeps the feature fast/offline-friendly.
- If Sigil later supports high-trust games, wagers, moderation decisions, or other fairness-sensitive uses, that must use a separate verifiable-randomness protocol (for example commit/reveal or server-assisted randomness) with visibly distinct UX. Do not silently upgrade ordinary `roll::` into a trust claim it cannot prove.


### `pick::`

`pick::` chooses one item from a set or range and should visually communicate that a random selection is happening.

| Form | Behavior |
|---|---|
| `pick::a, b, c;` | Chooses from explicit options |
| `pick::number::1-100;` | Random integer in range |
| `pick::<category>;` | Chooses from a built-in or user-defined category |
| `pick::flip;` | Flips a 3D coin and returns Heads or Tails |

Presentation:

- **Explicit options:** animate through candidate chips/cards like a fast roulette or slot-strip, decelerate, then land on the stored choice.
- **Number range:** rapidly cycle representative numbers, slow down, then settle on the stored integer.
- **Category:** briefly cycle through several category entries before settling on the stored selection.
- **Coin flip:** render a **real 3D coin** that launches upward, rotates around its horizontal axis through several full revolutions, shows both distinct faces during flight, then falls and lands on the stored side. The coin should have visible thickness, rim shading, perspective, lighting, and a brief landing bounce or wobble so it reads as a physical object rather than a flat circle rotating in 2D.
- Replaying the animation always lands on the original stored result.

#### Suggested built-in categories

Built-in categories should provide immediately useful defaults while remaining intentionally small and maintainable. Suggested defaults:

- `food` — pizza, tacos, burgers, sushi, pasta, curry, sandwiches, barbecue, salad, noodles, breakfast, seafood
- `movie` — action, comedy, drama, thriller, horror, sci-fi, fantasy, animation, documentary, mystery, romance, adventure
- `book` — fiction, mystery, sci-fi, fantasy, history, biography, science, philosophy, horror, romance, thriller, graphic novel
- `activity` — walk, movie, game, cook, read, exercise, café, museum, drive, picnic, photography, music
- `chore` — dishes, laundry, vacuum, trash, bathroom, kitchen, dusting, organizing, groceries, yard work
- `meal` — breakfast, brunch, lunch, dinner, snack, dessert
- `color` — red, orange, yellow, green, cyan, blue, purple, pink, gray
- `direction` — north, south, east, west
- `yesno` — yes, no
- `flip` — Heads, Tails

These lists are defaults, not protocol constants. A deployment may localize or extend them, and users should be able to create custom categories in settings.

For user-defined categories, support reusable sets such as restaurants, games, chores, names, destinations, or team members. Exact duplicate entries are deduplicated by default so accidental duplication does not silently weight an option. Duplicate entries have no weighting semantics.

For category-based picks, store both the category identifier and the chosen value in the event. The chosen value is authoritative even if the category list later changes.

`pick::flip;` is the canonical coin-flip syntax. Do not use bare `flip::` for randomness because `flip::` is already the upside-down text animation.

### Randomizer timeline behavior

- The chosen result should be visible after the animation settles without requiring expansion.
- Tapping/clicking the final result may expose a **Replay animation** action, but never a reroll action unless the sender explicitly creates a new randomizer event.
- Reduced-motion mode should skip tumbling/roulette motion and transition directly to the stored result with a restrained fade/scale reveal.
- If the app is resumed after the original animation should have completed, render the settled state rather than trying to catch up mid-animation.
- Randomizer results should include accessible text labels so screen readers announce the final value without depending on the animation.


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
| `quote::Author::Example quotation;` | Formatted pull-quote with attribution |

`quote::` optionally takes a source: `quote::Author::Source::text;`. Distinct from Markdown `>`, which is for quoting conversation.

### Display-helper presentation

- `swatch::` renders a generous color sample plus normalized text value; tapping copies the value.
- `kbd::` uses tactile key-cap shapes but remains selectable/readable text for accessibility.
- `rate::` renders filled/empty stars plus a textual value such as `4/5`. Ratings whose numerator exceeds the denominator, or whose denominator is zero/non-positive, are invalid and remain literal with a composer warning; do not silently clamp.
- `progress::` renders a labelled progress track and numeric percent; numeric input is clamped to `0–100` because progress has an intrinsic bounded domain. Changes should not imply live progress unless the event itself is updated.
- `quote::` uses restrained typography and attribution, with long quotes collapsed behind **Show more**.


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

### ASCII-art presentation

Use a dedicated monospace surface with horizontal drag/scroll affordance on mobile. Double-tap/click may fit-to-width only when doing so preserves character-cell proportions; otherwise retain 1:1 layout. Provide **Copy art** as a single action.


---

## 27. QR codes

```
qr::https://example.org;
qr::wifi::MyNetwork::password123;
qr::contact::@user:example.org;
qr::text::anything at all;
```

`qr::contact::` accepts a friendly handle when already resolved or a full account address such as `@user:server.example` when explicit addressing or disambiguation is needed.

Typed forms exist because the raw wifi payload (`WIFI:S:name;T:WPA;P:pass;;`) is full of semicolons and would need escaping throughout — construct it internally instead.

- A QR-generation library generates the QR module grid
- **Medium error correction** default — far more tolerant of poor scanning conditions than Low
- **Preserve the quiet zone.** QR requires white margin; don't crop tight to the bubble
- **Force a light background even in dark theme.** Most scanners expect dark modules on light; white-on-dark frequently fails. This is the single most common QR rendering mistake.

### QR presentation and actions

Render the QR in a clean light tile with a short human-readable payload summary beneath it. The summary is critical because users should know what they are about to scan/share.

Typed QR forms expose context-aware actions:

- URL → **Open link**, **Copy link**
- Wi-Fi → **Join network** where the platform allows it, plus **Copy network name**; never reveal a password by default in the timeline
- Contact → **View/Add contact**
- Text → **Copy text**

Tapping the QR opens a larger high-contrast version suitable for scanning from another device. Very dense QR payloads should warn when reliable scanning is likely to suffer and offer copy/share of the underlying payload.


---

## 28. Service-backed constructs

These constructs may use an external provider by default, but **every service-backed feature must also support a self-hosted endpoint**. Provider choice belongs in settings, and service failures must degrade quickly to a clear literal/error state rather than hanging the composer or timeline.

**Resolved-result rule:** service-backed content is resolved when the message is created and the canonical result needed for consistent rendering is stored with the event. Recipients do not independently rerun the query merely to render the same message. A deliberate **Refresh** action, where appropriate, creates/attaches a new snapshot rather than silently rewriting history.

### Translation

```
translate::es::Where is the library?;
translate::auto::¿Dónde está la biblioteca?;
```

`auto` means "detect the source language, then translate to my preferred language."

**Default provider:** Google Cloud Translation when configured and permitted by the deployment's billing/usage policy.

**Self-hosted option:** LibreTranslate-compatible endpoint.

Do not assume a permanently free Google quota in the protocol or UI. Provider pricing and quotas are deployment configuration rather than SigilText semantics.

#### Translation presentation

- Render the translated text as the primary readable result with the original immediately available beneath a small **Original** disclosure or side-by-side when space allows.
- Label source and target languages (`Spanish → English`) and indicate when source language was auto-detected.
- Store original text, detected/source language, target language, translated text, provider identifier/version when available, and resolution timestamp.
- Long translations collapse intelligently but never hide which text is original versus translated.
- Actions: **Copy translation**, **Copy original**, **Swap and translate** (creates a new request), and **Show provider details**.
- The active provider should be visible before sending privacy-sensitive text if the user has not chosen a trusted/self-hosted default.

### Definitions

```
define::petrichor;
```

**Default source:** Wiktionary-derived data or API access.

**Self-hosted option:** a locally hosted Wiktionary extract/index.

Cache definitions aggressively; dictionary content changes slowly and should not require a network request every time a previously seen word is rendered.

#### Definition presentation

Render a dictionary card with:

- word/headword and pronunciation at top;
- part-of-speech badge;
- the first concise definition immediately visible;
- additional senses collapsed under **More definitions**;
- example sentence, etymology, synonyms/antonyms only when supplied by the source and useful;
- optional audio-pronunciation action when the selected provider supplies audio.

Store the resolved definition snapshot/source attribution in the event so old messages do not silently change as dictionary data evolves. Provide **Copy definition** and **Open source** actions when a source URL exists.

### Weather

```
weather::Seattle;
weather::Seattle::forecast;
```

**Default provider:** Open-Meteo-compatible public API.

**Self-hosted option:** a configurable self-hosted weather service or Open-Meteo-compatible deployment.

Resolve ambiguous place names before sending when needed, then store resolved coordinates/location metadata so every recipient sees weather for the same place rather than independently geocoding the raw label.

#### Current-weather card — `weather::<place>;`

The compact timeline card should answer "what is it like there right now?" at a glance:

- resolved place name and local observation/forecast time;
- large current temperature;
- condition icon/illustration and short condition text;
- feels-like temperature when materially different;
- precipitation probability/current precipitation;
- wind speed/direction;
- compact high/low for the day;
- optional humidity and UV in the expanded details rather than overcrowding the first view.

Use simple condition motion sparingly (for example, a one-shot drifting cloud or rain streak entrance) and respect reduced-motion. The card must remain understandable as a static image.

#### Forecast card — `weather::<place>::forecast;`

Render a horizontally scrollable multi-day forecast, defaulting to the useful range supplied by the provider (typically several days, capped by the product). Each day tile shows:

- weekday/date;
- condition icon;
- high/low;
- precipitation probability;
- concise severe/meaningful condition indicator where applicable.

The first/current day may be expanded inline with a small hourly temperature/precipitation curve. Tapping a day opens an expanded forecast with hourly temperature, precipitation chance/amount, wind, and condition changes. Touch scrubbing should reveal hourly values without requiring hover.

Weather is inherently time-sensitive, so store the snapshot timestamp prominently. A **Refresh weather** action may request a new snapshot, but must not silently mutate the old message; refreshed data should be represented as a new/updated snapshot with clear timing.

#### Weather location/privacy UX

- The composer confirms ambiguous geocoding before send (`Seattle, Washington, US`).
- Store coordinates at sensible precision plus resolved display name; do not expose unnecessary precision in the visible card.
- Provider selection and whether queries leave the user's infrastructure are visible in settings.
- Location permission is never required merely to render a received weather card.

### Shared service rules

- Provider selection is configurable per deployment.
- Self-hosted endpoints are first-class, not hidden developer-only overrides.
- Resolve provider-backed content to a canonical result when consistency between recipients matters.
- Store provider/source attribution and resolution timestamp where useful for reproducibility.
- Cache responses where appropriate.
- Time out quickly and show a clear fallback state. Composer-side resolution should be cancellable; duplicate retries must not create duplicate send events.
- Never allow service failure to block sending an otherwise valid message; offer literal-send fallback.
- Privacy-sensitive queries such as translation text and weather location should make the active provider visible in settings.
- Network-backed cards display a subtle **snapshot time** when freshness materially affects interpretation.

---

## 29. Interactive Help

```
help::
help::info;
help::colors;
help::animations;
```

**With this much surface, discoverability is the real problem.** `help::` is the built-in learning and reference surface for SigilText, available directly in the composer rather than hidden in separate documentation.

### Help browser

Typing bare `help::` opens an IDE-like inline help browser instead of immediately sending a message. It should support keyboard navigation, type-to-filter, Enter to open a category, and Escape to dismiss.

Suggested categories:

- **Info** — grammar, stacking, terminators, and a few representative examples
- **Text** — Markdown, modifiers, sizes, spoilers, marks
- **Colors** — hues, brightness levels, gradients, rainbow
- **Animations** — shake, wave, pulse, glow, and other effects
- **Lists** — checklists, recurring lists, tasks
- **Time** — reminders, timers, countdowns, elapsed-time cards
- **Utility** — calc, convert, randomizers, QR, display helpers
- **Data** — charts and tables
- **Diagrams** — flow, sequence, timeline, mind map, org, state
- **Other** — recipes, math, ASCII art, translation, definitions, weather

Typing after `help::` filters categories and topics. The browser should feel like editor completion rather than a large modal or documentation page.

### `help::info;`

`help::info;` produces a compact getting-started cheat sheet. Keep it intentionally small: explain the `modifier::content;` shape, show modifier stacking, show that the final semicolon may be omitted when formatting runs to end-of-line, and include a few representative constructs such as formatting, a timer, and a reminder. End with a hint to type `help::` for the full reference.

### Category cheat sheets

`help::<category>;` produces the cheat sheet for that category. Each sheet should contain concise syntax templates, short descriptions, and a handful of copyable examples.

Help has two uses:

1. **Read locally** — open a cheat sheet inside the composer without sending anything.
2. **Share in chat** — send the same cheat sheet as a normal Sigil message to teach another person.

A power user who already knows the category can type and terminate `help::colors;` directly. A newer user can type bare `help::`, browse interactively, then choose **Insert**, **Preview**, or **Send cheat sheet**.

### Shared help metadata

Every SigilText construct should define the metadata needed by help and completion from one shared source: canonical name, aliases, category, short description, syntax template, valid options, and examples. The help browser, autocomplete, contextual errors, and sendable cheat sheets should all consume the same metadata so documentation cannot silently drift from supported syntax.

Contextual errors should link directly into the relevant help topic. For example, an invalid poll option can show the valid forms and offer **Open Poll Help**.

### Shared structured-card UX

All structured SigilText messages should follow a common visual language so the timeline does not feel like a collection of unrelated mini-apps:

- compact card header with type/icon only when the construct is not self-evident;
- primary information visible without opening a modal;
- one obvious primary interaction, with secondary actions in context/overflow menus;
- inline height capped so one rich message cannot dominate several screens;
- **Open / Show more** for dense content;
- touch targets sized for mobile and equivalent keyboard/focus behavior on desktop;
- static, readable final state for every animation;
- loading, error, offline, stale-snapshot, and unsupported-renderer states defined for every network/heavy construct;
- copy/share/export actions operate on underlying semantic data, not screenshots, whenever possible.


---

# PART III — CROSS-CUTTING CONSTRAINTS

## Animation consistency

Animations will drift between platforms unless their behavior is parameterized centrally in the SigilText specification. Define explicit values for duration, easing curves, amplitudes, cycle counts, particle behavior, glyph displacement/substitution rules, spring behavior, and other effect parameters rather than relying on toolkit-specific named defaults. Playback semantics are part of the specification too: autoplay once for new content, no automatic restart from re-rendering, and explicit user-triggered replay.

Named easing constants differ between UI toolkits. Store explicit cubic-bezier control points so implementations stay in the same visual family.

**Accept that particles and shaders will look similar rather than identical** across platforms. Shared parameters should preserve the intended character of each effect even when rendering engines differ.

**Safeguard:** maintain a test room with one message per effect. Screenshot on each platform and compare. Drift rots silently otherwise.

## General constraints

- **Namespace consistency.** All Sigil-specific event metadata lives under one `sigil` namespace/object. Do not mix unrelated naming schemes such as `com.sigil.*`, `sigil_*`, and top-level one-off fields.

- **Respect reduce-motion.** Honor the system preference; provide a Sigil-level toggle. Animated text is hostile to motion-sensitive users.
- **Performance.** Pause animations for messages scrolled out of view. Per-character animated items across a long timeline will otherwise burn CPU.
- **Lazy rendering.** Don't parse or render heavy constructs (diagrams, QR, large art) until visible.
- **Graceful degradation.** Always populate plaintext and rich-text fallbacks so clients without Sigil-specific rendering still show readable content. `redact` is the explicit exception: its source text is replaced with `[REDACTED]` (or localized equivalent) in every fallback and secondary surface.
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
- `scatter::text;` bursts glyphs outward and returns them to the exact original layout
- `pulse::text;` changes geometry with a heartbeat/spring motion without opacity cycling
- `glow::text;` keeps glyph geometry still while the glow blooms and fades
- `glitch::text;` corrupts/displaces/substitutes glyphs temporarily; no RGB-overlay-only implementation
- `flip::text;` reads as proper upside-down text rather than a simple rotated line box
- `barrel::text;` performs bounce → forward flip → landing, without continuous in-place spinning
- New animated message autoplays once
- Scrolling an animated message out of view and back does not replay it
- Recomposition/re-render does not replay an animation
- Historical animated messages load in their settled state
- Replay action replays only the selected message's animations on desktop and mobile
- Reduced-motion mode suppresses autoplay and uses the defined reduced-motion replay behavior
- Rainbow on single char: `rainbow::A;`
- Conflicting sizes: `big3::small1::x;` (last wins)
- Out-of-range: `small4::text;` (not a modifier)
- Three-modifier stack: `underline::bold::red::text;`
- `mark::yellow::text;` and bare `mark::text;`
- `redact::secret;` fallback body contains `[REDACTED]`, never `secret`
- `redact::` content is absent from copy/export, notifications, search indexing, and accessibility labels
- Canonical transmitted body for `redact::secret;` contains `[REDACTED]` and never contains `secret`
- Effect ranges after redaction are calculated against the already-redacted canonical body
- Text-effect ranges over emoji/combining-mark grapheme clusters never split a user-perceived character
- Canonical NFC normalization produces identical effect ranges across clients

### Checklists & tasks
- Standard list, check and uncheck freely
- `-x-` pre-checked on creation
- Recurring list — manual uncheck blocked
- Weekly created Friday → resets following Friday, 12:01 AM
- Monthly created on the 31st → Feb 28, Mar 31 (clamp then snap back)
- Monthly on the 31st in a leap year → Feb 29
- Reset across a DST boundary still fires 12:01 AM in the stored recurrence timezone
- Two viewers in different timezones compute the same recurring-list reset instant from the authoritative recurrence timezone
- Unauthorized member attempts to change recurrence timezone/interval → rejected
- Authorized recurrence edit records canonical editor identity and produces one authoritative recurrence state
- `-r-` items survive reset unchecked; plain `- ` items removed
- Item with inline modifier: `- red::shake::urgent` styles to end of item line without an inner terminator
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
- Graphical and syntax-created polls render identically

### Reminders, countdowns, timers
- `remind::07/05/27 9:30am::text;` → US locale → July 5
- Same string, UK sender → May 7; recipient sees sender's resolved date
- `remind::tomorrow::text;` → defaults 9:00 AM
- `remind::tomorrow 16:45::text;` → 24h parses
- Composer shows resolved date before send
- Fired reminder appears in Notes → Past
- `countdown::` future date; `ago::` past date
- `timer::1 hour 45 min;`, `1h45m`, `90 minutes` all equivalent
- Zero-length or negative timer → invalid/literal
- `started_at >= ends_at` → invalid event/state
- Two clients show identical remaining time
- Timer progress percentage matches across clients because both use stored `started_at` and `ends_at`
- Expired timer → ended-state bubble
- Timer absent from Notes tab

### Charts, diagrams, tables, recipes
- Pie/donut values not summing to 100 → normalize
- Pie/donut negative value → invalid
- Pie/donut all-zero values → invalid
- Chart `NaN` / infinity / malformed numeric → invalid
- Bar/line negative value remains valid
- Scatter non-finite x or y → invalid
- Scatter with numeric x = y pairs
- Area and line render the same data differently
- Item label with inline modifier: `- red::urgent = 40%`
- Chart row with multiple `=` uses the last unescaped `=` outside code spans as the label/value separator
- Chart label with literal equals uses `\=`
- Chart row with no valid `=` value separator remains literal with composer hint
- Flow with a cycle
- Flow with edge labels
- Sequence with `->` and `-->`
- Timeline with out-of-order dates
- Mindmap with multi-level nesting
- Org chart containing a cycle → reject with a clear error
- State machine with self-transition (`- Idle -> Idle`)
- Single-node diagram
- Malformed edge (`- A B`) → literal
- Repeated identical normalized diagram node label resolves to the same node
- Empty diagram node name → invalid
- Table row with fewer cells than headers pads empty trailing cells
- Table row with more cells than headers is rejected/warned rather than silently truncated
- Table emits valid HTML rich-text fallback
- Table with all-empty headers → invalid
- Table with duplicate header labels remains valid and preserves positional identity
- Completely empty table row → invalid
- Recipe missing optional `serves::` / `time::`
- Recipe missing `ingredients:` or `steps:` → invalid
- Recipe with `steps:` before `ingredients:` → invalid
- Repeated recipe section marker → invalid
- Recipe ingredients checkable, not persisted
- Recipe step with literal semicolon

### Utility
- `calc::17 * 34;`, `calc::(5+3)/2;`
- `calc::` invalid expression → literal, no crash
- `convert::20C;` → °F; `convert::68F;` → °C
- `convert::` unknown unit → literal
- `roll::2d6;` → exactly two stored d6 face results, rendered as two volumetric 3D cube dice, subtotal 2–12
- Replaying `roll::2d6;` shows the same stored faces/result, never rerolls
- `roll::2d6, 1d20;` → per-group stored faces/subtotals plus combined total
- Large dice pools collapse to a compact visual instead of animating every die
- `roll::0d6;`, `roll::1d0;` → literal
- `pick::` single option
- Explicit-option `pick::` animates through candidates and lands on the stored choice
- `pick::number::` animation lands on the stored integer
- `pick::flip;` performs a volumetric 3D coin-flip animation, shows both faces in flight, and lands on the stored side
- Replaying any `pick::` animation preserves the original result
- `pick::number::100-1;` → reversed range handled
- `pick::unknowncategory;` → literal
- All four `swatch::` formats; invalid color → literal
- `rate::7/5;` → invalid/literal; never clamp
- `rate::4/0;` → invalid/literal
- `progress::150;` → clamp to 100
- `progress::-10;` → clamp to 0
- `kbd::` single key, no `+`
- `math::` inline and block; invalid LaTeX → literal
- `math::` renders through Sigil math metadata with a readable plaintext fallback
- `art::` whitespace preserved exactly
- `art::` keeps `red::text;` inside literal
- `art::` wider than bubble → horizontal scroll, no wrap
- `qr::` all four typed forms
- `qr::wifi::` password with special characters
- **`qr::` verify scannability in dark theme specifically**
- `qr::` very long input still scannable

### Service-backed
- Each with service reachable
- Each with service **unreachable** → literal fallback, clear error state, **no hang**
- `translate::auto::` detection
- `define::` word with multiple senses
- `define::` nonexistent word
- `weather::` ambiguous place name

### Rich-card UX
- Checklist long-list preview caps inline height and **Show all** expands it
- Poll result bars animate once and remain readable without color
- Reminder card shows resolved local date/time and fired state
- Timer restored from off-screen position derives current progress from absolute end time
- Chart mobile scrub selects nearest line/area point without hover
- Chart large dataset uses inline preview and full-data expanded view
- Diagram that cannot remain legible at bubble width offers **Open diagram** instead of over-shrinking
- Sequence diagram with many participants horizontally scrolls
- Table with many columns horizontally scrolls on mobile
- Recipe cooking mode preserves original quantities when local serving scaling is used
- Block math wider than bubble horizontally scrolls without shrinking below readable size
- Countdown never displays a negative remaining value and becomes elapsed-time semantics after the target passes
- QR Wi-Fi card does not expose password in the timeline
- QR expanded view preserves quiet zone and scanning contrast
- Translation event stores original, translated text, language metadata, source/provider, and resolution time
- Definition event retains a stable resolved snapshot and source attribution
- Current-weather card shows current temperature, condition, daily high/low, precipitation, and wind without expansion
- Forecast card shows multiple days and opens a touch-scrubbable hourly detail view
- Weather refresh produces a new clearly timestamped snapshot rather than silently rewriting history
- Received weather card renders without requesting recipient location permission
- Every structured card has a static readable state under reduced motion
- Replay action is offered for content animations but not for incidental UI transitions
- Dense structured cards cap inline height and expose **Open / Show more**


### Graphical creation / round-trip

- Every sendable construct appears in exactly one attachment-panel category or formatting surface
- Attachment-panel search finds constructs by canonical keyword and common synonym (`dice` → `roll`, `forecast` → `weather`)
- Builder-created event and syntax-created equivalent event normalize to the same canonical payload
- Reopening a syntax-created construct in the graphical editor preserves all supported options
- Reopening a graphical construct and switching to syntax does not lose semantic data
- Unsupported advanced option remains visible under **More options** rather than becoming syntax-only
- Mobile panel uses category drill-down and does not render the full catalog at once
- Desktop panel is fully keyboard navigable
- Recent/favorite tools do not duplicate or hide canonical category placement

### Structured grammar / escaping

- Empty structured block title where title is required → composer warning / literal
- Blank item line inside structured block → ignored only when explicitly allowed; otherwise preserved as empty item per construct rule
- `\;` inside structured item renders literal `;` and does not terminate the block
- Mixed Markdown plus Sigil modifiers inside a structured row parses without crossing row boundaries
- A structured block opener inside another structured block item is literal unless nested blocks are explicitly supported
- Nested structured blocks are currently unsupported and must fail/literalize predictably rather than recurse accidentally
- Duplicate block option category → last-one-wins plus composer warning
- Unknown block option → literal/error hint; never silently reinterpret as title
- Final `;` preceded by escaped backslash parity is handled correctly (`\;` escaped, `\\;` terminates after literal backslash)
- Windows CRLF and Unix LF produce identical parsed structure

### Identity references

- Bare `@user` resolves from known contacts/current participants when unambiguous
- Full `@user:server.example` address resolves explicitly and remains available for disambiguation/details
- Two identities with the same friendly `@user` require disambiguation before binding
- Unresolved friendly `@user` before send → no guessed identity binding
- Serialized mentions retain a stable identity reference even if display name or friendly handle later changes
- Reassigned `@user:server.example` must not retarget an old serialized mention bound to a different stable identity
- Offline mention of an already-known identity resolves from cached/local context
- Contact-card `user_id`, friendly handle, full address, display name, and avatar round-trip without retargeting
- Contact-card stable identity mismatch with authenticated identity data is rejected/warned
- vCard export/import preserves full `@user:server` address and stable Sigil identity metadata where supported

### Dates / time

- Relative date composed just before midnight resolves once at compose/send time and does not shift afterward
- Reminder across DST spring-forward gap resolves to a valid local instant with explicit confirmation
- Reminder during DST fall-back ambiguity requires/uses a deterministic offset choice shown in confirmation
- Timer received after its end time renders directly as Ended and does not schedule a stale notification
- Countdown target in recipient's different timezone still refers to the sender-resolved canonical instant/date semantics
- Monthly recurrence created on Feb 29 defines non-leap-year behavior and snaps back in leap years
- Device timezone change updates display/local notifications without changing stored canonical timestamps

### Randomness / deterministic replay

- Randomizer result is generated exactly once and persisted before send
- Retry/resend of the same event id does not reroll
- Multi-device rendering of one randomizer event shows identical result
- Ordinary randomizer UI never claims cryptographic/verifiable fairness
- Fairness-sensitive mode, if added, must use a distinct verifiable-randomness protocol and visibly distinct UX
- Replay animation cannot mutate stored result
- Reduced-motion path reveals the same stored result immediately
- Large valid dice pools at or below the configured hard limit may use compact rendering; notation exceeding the configured hard limit is invalid/rejected
- Malformed dice notation (`2dd6`, negative dice, zero sides, excessive sides) remains literal
- User-defined pick category deleted after send still renders stored chosen value
- Empty explicit pick list is invalid
- Duplicate explicit pick options are deduplicated before selection; duplicate entries never create implicit weighting

### Service-backed / network

- Translation provider timeout offers literal-send fallback without hanging
- Translation retry does not duplicate outgoing messages
- Weather geocoding returns multiple matches → composer requires location choice before send
- Weather snapshot with stale provider timestamp displays stale/freshness state
- Weather provider unavailable after receiving a stored snapshot does not affect rendering
- Definition with no result renders a stable not-found state, not a spinner
- Self-hosted provider returning malformed data fails safely and never injects unsanitized rich text
- Provider change after message creation does not alter stored resolved result
- Offline recipient can render all resolved service-backed cards from event data alone

### Accessibility / rendering

- Every interactive card is fully operable by keyboard
- Every graphical result has a meaningful screen-reader label and static text fallback
- Animations never convey the only copy of semantic information
- `scratch` works by swipe on touch, click-drag on desktop, and has a keyboard-accessible Reveal action
- High-contrast mode preserves chart/poll distinction without relying only on color
- 200%+ text scaling keeps primary controls reachable and prevents clipped critical data
- RTL text works in modifiers, tables, polls, and cards without reversing semantic operators incorrectly
- Emoji and multi-codepoint graphemes are never split incorrectly by text-effect ranges
- Unicode normalization differences do not corrupt effect offsets; canonical NFC + extended grapheme-cluster indexing yields identical ranges
- Very long unbroken strings do not force the timeline wider than viewport
- Copying structured-card semantic data returns text/data, not inaccessible visual-only output

### Resource / abuse limits

- Maximum structured-block size is enforced before expensive parsing/rendering
- Maximum chart points, diagram nodes/edges, table rows/columns, QR payload bytes, dice count, and animation span length have explicit configurable caps
- Oversized constructs degrade to literal/compact/error state without freezing the UI
- Malicious deeply nested Markdown does not trigger pathological parser behavior
- Repeated animated messages off-screen consume no active animation resources
- Heavy cards are lazily instantiated and disposed/recycled safely during rapid scrolling

### General
- Structured block row/item markers are interpreted according to the active construct before Markdown; ordinary rows use `- ` and construct-specific markers such as checklist `-x- ` / `-r- ` are also recognized
- Inline modifier inside a structured item is line-scoped and terminates at end of that item line without its own `;`
- Unescaped inner `;` inside a structured item is rejected/reserved for block termination; literal semicolon uses `\;`
- Final unescaped `;` on the last block line terminates the structured block
- Literal `\;` inside a structured item does not terminate the block
- Friendly `@user` may be shown when unambiguous, while serialized mentions retain a stable identity reference
- Full `@user:server.example` remains the explicit account address and may be shown for lookup, disambiguation, or identity details
- Ambiguous or unresolved `@user` requires disambiguation before binding a mention/contact
- Every construct inside a code span or fenced block → literal
- Every construct in a normal chat room → identical to notes room
- Unterminated structured block (no final `;`) remains literal/incomplete and never emits an interactive structured event

## Canonical backend representation

Platform integration is tracked in [Status.md](Status.md).

- Inline text uses NFC and grapheme indices under Unicode 17.0.0. Version-1 JSON contains plaintext, regenerated HTML and ordered `{kind,value}` effect descriptors. Unknown fields/versions, overlapping ranges, duplicate effects and unsafe links fail. Raw HTML is escaped; parsing never performs networking.
- Redaction removes content and associated link destinations before serialization. Original source and reveal data are not transmitted. Code retains literal redaction syntax; partial-grapheme styles expand to the whole grapheme.
- Bounds: 32 KiB source/body, 1,024 spans, 32 nesting levels, 4,096 intermediate pieces, nine gradient stops, 2,048-byte links, 4,096 bytes per grapheme and 60 KiB wire data. Overflow fails without returning unredacted source.
- Standalone cards bind creator account, timestamp and message ID to the authenticated event. Limits: 256 checklist items, 64 poll options, 512-byte titles and 2,048-byte labels. Inline text, cards and actions remain distinct typed bodies.
- Actions reference creation ID, creator and canonical digest. Their transport ID hashes the exact action; actor and timestamp match the authenticated envelope. Predecessors share the item/ballot register. Validated depth orders dependencies; concurrent siblings resolve by canonical account and action ID. Missing dependencies stay pending; invalid semantics become rejected journal entries.
- Task completions form an attributed set. Undo names one completion by the same account/item and declares a timestamp in its first 30 seconds. It removes only that completion. Declared timestamps are application claims, not trusted enforcement against modified clients.
- Recurring checks bind the exact calendar period. One-offs accept checks only in the creation period; persistent items reset in the current projection without replaying missed resets. Calendar rules pin bundled IANA 2026c. Missing 00:01 shifts forward by the gap; repeated 00:01 chooses the earlier instant.
- Creation tombstones block new outgoing actions. Archive deletion does not erase all causal journals, undo derived state or recall ciphertext. Recovery rebuilds from accepted history, never live messaging keys. Poll result visibility is a presentation rule; participants can inspect decrypted actions.
- Compositions contain up to 64 text/card parts. Child card IDs derive from the authenticated root message and ordinal; root deletion/expiry governs every child. Mentions bind an explicitly selected canonical contact. Prepared drafts freeze dates, randomness and contact bindings across restart.
- Creator edits bind exact definition revisions; recurrence delegation names a creator-issued policy. Revocation supersedes old-policy edits independently of their causal depth. Checklist actions may await a missing definition without applying to a different item set.
- Poll closure freezes the creator's observed accepted ballot heads. Authenticated 64-ballot pages prove inclusion in the closure's Merkle root; counts remain unavailable until every page validates, and late ballots cannot change them. Preparation/pages survive restart and use ordinary encrypted action delivery.
- Notes project current conversation content and card definitions. Durable versioned alarm jobs require platform scheduling/cancellation acknowledgments; callbacks recheck visibility and the exact version before returning notification text. Restored past reminders do not trigger a historical notification burst.
- Math uses bounded, macro-free LaTeX and escaped MathML without source annotations. Service cards retain resolved snapshots and attribution; recipients do not query providers to render them. Provider execution belongs to the maps/integrations subsystem.

Sources: [Unicode normalization](https://www.unicode.org/reports/tr15/), [grapheme segmentation](https://www.unicode.org/reports/tr29/tr29-47.html). Codecs and limits live in `text/src/` and `protocol/src/text.rs`.
