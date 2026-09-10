# Sigil design direction

Status: agreed design contract; requirements below are not claims of implementation completeness.

## Intent

**Modern correspondence, with the intimacy of a notebook.**

Sigil takes inspiration from the care of writing with a fountain pen in a well-made notebook: deliberate typography, precise details, quiet surfaces, and room to think. The user's Leuchtturm1917 notebook and LAMY Safari are experiential references, not products to copy or brands to reproduce.

The interface should feel personal, calm, tactile through its proportions, and contemporary. Express this through typography, spacing, shapes, and tonal contrast. Avoid simulated paper textures, handwriting fonts, ornamental antique controls, and decorative effects that compete with messages.

Compose Multiplatform implements the shared visual language without requiring Material component styling. Platform accessibility and Apple-device acceptance remain separate checks.

## Visual language

- Neutral ink-and-paper palette: whites, off-whites, grays, charcoal, and black. The default has no sage-green or other chromatic brand accent.
- Deliberate margins, readable line lengths, and consistent vertical rhythm.
- Use tonal separation before decoration. Main-tab headers match the page; conversation headers and footers have no border or shadow. The gesture area has no separate background strip.
- Squircles identify controls, tool tiles, fields and selection highlights. Circles identify avatars, receipts and radio controls. Avoid redundant cards around rows or content.
- Share spacing and corner tokens across clients: use a 4-unit spacing grid, usually 8 within related controls, 16 within sections and 24 between sections. Compact message grouping is an explicit exception. Size optical adjustments centrally, not per screen.
- Hover, press and focus treatments follow the whole control's shape, never its label separately. Each focused task has one prominent primary action, quieter secondary actions and explicit destructive treatment.
- Typography and content establish hierarchy before decoration does.
- Motion is brief and purposeful: acknowledge an action or explain a transition.

Light mode uses paper-like neutral surfaces and dark ink. Dark mode uses charcoal/black surfaces and light ink, with its own contrast relationships. Neither mode requires a textured background. The identity must survive changing the accent, font, or conversation background.

## Typography and icons

The global typography setting has two choices:

| Choice | Scope |
| --- | --- |
| Newsreader — default | All ordinary application text, including messages, headings, navigation, buttons, settings, timestamps, and administrative screens. |
| Google Sans Flex | Replaces Newsreader throughout the same roles. |

Do not use a fixed serif-heading/sans-control split. Google Sans Code remains the font for code blocks and explicitly monospaced content under either choice.

Use size, weight, and spacing to establish hierarchy within the selected family. Bundle fonts locally. Provide appropriate fallback fonts for scripts and glyphs missing from the selected family, while preserving the user's choice wherever supported. Emoji retain appropriate emoji rendering.

Verify both choices in compact controls, long localized labels, numeric data, and enlarged text. Layout must adapt rather than clip labels or depend on one font's measurements.

Starting size/line-height roles are 18/26 for messages, 16/23 for supporting text and 14/20 for metadata, in scalable units. Tune Newsreader optical sizing and baseline metrics centrally; do not compensate with arbitrary padding or compress multiline text. Controls remain optically centered.

Material Symbols Rounded uses consistent weight and optical size, filled by default. One action has one icon across the app:

| Meaning | Rule |
| --- | --- |
| Main navigation and ordinary actions | Filled; selected navigation adds a squircle background. |
| Pin/bookmark toggle | Outline when unset, filled when set, with a corresponding accessible label. |
| Microphone/camera | Normal versus slashed glyph with an explicit state; outline alone never means off. |
| Checkbox/radio | Empty versus checked/selected, preserving familiar semantics. |
| Back / dismiss / search | `chevron_left` / `close` / `search`; retain their natural line forms. |
| Attach / send | `add` changing to `close` when open / `send` paper airplane, including captions and voice messages. |
| Place / locate myself | `location_on` / `my_location`. |
| Reply / reply in thread | `reply` / `forum`, accompanied by labels during gestures. |

Use 24-unit action glyphs within at least 48-unit touch targets; compact metadata uses smaller noninteractive glyphs. Unfamiliar actions need visible labels. Back/reply direction follows layout direction; unrelated symbols do not mirror indiscriminately. A lock means encrypted content, not independently verified identity.

## Global appearance

Global appearance establishes the inherited theme for the app and all conversations without overrides.

- **Palette source:** Sigil neutral default, a user-selected custom accent, or Android dynamic colors where available.
- **Accent color:** explicitly customizable in the global theme settings. A custom accent produces coordinated interactive, selection, focus, and supporting surface colors.
- **Appearance mode:** light, dark, or follow system, independent of palette source. Follow system is the proposed initial setting.
- **Typography:** Newsreader or Google Sans Flex throughout, with the code-font exception above.

The neutral default still has accent roles; their values are neutral. Selecting Android dynamic colors is optional and does not change Sigil's typography, geometry, or component design. Selecting a custom accent replaces the dynamic palette source rather than creating competing accent settings.

Offer a preview and a clear reset to the Sigil default. Avoid making users configure dozens of individual color roles to obtain a complete theme.

Appearance is a settings hub with focused pages:

| Page | Controls |
| --- | --- |
| Colors & backgrounds | Mode, neutral/custom/dynamic accent, curated presets, inherited chat background and a real timeline preview. Custom picker exposes hex/RGB under Advanced. |
| Typography | Font family, text size and representative writing/code previews. |
| Layout | Comfortable/compact conversation rows, preview-line count, optional collections and their labels; pane/sidebar options on wider screens. |
| Motion & media | Reduced motion, message effects and GIF autoplay. Download/storage policy belongs elsewhere. |
| Maps | Follow appearance or a chosen map style, light/dark variants and supported label-font choices; preview before applying. |

Appearance defaults to private account synchronization. Advanced allows this device to opt out without affecting other devices; returning to account settings clearly replaces its local override. Platform-only settings and layout choices for incompatible form factors remain local. Background assets require encrypted synchronization before claiming they follow the account. Appearance controls presentation; location permissions, providers and live-sharing consent belong outside it.

## Private conversation themes

Every conversation may have a theme that belongs to the viewer. In a DM, both participants can independently customize the entire conversation as it appears to them. The same ownership model applies to groups. A theme change does not alter anyone else's screen, become a shared room setting, or send a conversation event announcing the change.

The core interaction is **choose a conversation accent and derive a complete, coordinated conversation palette**. The reference is the idea of accent-based chat customization in Google Messages, not an exact copy of its controls or implementation.

The derived palette covers the whole conversation surface:

- Background and supporting surface tones.
- Incoming and outgoing bubbles, with distinct but coordinated treatments.
- Text, secondary text, links, icons, and dividers.
- Header, composer, buttons, selection, and focus states.
- Reactions, quoted replies, and structured SigilText card surfaces and controls.

“Customize everything” means the theme reaches all these surfaces. It does not require a separate color picker for each implementation-level role. The accent is the primary control; advanced individual-role editing is not yet specified.

Conversation backgrounds support solid colors, gradients, and user-selected images. Provide a preview containing both message directions, the composer, and a structured card. Allow image positioning/cropping and a readability treatment such as dimming or a contrast overlay. An image does not silently replace the chosen accent; extracting an accent from it can be considered separately.

Inheritance rules:

1. Start with the global palette and appearance mode.
2. Apply the conversation accent, if set, to derive local color roles for the active light/dark mode.
3. Apply the conversation background choice, if set.
4. Resolve readable foregrounds and protective surfaces against that result.

Unset choices continue to inherit. A conversation with only a custom background still follows global accent changes. An explicit conversation accent persists when the global accent changes. Clearing an override restores inheritance; resetting the conversation clears all its appearance overrides. Typography remains the global choice.

Synchronize private theme preferences encrypted across linked devices by default, subject to the device opt-out. Image framing may remain device-specific. Never present local-only assets as synchronized or turn viewer-private customization into shared room settings.

## Theme implementation contract

Use shared semantic color roles rather than literal colors scattered through components. The same palette derivation must produce consistent results across clients. Retain only one active theme implementation as the design evolves.

Derive useful tone relationships, not a blanket tint over the screen. Incoming/outgoing content, primary/secondary actions, and foreground/background must remain distinguishable for very light, very dark, saturated, and neutral accents.

Outgoing bubbles and primary actions carry stronger accent emphasis; incoming bubbles and supporting surfaces use quieter related tones. Conversation menus, builders, viewers and pickers inherit the conversation palette; account settings use the global palette. Pair foregrounds with their actual surface, including captions, playback controls, cursors and selection handles. Maintain at least 4.5:1 normal-text contrast and 3:1 for essential control/state cues. Wallpapers receive protective surfaces/overlays as needed.

Theme customization must preserve error, warning, destructive-action, and security-state meaning. These states may retain functional colors in the otherwise neutral default and always need a non-color cue. Photographs, video, avatars, and authored media are not recolored. Special rendering requirements, such as a scannable QR code's light backing, take precedence over decorative theming.

SigilText named colors resolve against the effective conversation theme as required by SigilText.md. Redaction, hidden poll results, and other privacy semantics must remain correct in every visual and accessible representation.

## Layout and principal surfaces

### Inbox

Use the cleaner reference inbox as the starting point: clear conversation rows, circular avatars, readable names and previews, restrained separators, and consistent timestamp/unread placement. Keep search and new-message actions discoverable without an oversized header. Reduce competing pins, badges, and status symbols.

Collections remain optional and off by default. Enabling them must not make navigation excessively tall. Messages, Calls and Settings share persistent bottom navigation. Notes shows only conversations containing notes, with Note to Self first when used; notes pins are independent of inbox pins.

### Conversation and composer

Messages are the visual center. Use clear message grouping, modest sender metadata, deliberate spacing, and distinct incoming/outgoing treatment. Preserve readable content width on larger windows.

Emoji-only messages render without a bubble and play [Noto animated emoji](https://googlefonts.github.io/noto-emoji-animation/) (CC BY 4.0). Keep Unicode text as the encrypted message content; this is a client presentation rule, separate from reactions. Bundle assets locally with attribution, honor reduced motion, and use static emoji when animation is unavailable. Choose the asset format during UI implementation.

The composer should feel like a writing space. Keep the main text entry and send action clear; disclose formatting, attachments, and structured-content builders progressively. Builders should preserve drafts and return naturally to writing. Avoid duplicated composers and layers of nested sheets.

The composer grows to hold an attachment tray above a persistent text/caption field. Items can be previewed, removed or retried without losing their caption. Cap growth and scroll the tray so input and Send stay reachable above the keyboard. Sending commits attachment and caption together; a failed upload retains both for retry. The attachment is not a replacement for the text field.

Create uses search and the categories in SigilText.md: Plan & Organize, Ask & Decide, Data & Visualize, Share & Encode, Reference & Services; Help remains separate. Formatting stays with the text controls. Builders share a title, essential fields, expandable options, preview and a named action. Each poll/list item gets its own field plus one trailing empty row; insertion animates without stealing focus. Use date/duration/choice controls where appropriate. Builders remain within the continuous composer panel, expanding that same surface when needed. Keyboard/panel switches preserve its height without overshoot.

Structured cards belong to the same visual system as messages. Give them clear titles, content hierarchy, and obvious actions. Cap large inline content and provide expanded views, following SigilText.md. Composite-looking AI mockups do not authorize unsupported nested structured blocks or change message semantics.

Cards share a small type indicator, meaningful title, content and relevant actions. Notes read like writing; polls expose choices; checklists have directly usable rows; charts/tables/diagrams expand for detail. Graphical builders and typed syntax share the same canonical content and renderer, including previews. Hidden content must remain hidden in previews, accessibility and expanded views.

### Media, captions and viewers

The attachment/card is the message surface; do not add another bubble around it. Preserve sender direction, grouping, reactions, pins and tap-for-details receipts. Captions attach to the same surface below its content, with readable backing rather than text over a busy image.

| Content | Timeline / expanded view |
| --- | --- |
| Images | Rounded aspect-preserving preview / zoom, pan and full-resolution inspection. |
| GIFs and video | Poster or permitted animation with playback/duration cues / larger playback, seeking and relevant audio controls. GIFs honor autoplay/reduced motion; videos do not autoplay sound. |
| Voice memos | Integrated play/pause, seekable waveform, elapsed/total time and optional caption / larger waveform, seeking and speed control. |
| MP3 and other supported audio | Compact player with title and available metadata / larger player with seeking and speed controls. Keep recording-specific affordances out of music/file players. |
| Link previews | One coherent title/domain/description/image card, integrated with accompanying text / preview details and explicit Open link. Viewing details must not silently launch a browser or fetch new remote content. |
| PDFs, documents and other files | Type, name, size and supported safe preview / larger supported viewer, otherwise file details with explicit Save/Open externally. Never imply an unsupported format can render. |
| Code | Google Sans Code, preserved whitespace, theme-aware highlighting when recognized, language label and Copy / expanded selectable code with wrap toggle and horizontal scrolling. Unknown languages stay readable plain code. |

All message attachments have an expanded destination, even when it can only show file details. Expansion preserves caption, playback position and timeline return position. Small media must not stretch; inline crops must not alter original content. Viewers adapt to large windows and keyboard navigation. Loading, transfer progress, cancellation, retry, missing/deleted content and unavailable preview use shared states. Preview and expansion obey view-once, expiry and existing save restrictions.

Camera opens within the attachment surface, with usable preview, shutter, camera switch and close/back controls clear of system insets. After capture, show Retake/Use, then stage the result with an editable caption; capture never sends automatically. Only the necessary permission is requested when capture starts.

Voice recording uses the same growing composer: recording indicator, responsive waveform, timer, stop/pause/resume and discard where supported. Stopping produces an editable draft with playback and a caption field, then explicit airplane Send. Switching panels does not silently discard or send a recording. The draft player and sent player share their anatomy and visual transition; errors preserve usable recorded content.

### Reply gestures

Dragging a message away from its anchored screen edge reveals the reply icon and “Reply”; dragging toward that edge reveals the thread icon and “Reply in thread.” The revealed indicator follows drag progress, reaches a clear armed state at the threshold, and triggers on release with optional haptic feedback. Reversing/cancelling restores the message without action. Captions and media participate in the same gesture; waveform seeking, text selection, map panning and viewer gestures take priority within their controls. Both actions remain in the message menu for keyboard and accessibility use.

### Location

Offer three explicit choices: **Send current location** (one sample), **Share live location** (updates for a chosen duration), and **Drop a pin** (chosen place). Optional address/place search requires a geocoder: map tiles alone do not provide a search index. Current location and pin placement remain available without address search, including a text coordinate alternative to map gestures.

Show the selected place and available accuracy before sending. Request permission when locating the device, not when opening the picker. Current/live markers contain the person's avatar or initials; dropped pins use an ordinary place marker. Current location shows its sample time and never implies ongoing tracking.

Live markers use subtle expanding radio rings while updates are fresh and sharing is active. Show remaining duration, last update and Stop sharing. Stale/offline/ended states stop the rings and display their actual status; reduced motion uses a static live indicator. Animate movement only between received positions, never invent location or precision. End/expiry stops further sharing and leaves an explicitly ended card.

Each mode has a compact map card and an expanded map with clear identity/place details, accuracy when known, recenter and appropriate sharing controls. Expand the existing panel for map selection; preserve it on Back. Map style follows the viewer's preferences, keeping markers and labels readable; source attribution remains visible. Provider settings disclose who receives search queries. Search and opening a map never silently start live sharing.

### Motion

Preserve the shared stationary header/footer surfaces. Main tabs move according to their order; header items change within the same header. Subpages and timelines rise from below, behind the footer; composer controls replace navigation controls as the footer grows. Conversation tint follows the main movement. Reverse on Back, keep the gesture area transparent and do not restart transitions during sync refresh.

Use shared timing bands: roughly 120–160 ms for feedback and 180–240 ms for inline expansion; preserve the accepted navigation choreography. Inner composer pages enter from the right and reverse on Back. Retarget interrupted motion from its current position; never queue stale toggle animations. Layout, selection and sending must not wait for decorative motion.

SigilText content effects follow its existing one-shot/new-message, settled-history and explicit-replay rules. Reserve final text layout before playback: wave/typewriter effects cannot resize bubbles or push the timeline repeatedly. Animate shaped text without splitting joined scripts or emoji. Selection, copying and accessibility retain canonical text; spoilers remain protected. New rows and option sections expand smoothly without moving the active input out of view. Reduced motion presents settled content; offscreen content does not animate.

### Calls

Use the dark call reference for participant layout, rounded video tiles, restrained active-speaker indication, and a clear control group. Mute, camera, speaker, sharing, and ending a call need recognizable states and labels. Keep the destructive end-call action visually distinct. Call presentation may use a dedicated dark surface where appropriate without changing the user's global theme preference.

### Administration

Apply the same typography, spacing, and surface language to setup, health, storage, backups, users, and updates. Lead with understandable state and the next useful action. Operational warnings must remain prominent enough to act on within the otherwise quiet interface.

### Adaptive layouts

Share the design language across phone, tablet, desktop, and web. Use available width for panes and readable content rather than stretching a phone layout. Respect platform input, insets, keyboard behavior, and accessibility conventions. Exact breakpoints and component measurements remain implementation work.

## Accessibility and performance

- Maintain readable text and distinguishable controls across both fonts, both appearance modes, dynamic palettes, conversation accents, and image/gradient backgrounds.
- Support large text, including 200% scaling, without clipping core actions; support RTL, localized content, keyboard navigation, and screen-reader semantics.
- Provide visible focus and adequately sized interaction targets. Do not rely on color alone for state or message direction.
- Honor system reduced motion and the SigilText animation/replay rules. Pause offscreen motion and keep static content understandable.
- Prefer inexpensive solid/tonal surfaces and overlays. Avoid always-on blur, simulated paper shaders, and decorative continuous animation.
- Cache derived palettes and appropriately sized background assets; theme work must not slow typing or timeline scrolling.

## Reference images and logo

The supplied Design Mockups are visual guides; explicit decisions take precedence over AI artifacts and outdated borders, icons or labels. Use the supplied production SVG logo assets, not the exploratory raster logo.

References: Mobbin's [buttons](https://mobbin.com/glossary/button), [fields](https://mobbin.com/glossary/text-field) and [panels](https://mobbin.com/glossary/bottom-sheet); Google's [Material Symbols](https://developers.google.com/fonts/docs/material_symbols); WCAG [text](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html) and [control contrast](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html). These inform interaction rules without replacing Sigil's identity.

## Review before production styling is accepted

Review synthetic inbox, conversation, structured builder, call, and administrative screens in both font choices and both appearance modes. Include a neutral default, a custom global accent, Android dynamic colors, and independently themed conversations with solid, gradient, and image backgrounds.

Verify inheritance and resets, viewer-private ownership, light/dark transitions, legibility under extreme accent choices, large text, RTL, keyboard focus, and screen-reader navigation. Validate platform behavior on each actual target; visual approval does not resolve the outstanding framework accessibility or Apple-platform checks.

Every shared component also needs keyboard-open, hover/focus/pressed/disabled/error/loading and interrupted-animation review. Include camera/voice drafts with captions, each media family and expanded viewer, all location modes, code, and both reply gestures. Preview screens reuse production components. Remaining optical measurements and map-style presets require visual acceptance; avoid screen-specific substitutes.
