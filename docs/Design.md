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
- Use tonal separation before decoration. Main-tab headers float over their scrolling content. Conversation headers and composers float over the timeline with rounded corners, translucent tonal surfaces and a soft shadow; no borders or bright edge highlights. The timeline extends behind the transparent gesture area.
- Squircles identify controls, tool tiles, fields and selection highlights. Circles identify avatars, receipts and radio controls. Avoid redundant cards around rows or content.
- Share spacing and corner tokens across clients: use a 4-unit spacing grid, usually 8 within related controls, 16 within sections and 24 between sections. Compact message grouping is an explicit exception. Size optical adjustments centrally, not per screen.
- Hover, press and focus treatments follow the whole control's shape, never its label separately. Each focused task has one prominent primary action, quieter secondary actions and explicit destructive treatment.
- Typography and content establish hierarchy before decoration does.
- Motion is brief and purposeful: acknowledge an action or explain a transition.

Translucent chrome uses painted soft shadows rather than Android elevation shadows: elevation can expose rectangular shadow gaps behind child text and icons. Preserve the glass blur and tint. Main-header titles align by the bundled font's cap height rather than an uncorrected line box. Supporting text uses a quieter ink role with at least 4.5:1 contrast on its supporting surfaces.

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

Material Symbols Rounded uses consistent weight and optical size. Ordinary actions and inactive navigation use outlines; selected navigation uses filled glyphs. One action has one icon across the app:

| Meaning | Rule |
| --- | --- |
| Main navigation and ordinary actions | Outline actions and inactive navigation; selected navigation uses a filled glyph and squircle background. New-conversation/draft stays outlined, including on hover. |
| Pin/bookmark toggle | Outline when unset, filled when set, with a corresponding accessible label. |
| Microphone/camera | Normal versus slashed glyph with an explicit state; outline alone never means off. |
| Checkbox/radio | Empty versus checked/selected, preserving familiar semantics. |
| Back / dismiss / search | `chevron_left` / `close` / `search`; retain their natural line forms. |
| Voice recording | `graphic_eq` waveform; the same contextual action becomes stop while recording and send when a draft is ready. |
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
| Maps | Choose from this account's server-enabled styles, follow appearance or select supported light/dark variants and label fonts; preview before applying. |

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

Use continuous conversation rows with circular avatars, readable names/previews and consistent timestamp/unread placement. Keep the inset floating glass header, search icon and outlined new-message action; no persistent search bar. Hover and focus must remain legible in both themes. Conversation selection uses the same floating header with Close, a selection count, state-aware Pin, Collection and an overflow menu. Keep secondary actions in that menu rather than a horizontally scrolling toolbar. Selected rows use an inset tonal highlight and a check in the avatar position.

Collections remain optional and off by default. Use equal squircle tiles on fixed-width tracks with labels below, respecting the label preference. Long labels must not resize the tiles. Collections scroll away with the conversation rows beneath the floating header; do not introduce a separate clipped list or a fade over the first row at rest.

On mobile, Messages, Calls, Notes and Settings—in that order—use a floating icon-only footer spanning the viewport with 16dp side insets, not a narrow centered capsule. Its default height is 64dp, with 48dp-tall navigation targets. Main and Settings detail headers use a 68dp default height and the 28sp headline-medium title role, growing with text size; do not carry over the larger conversation-header dimensions. Keep the area behind the system gesture indicator transparent. Wide windows retain the collapsible floating directory. The inbox header keeps Search followed by an unfilled New conversation icon; there is no floating draft button. Notes is a main destination with a title and an explicit search action, rather than a permanent search field. It shows only conversations containing notes, with Note to Self first when used; notes pins are independent of inbox pins.

Settings and Calls share this floating main-page chrome and continuous rows. Main-page frost must use the conversation chrome treatment unchanged, including an opaque page background inside the captured backdrop; never blend blurred content over a second sharp copy. Settings sections use spacing and typography, not separate enclosing cards. Use compact profile presentation, quiet outline leading icons and chevrons, and consistent continuous rows. Privacy and Appearance switches toggle from the whole row. Detail content scrolls beneath its floating header. Call history has All and Missed filters, date groups, direction/status and known connected duration, with a quiet trailing time and a separate call action. Tapping a row opens call details with explicit audio, supported video, and message actions. Do not infer missed-call state or duration from incomplete history data; older entries may have unknown media and duration.

### Conversation and composer

Messages are the visual center. Use clear message grouping, modest sender metadata, deliberate spacing, and distinct incoming/outgoing treatment. Preserve readable content width on larger windows.

Emoji-only messages render without a bubble and play [Noto animated emoji](https://googlefonts.github.io/noto-emoji-animation/) (CC BY 4.0). Keep Unicode text as the encrypted message content; this is a client presentation rule, separate from reactions. Bundle assets locally with attribution, honor reduced motion, and use static emoji when animation is unavailable. Choose the asset format during UI implementation.

The composer should feel like one continuous writing surface; do not add a separate filled box behind its text entry. Keep the main text entry and send action clear; disclose formatting, attachments, and structured-content builders progressively. Builders should preserve drafts and return naturally to writing. Avoid duplicated composers and layers of nested sheets.

The composer grows to hold an attachment tray above a persistent text/caption field. Items can be previewed, removed or retried without losing their caption. Cap growth and scroll the tray so input and Send stay reachable above the keyboard. Sending commits attachment and caption together; a failed upload retains both for retry. The attachment is not a replacement for the text field.

Use 8dp composer outer padding and control gaps. Panels share the horizontal inset and 8dp top padding; the composer row supplies the gap below them. Short panels fit their content; only overflowing panels scroll. Image messages preserve their aspect ratio without a surrounding bubble or letterboxing.

Create has no search field or category tabs. Use four-column grids with at most three rows per page and bottom pagination dots when needed. Pages continue the tool list without categories. Contact, Poll, Checklist and Recipe belong on the first Create page, without duplicates in the main attachment grid. Dice, Coin, Cards and Random Number are separate Create tools. Formatting is a compact rich-text toolbar that remains usable with the keyboard. Builders share a title, essential fields and expandable options. Their validated confirmation action uses the main composer button to stage the item; sending remains a separate press after staging. Forms do not show live previews; previews belong to typed SigilText. Each poll/list item gets its own field plus one trailing empty row; insertion animates without stealing focus. Use date/duration/choice controls where appropriate. Builders remain within the continuous composer panel, expanding that same surface when needed. Keyboard/panel switches preserve its height without overshoot.

Structured cards belong to the same visual system as messages. Give them clear titles, content hierarchy, and obvious actions. Cap large inline content and provide expanded views, following SigilText.md. Composite-looking AI mockups do not authorize unsupported nested structured blocks or change message semantics.

Cards share a small type indicator, meaningful title, content and relevant actions. Notes read like writing; polls expose choices; checklists have directly usable rows; charts/tables/diagrams expand for detail. Graphical builders and typed syntax share canonical validation. Typed SigilText previews cards above the composer; static formatting previews inline, while animated text has an indicator without playing. Randomizers preview their objects without choosing a result; on send, dice roll and coins flip from the preview into the timeline. Keep their tray open until they leave, then collapse it. Cards stay at their preview position while the tray lowers beneath them, then shuffle and settle the selected card into its message; do not fly the cards upward. Show coin height and edge rotation, keep motion behind the header and clear of input controls, and keep captions in compact separate bubbles. Hidden content must remain hidden in previews, accessibility and expanded views.

### Media, captions and viewers

The attachment/card is the message surface; do not add another bubble around it. Preserve sender direction, grouping, reactions, pins and tap-for-details receipts. Captions attach to the same surface below its content, with readable backing rather than text over a busy image.

| Content | Timeline / expanded view |
| --- | --- |
| Images | Rounded aspect-preserving preview with no type label / gesture zoom, pan and full-resolution inspection. |
| GIFs | Small GIF chip on the picture; automatic looping under the motion preference / same looping presentation with no play/pause controls. |
| Video | Play button over the picture, no redundant type/duration label / playback, seeking and relevant audio controls. Do not autoplay sound. |
| Voice memos | Integrated play/pause, seekable waveform, elapsed/total time and optional caption / larger waveform, seeking and speed control. |
| MP3 and other supported audio | Compact player with title and available metadata / larger player with seeking and speed controls. Keep recording-specific affordances out of music/file players. |
| Link previews | One coherent title/domain/description/image card, integrated with accompanying text / preview details and explicit Open link. Viewing details must not silently launch a browser or fetch new remote content. |
| PDFs, documents and other files | Type, name, size and supported safe preview / larger supported viewer, otherwise file details with explicit Save/Open externally. Never imply an unsupported format can render. |
| Code | Google Sans Code, preserved whitespace, theme-aware highlighting when recognized, language label and Copy / expanded selectable code with wrap toggle and horizontal scrolling. Unknown languages stay readable plain code. |

All message attachments have an expanded destination, even when it can only show file details. Expansion preserves playback position and timeline return position. Authored captions remain in the timeline; image/video/GIF viewers omit caption blocks. Small media must not stretch; inline crops must not alter original content. Viewers adapt to large windows and keyboard navigation. Loading, transfer progress, cancellation, retry, missing/deleted content and unavailable preview use shared states. Preview and expansion obey view-once, expiry and existing save restrictions.

Expanded visual media uses an immersive rounded presentation over the dimmed conversation, with a floating glass header for Close, sender/time and available save/menu actions, and a compact reaction bar. Use actual message reactions. Images use pinch, pan and double-tap, without redundant Fit/Zoom/Back controls; retain keyboard/accessibility equivalents. GIFs have no playback toolbar.

Camera opens within the attachment surface, with usable preview, shutter, camera switch and close/back controls clear of system insets. After capture, show Retake/Use, then stage the result with an editable caption; capture never sends automatically. Only the necessary permission is requested when capture starts.

Voice recording uses the same growing composer: recording indicator, responsive waveform, timer, stop/pause/resume and discard where supported. Stopping produces an editable draft with playback and a caption field, then explicit airplane Send. Switching panels does not silently discard or send a recording. The draft player and sent player share their anatomy and visual transition; errors preserve usable recorded content.

The writing field and attachment toggle sit directly on the continuous composer surface. The toggle has a full touch target and transient interaction feedback, without a permanently filled tile behind its plus/close icon.

### Reply gestures

Dragging a message away from its anchored screen edge reveals the reply icon and “Reply”; dragging toward that edge reveals the thread icon and “Reply in thread.” The revealed indicator follows drag progress, reaches a clear armed state at the threshold, and triggers on release with optional haptic feedback. Reversing/cancelling restores the message without action. Captions and media participate in the same gesture; waveform seeking, text selection, map panning and viewer gestures take priority within their controls. Both actions remain in the message menu for keyboard and accessibility use.

### Location

The existing attachment grid directly offers **One-time location**, **Real-time location**, and **Drop a pin**. Each opens its picker inside the growing composer, above the persistent writing row on the same continuous surface. Back returns to the grid. Do not create a separate attachment sheet, intermediate choice screen or full-screen picking flow. The expanded map is for tapping an already-shared timeline card.

Each picker has one device-location/recenter control within the map, with no duplicate text button. Drop a pin uses a small “Drop a pin” chip inside the map preview; no instruction paragraph or manual coordinate fields. Recenter moves the camera without selecting or erasing a pin. Live selection adds duration choices. Use the existing composer action with an airplane icon when it sends the location directly; a checkmark must not disguise a send. Preserve the draft and permission/error handling. Ask for location permission when locating the device; opening a picker never starts sharing.

Shared cards are map surfaces with the ordinary message receipt below. Do not add place/accuracy/sample-time/last-seen/waiting-for-update metadata blocks. Real-time cards show a remaining-time countdown on the map. Keep authored captions as message content. Current/live markers use the person's avatar or initials; dropped pins use a place marker.

Live rings reflect fresh active updates only. Stale updates stop the rings without adding diagnostic text; never invent positions or precision. Expiry or Stop sharing ends the rings and countdown and leaves a concise ended state. Reduced motion uses a static indicator. Retain state needed for correctness internally without exposing it as routine decoration.

The expanded shared-map viewer uses floating glass chrome, a dominant map, concise identity and live remaining-time/Stop sharing controls. Recenter and source attribution stay available; map style follows the viewer's preferences. No coordinate forms or sample/accuracy diagnostics. Permission failures and unavailable maps retain concise actionable recovery. Optional place search requires a geocoder and explicit provider disclosure; map tiles alone are not a search index.

Automatic address previews detect likely addresses on the client before encryption, resolve them and show a removable map card with the address beneath it. Preserve surrounding text as a caption; ambiguous matches require selection and failed lookup preserves plain text. Send resolved coordinates with the encrypted message so recipients do not repeat the search. The setting explains that the lookup service receives the candidate address, not the entire draft.

Admin → Maps offers existing mounted archives or supported regional downloads, with size, progress, cancellation and update controls. Self-hosted address/business lookup uses a compatible local search engine and index: import a supported prebuilt index when available, otherwise build from source data. Dataset downloads/updates contact their source; local queries require no third-party requests or silent external fallback. Server-hosted lookup still reveals the query to that server.

Browser maps currently render the installed Protomaps vector schema with Sigil neutral light/dark colors and the selected Newsreader or Google Sans Flex font. They use authenticated local tiles, bounded geometry and one active Canvas2D map. This is a limited rendering profile, not full MapLibre style compatibility; unsupported schemas show an error. Inline browser location cards open the interactive map rather than creating a map renderer for every message.

Admins enable a catalog of map styles for their users. Each style family (Sigil, Ancient, Cyber, etc.) has a tested variant for each supported tile schema/version, including its fonts, sprites and attribution. Expose only compatible installed combinations; vector styles cannot recolor raster tiles. Style selection is private and scoped to the active account/server. Different servers may offer different catalogs; a removed/unavailable choice visibly falls back to an available default. Federated locations render using the recipient's catalog, not the sender's styling. Automatic installation and additional style families remain implementation work.

### Motion

On wide screens, use a centered floating conversation beside an expandable floating directory; its collapse chevron sits beside Sigil. On mobile keep the avatar/status dot in conversation headers. Threads, Notes and Pins remain in the conversation menu; notes use a Keep-style grid and individual threads reuse the timeline composer. Main tabs move according to their order; header items change within the same header. Opening a conversation first raises its timeline from below, then brings its distinct floating header down from above and composer up from below. Reverse on Back; do not morph the inbox header into the conversation header or restart transitions during sync refresh. Keep the gesture area transparent and give the status area above the floating header an opaque background through the system glyph band, followed by a soft translucent fade that leaves the first item unobscured at rest. Appearance previews extend their background behind both floating surfaces.

Use shared timing bands: roughly 120–160 ms for feedback and 180–240 ms for inline expansion; preserve the accepted navigation choreography. Inner composer pages enter from the right and reverse on Back. Retarget interrupted motion from its current position; never queue stale toggle animations. Layout, selection and sending must not wait for decorative motion.

The conversation backdrop, including wallpaper, travels with its timeline on entry and exit. Keep the incoming inbox behind the departing conversation and reveal its header only after the conversation leaves; do not insert a temporary footer surface. Camera panels remain below the settled conversation header, including while the keyboard is open.

SigilText content effects follow its existing one-shot/new-message, settled-history and explicit-replay rules. Reserve final text layout before playback: wave/typewriter effects cannot resize bubbles or push the timeline repeatedly. Animate shaped text without splitting joined scripts or emoji. Selection, copying and accessibility retain canonical text; spoilers remain protected. New rows and option sections expand smoothly without moving the active input out of view. Reduced motion presents settled content; offscreen content does not animate.

### Calls

The Calls header includes an outlined plus action for New call. Selecting a contact in its picker does not place a call; Audio call and Video call are explicit actions, available according to platform support and the current contact/call state.

Use the dark call reference for participant layout, rounded video tiles, restrained active-speaker indication, and a clear control group. Mute, camera, speaker, sharing, and ending a call need recognizable states and labels. Keep the destructive end-call action visually distinct. Call presentation may use a dedicated dark surface where appropriate without changing the user's global theme preference.

### Administration

Apply the same typography, spacing, and surface language to setup, health, storage, backups, users, and updates. Lead with understandable state and the next useful action. Operational warnings must remain prominent enough to act on within the otherwise quiet interface.

The overview is a visual operational dashboard: real queue trends, storage composition, service health and actionable failures, with labeled units, observation windows, refresh time and accessible numeric equivalents. Charts link to relevant settings or diagnostics. Missing history stays empty rather than inventing a trend. Keep content private: no message plaintext, contact graphs or inferred private membership. Administrators enable supported push providers; devices default to Google delivery when available, with provider selection under advanced notification settings.

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
