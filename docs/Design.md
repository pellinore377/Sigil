# Sigil design

Status: the design contract as built. Where a rule names a component, that is how the component is styled today; a change to the rule is a change to the component.

## Intent

**Modern correspondence, with the intimacy of a notebook.** Deliberate typography, precise details, quiet surfaces, room to think. The interface is calm, personal and contemporary, expressed through type, spacing, shape and tonal contrast. No paper textures, handwriting fonts, ornamental controls or effects that compete with messages.

Compose Multiplatform implements one visual language on every client. Platform accessibility and store acceptance remain separate checks.

## Tokens

| Token | Rule |
| --- | --- |
| Palette | Ink and paper neutrals. Light: paper surfaces, dark ink. Dark: charcoal and black, light ink. No chromatic brand accent by default; the accent is the user's. |
| Spacing | 4-unit grid: 8 within a control, 12 between a panel and the composer, 16 within a section, 24 between sections. Compact message grouping is the one exception. |
| Corners | 24dp on chrome and cards, 20dp on tonal groups, 16dp on buttons and tiles, 14dp on fields and keys, 10dp on chips and cells. |
| Shapes | Squircles for controls, tool tiles, fields, keys and selection. Circles for avatars, receipts, radio and swatch dots. |
| Type | Newsreader everywhere by default, Google Sans Flex as the alternative, Google Sans Code for code. Messages 18/26, supporting 16/23, metadata 14/20, in scalable units. |
| Small capitals | Section and day labels are labelSmall, uppercase, 1.4sp tracking, in the quiet ink; they read aloud as written. |
| Quiet ink | onSurfaceVariant for supporting text, glyph-only keys and metadata, never below 4.5:1 on its surface. |
| Tonal fill | onSurface at 4.5% alpha for groups, cards and fields; 7% for a tile at rest; 16% for a selected tile. Never a border. |
| Glyphs | Material Symbols Rounded. Outline for ordinary actions and inactive navigation, filled for selected navigation. 24 in 48 targets; 20 or smaller only for metadata. One action, one glyph, everywhere. |
| Shadows | Painted soft shadows on glass only (8dp blur, 15% black, 3dp down). Never Android elevation. |

## Glass chrome

Headers, the composer, the main navigation, composer panels, the reaction drawer and every viewer header share one treatment: a captured backdrop blurred at 14dp, tinted with surfaceContainerHigh at 82%, clipped to a 24dp corner, with the painted shadow. The capture wraps the page ground, so what shows through is the real page, not a copy.

- Conversation header and main headers float 12dp from the sides and sit 12dp below the status bar. Main and settings headers are 68dp with the headline-medium title; the conversation header is taller and carries back, avatar, name, call, video and menu.
- The composer floats 12dp from the sides and 8dp above the bar or keyboard. Nothing sits behind the gesture area.
- The main navigation is a 64dp icon-only bar with 16dp side insets, in the order Messages, Calls, Notes, Settings.
- A page's rounded corner shows the page beneath it; glass never has a bright edge.

## Floating panels

Every composer panel is its own glass above the composer, separated by one 12dp gap, the same as the composer's side margin. The gap animates with the panel. A panel fades through its last 96dp as it collapses, so it never thins into a line.

| Panel | Height and content |
| --- | --- |
| Attachments | 252dp. Five tiles across, 48dp squircle tiles with 24 glyphs and labelSmall labels that take their column's width and wrap. Emoji, Photos, Camera, Files, One-time location, Real-time location, Drop a pin, Create, Format. |
| Voice | 252dp, the same as Attachments. Stage, then Cancel, the record or stop key, and Attach. Stopping keeps the recording in the panel; only Attach hands it to the composer. |
| Emoji | Peek at 320dp, full to one gap beneath the header. See the emoji sheet. |
| Create | Five across, paged, no search or categories. |
| Format | 104dp compact toolbar that works with the keyboard. |
| Camera and location | Fill to the header's gap. |

A panel never rises past the header. The ceiling comes from the live insets, the composer's own height and the header's bottom, and is enforced in layout so the keyboard cannot push a panel under the header while it moves.

## Tonal groups

Settings category pages and the onboarding card use tonal groups: rows on one 20dp surface at 4.5% ink, hairlines at 8% between rows, a small-capital label above each group and notes in the quiet ink outside the groups. The settings index itself keeps continuous rows. Toggles toggle from the whole row; choices are segmented squircle rows; the typography choice is tiles.

Appearance follows this on every page and keeps its live timeline preview inside the first group. Accent presets are twelve muted swatches, six to a row, each taking its share of the width so the rows fill and sit centred; a custom colour sits beneath with the current name.

## Keys, buttons and fields

- **Key**: a 40 to 44dp squircle on surfaceVariant with a 22 glyph, for call-back, tool and viewer actions. The primary key uses the primary colour.
- **Button**: 16dp squircle, filled for the one primary action on a surface, outlined for a reset, text for quiet choices at the foot of a card.
- **Field**: a quiet tonal fill at 6% ink with a 14dp corner and no outline, on cards; outlined fields remain in builders.
- **Chip**: 10dp squircle, tonal at rest, primary when selected; a selected chip carries its name, the rest only their glyph or emoji.
- **Switch, radio, check**: platform controls with the row as the target.

## Messages and cards

Messages are the centre. Clear grouping, modest metadata, distinct incoming and outgoing tones derived from the accent, readable width on wide windows. Emoji-only messages have no bubble and play Noto animated emoji under the motion preference.

The attachment or card is the message surface; no second bubble around it. Filled content paints no ground beneath it, so no grey bleeds at the corners in light mode. Captions attach beneath the content on their own ground at the card's width.

| Content | Timeline | Expanded |
| --- | --- | --- |
| Images, GIFs, video | Aspect-preserving frame, GIF chip or play key, no type label | A carousel of every picture, clip and gif in the conversation, over a 92% scrim, with a glass header and reaction bar; pinch, pan and double-tap; a zoomed picture holds the carousel still |
| Voice memo | Play key, seekable waveform, time, caption | The same player, larger |
| Music and audio | Album art, title and artist, a music-note badge in a squircle | The Room player: art and title centred between the glass header and the transport, a slim scrubber, a squircle play key, a lyrics key that lights only when lyrics are in view, and lyrics that scroll beneath with a frosted return-to-top key |
| Text, Markdown, sheets, PDF | A page preview with a type chip and the file name | A viewer with a glass header (back, name, type and size, download); text is selectable; PDF pages ride a carousel with pinch zoom, and zoom keys appear only on the web |
| Sheets | The first rows in a grid | A full grid: tap selects a cell, a row number or a column number; one round handle on the block's lower-right corner stretches it and springs back; a hold offers to copy the block as tab-separated text. Selection is a 2dp primary border on the block's outer edge, never a fill |
| Other files | A chip with name, type and size | File details with Save and Open externally |
| Code | Google Sans Code, language label, Copy | Selectable code with wrap and horizontal scroll |
| Contact | A 48dp ink-tinted monogram (letters only, the person glyph for a hidden or letterless name), the name in titleMedium to 3 lines with an ellipsis, the full address in quiet labelMedium, then a full-width ink Message action on received cards only; one spoken label, "Contact, name, address"; no type header | — |
| QR code | A white 14dp tile snapped to whole modules with its quiet zone, never inverted; beneath it what scanning yields (the URL or address in code type, the network name with "password hidden", or the text) and one ink action for the recipient: Open link, Copy password through the sensitive clipboard, or Message for a contact code (the same `contact_open` as the contact card); outgoing codes carry no Copy password | Tapping the tile opens a larger tile, with Show password for Wi-Fi |
| Pull quote | A 1.5× displayLarge opening mark hanging at ink 56% in a 36dp margin, the quotation in italic titleMedium, then "— Author, *Source*" in labelMedium on its own line at the same start edge; no type header | — |
| Keyboard shortcut | Mechanical keycaps lit from above, shaded from the bubble ground: a 47dp body with a 10dp squircle and 3dp front edge around a 35dp face with a 7dp squircle; code type at labelMedium; modifiers carry Material Symbols glyphs (⌘ ⌥ ⌃ ⇧ ↵ ⇥) beside the word the sender typed; quiet "+" joiners wrap with the key after them | — |
| Color swatch | An opaque 14dp-cornered rectangle with no border, ring or checkerboard (200 × 96dp alone, 96 × 64dp in a palette of consecutive swatches, three to a row, four as two by two), the value as #RRGGBB in code type beneath and any alpha in words ("50% opacity"); a colour that matches the bubble ground gets an ink 12% hairline | — |
| Recipe | Title, then servings and time with a − / + servings stepper that rescales amounts (adjusted lines are marked); INGREDIENTS as 48dp check-off rows (struck through, shared with the cooking view for the session) and STEPS numbered, capped at 5 and 3 with an in-place "Show N more" | The cooking view, from the menu's Details |
| Translation | Small-caps target language over the translation in titleMedium, then the small-caps source language (with "detected") over the original in quiet ink; provider attribution as a quiet foot, a link when the provider gives a source | — |
| Definition | The word in headlineMedium, pronunciation and language in quiet ink with a play key when audio exists, senses numbered and grouped under small-caps parts of speech, examples in quiet italic, five senses then "Show all N"; web bundles a Noto Serif IPA subset so pronunciations never fall to tofu | — |
| Weather | Place as title; whole-degree hero in displayMedium beside the condition glyph and words, high/low/feels-like in quiet type; one row of up to four metrics; an hourly strip that fits the width (never scrolls); for forecasts, day rows with a low–high range bar on a shared scale; attribution foot with a °C/°F switch defaulting by region | — |
| ASCII art | Code type on the bubble itself, no panel, 1.3 line height, never wrapped or scrolled; wide art scales uniformly to fit and becomes a button labelled Enlarge | Wide art opens full screen at 1:1, scrolling both ways |
| Diagrams | Flows in square boxes with decision diamonds; states in rounded tiles with an entry dot, a bullseye under every state with no way out, and curved returns; mind maps from an ink centre with branches clockwise above, right, below and left and each branch's topics stacked on its far side, capped at 16 topics. Mind map branch families follow the chart colour rule: an ink ramp up to four, the named palette from five | Every topic; large mind maps on deterministic rings |

Every attachment has an expanded destination, reached through the in-window presentation host rather than a dialog, so its glass blurs the real page. Expansion preserves playback and return position.

## Message menu

A hold on a message opens the menu in the presentation host, above the page. Everything beneath, page, header and composer, blurs as one layer at 18dp and dims by half. The menu is a sandwich: the reaction pill, the bubble, then the actions, centred in the band between header and composer. The bubble is a copy composed in the host; the row's own bubble stays composed but invisible so nothing reloads. The copy travels from the row to its slot with a small spring and back again, clipped to the band so it passes beneath the chrome; the pill and actions are never clipped. Media cards land with the picture already in hand.

The pill holds six quick reactions and an add-reaction key. Actions are Reply, Forward, Copy, Reply in thread, Pin, Add to notes, then Details, Replay animation, Edit and Delete when they apply.

## Emoji sheet and reaction drawer

One sheet serves both. Eight emoji across between hairline rules, a search field at the top, and category chips pinned along the bottom, each an emoji, the current one named. Tapping a chip glides to that group. A hold on an emoji with skin tones opens a small row of the six variants. Search shows recents first, then narrows by Unicode name as you type. Recents come first in the sheet too.

- **Panel**: from the Emoji tile. Two heights, peek and full; a drag rides between them and settles on the nearer when let go. The full sheet takes the layout ceiling itself and tracks the keyboard frame for frame. Picking inserts at the cursor and keeps the panel open.
- **Drawer**: from the add-reaction key. Frosted glass over a 42% scrim, covering the composer, with a handle. Peek at 46% and full at 92% of the window, never past the header; the handle drags up to grow, down to shrink, and past the peek to close. Its chips sit above the gesture bar. Picking reacts and closes.

Emoji are the platform's own. Recents live in the app.

## Calls

The register: All and Missed chips, small-capital day labels, then one row per call with the avatar, the name, a quiet line of glyph plus outcome and length, a tabular time and a squircle call-back key. A missed call reads by its word and glyph, never by a warning colour. Tapping a row slides in the details: avatar, name, when the last call was, a primary audio key with video and message beside it, then that person's calls. Never infer missed state or duration from incomplete history.

## Onboarding

The mark and the word Sigil above one tonal card that refills as the steps go by, with quiet text actions beneath it. Everything in the card shares one left edge.

1. **Server**: the address field and a status line with a dot once the server answers, naming what it offers.
2. **Sign in**: only what the server offers. One method ends the card in a single primary key; several become rows with a squircle key, a title and a line saying what the method means.
3. **Refilled**: the server line becomes the card's header with Change, and the card holds the password fields, the invitation code, or the name step with its verified line. The foot offers the other ways in. While single sign-on is open elsewhere, the card holds one quiet waiting line; the foot offers Open sign-in again and Cancel.
4. **Welcome back**: only after signing in to an account that already has devices. The address, one line, and Recover with passkey as the key; the foot offers Use a recovery code, Link from another device instead, and Start over with a new identity, which asks first because contacts must accept the new identity and old devices are signed out. Without passkeys on this platform the recovery code is the key.
5. **Protect your account**: once, for a new account without a passkey. Create passkey as the key, Not now at the foot.
6. **Before you begin**: once, after the first sign-in, a card of what contacts may see with the real read receipt, typing, activity and message request switches, then Start messaging.

The first card's foot carries only the link action, which needs no server: a device with a camera scans the other device's code (Link from another device); the web shows its own (Link from your phone). Recovery never appears before sign-in. Settings keeps Account recovery (passkeys, Add a passkey, Show recovery code); linking a new device from Devices offers Show a code or Scan its code. About carries a tap-through preview of the flow with made-up state.

## Inbox, notes and settings

Continuous rows with circular avatars, readable names and previews, a quiet timestamp. Selection uses the same floating header with Close, a count, Pin, Collection and an overflow. Collections are optional squircle tiles on fixed tracks that scroll with the rows. Notes shows only conversations with notes, Note to Self first. The settings index keeps its compact profile, quiet leading glyphs and chevrons; the category pages use tonal groups. Admin sections that configure an outside service offer one-tap presets as switch rows; key fields are write-only, and errors show beside the save button.

## Motion

Brief and purposeful: acknowledge an action or explain a transition. Panels tween over the standard duration and snap while the keyboard moves. Page changes slide horizontally with a fade; call details slide in over the register. The menu bubble uses a light spring. Dice, coins and cards animate behind the chrome, never over it. Reduced motion is always honoured; message effects replay only on request.

## Reply gestures

Dragging a message away from its edge reveals Reply; toward its edge reveals Reply in thread. The indicator follows the drag, arms at the threshold and fires on release. Media, captions and cards take part; seeking, selection, maps and viewer gestures win inside their controls. Both actions stay in the message menu.

## Location

One-time location, Real-time location and Drop a pin live in the attachment grid and open inside the panel above the composer. One recentre control in the map, a small Drop a pin chip, duration choices for live sharing, and the composer's own send key. Shared cards are map surfaces with the ordinary receipt; live rings reflect fresh updates only and end into a concise ended state. The expanded map uses the glass chrome, a dominant map and remaining-time or Stop sharing controls.

## Theme

The accent derives a coordinated palette: incoming and outgoing tones, supporting surfaces, text, glyphs, selection and focus. Each conversation may carry its own accent and background, private to the viewer, inheriting anything unset from the account. Semantic roles only, never literals in components. Error, destructive and security states keep their meaning and a non-colour cue. Media and avatars are never recoloured. SigilText named colours resolve against the effective theme.
