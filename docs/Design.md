# Sigil design direction

Status: agreed visual direction and theming requirements; implementation details and logo remain provisional.

## Intent

**Modern correspondence, with the intimacy of a notebook.**

Sigil takes inspiration from the care of writing with a fountain pen in a well-made notebook: deliberate typography, precise details, quiet surfaces, and room to think. The user's Leuchtturm1917 notebook and LAMY Safari are experiential references, not products to copy or brands to reproduce.

The interface should feel personal, calm, tactile through its proportions, and contemporary. Express this through typography, spacing, shapes, and tonal contrast. Avoid simulated paper textures, handwriting fonts, ornamental antique controls, and decorative effects that compete with messages.

This document records the selected direction following the feasibility discussion. It supersedes earlier visual-concept alternatives in plan.md. Compose Multiplatform is the leading provisional UI candidate; reliable web accessibility and Mac/iPhone validation remain open. The design is not tied to Material component styling.

## Visual language

- Neutral ink-and-paper palette: whites, off-whites, grays, charcoal, and black. The default has no sage-green or other chromatic brand accent.
- Deliberate margins, readable line lengths, and consistent vertical rhythm.
- Fine dividers, subtle borders, and restrained tonal separation. Shadows are occasional aids to hierarchy.
- Soft corners balanced with precise alignment. Avoid putting every row or control inside another rounded card.
- Typography and content establish hierarchy before decoration does.
- Motion is brief and purposeful: acknowledge an action or explain a transition.

Light mode uses paper-like neutral surfaces and dark ink. Dark mode uses charcoal/black surfaces and light ink, with its own contrast relationships. Neither mode requires a textured background. The identity must survive changing the accent, font, or conversation background.

## Typography and icons

The global typography setting has two choices:

| Choice | Scope |
| --- | --- |
| Newsreader — default | All ordinary application text, including messages, headings, navigation, buttons, settings, timestamps, and administrative screens. |
| Google Sans Flex | Replaces Newsreader throughout the same roles. |

Do not use a fixed serif-heading/sans-control split. Google Sans Code remains the font for code blocks and explicitly monospaced content under either choice. Material Symbols supplies the icons; Rounded is the proposed style, pending visual review.

Use size, weight, and spacing to establish hierarchy within the selected family. Bundle fonts locally. Provide appropriate fallback fonts for scripts and glyphs missing from the selected family, while preserving the user's choice wherever supported. Emoji retain appropriate emoji rendering.

Verify both choices in compact controls, long localized labels, numeric data, and enlarged text. Layout must adapt rather than clip labels or depend on one font's measurements.

## Global appearance

Global appearance establishes the inherited theme for the app and all conversations without overrides.

- **Palette source:** Sigil neutral default, a user-selected custom accent, or Android dynamic colors where available.
- **Accent color:** explicitly customizable in the global theme settings. A custom accent produces coordinated interactive, selection, focus, and supporting surface colors.
- **Appearance mode:** light, dark, or follow system, independent of palette source. Follow system is the proposed initial setting.
- **Typography:** Newsreader or Google Sans Flex throughout, with the code-font exception above.

The neutral default still has accent roles; their values are neutral. Selecting Android dynamic colors is optional and does not change Sigil's typography, geometry, or component design. Selecting a custom accent replaces the dynamic palette source rather than creating competing accent settings.

Offer a preview and a clear reset to the Sigil default. Avoid making users configure dozens of individual color roles to obtain a complete theme.

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

Recommended, not yet decided: synchronize private theme preferences and background assets, encrypted, across the user's linked devices. Image framing may need a device-specific crop. This must never turn viewer-private customization into a theme shared with other participants.

## Theme implementation contract

Use shared semantic color roles rather than literal colors scattered through components. The same palette derivation must produce consistent results across clients. Retain only one active theme implementation as the design evolves.

Derive useful tone relationships, not a blanket tint over the screen. Incoming/outgoing content, primary/secondary actions, and foreground/background must remain distinguishable for very light, very dark, saturated, and neutral accents.

Theme customization must preserve error, warning, destructive-action, and security-state meaning. These states may retain functional colors in the otherwise neutral default and always need a non-color cue. Photographs, video, avatars, and authored media are not recolored. Special rendering requirements, such as a scannable QR code's light backing, take precedence over decorative theming.

SigilText named colors resolve against the effective conversation theme as required by SigilText.md. Redaction, hidden poll results, and other privacy semantics must remain correct in every visual and accessible representation.

## Layout and principal surfaces

### Inbox

Use the cleaner reference inbox as the starting point: clear conversation rows, circular avatars, readable names and previews, restrained separators, and consistent timestamp/unread placement. Keep search and new-message actions discoverable without an oversized header. Reduce competing pins, badges, and status symbols.

Collections remain optional and off by default under the product plan. Enabling them must not make navigation excessively tall. The AI references do not settle the final bottom-navigation structure. Notes remain a per-conversation view, separate from pinned messages and Note to Self.

### Conversation and composer

Messages are the visual center. Use clear message grouping, modest sender metadata, deliberate spacing, and distinct incoming/outgoing treatment. Preserve readable content width on larger windows.

Emoji-only messages render without a bubble and play [Noto animated emoji](https://googlefonts.github.io/noto-emoji-animation/) (CC BY 4.0). Keep Unicode text as the encrypted message content; this is a client presentation rule, separate from reactions. Bundle assets locally with attribution, honor reduced motion, and use static emoji when animation is unavailable. Choose the asset format during UI implementation.

The composer should feel like a writing space. Keep the main text entry and send action clear; disclose formatting, attachments, and structured-content builders progressively. Builders should preserve drafts and return naturally to writing. Avoid duplicated composers and layers of nested sheets.

Structured cards belong to the same visual system as messages. Give them clear titles, content hierarchy, and obvious actions. Cap large inline content and provide expanded views, following SigilText.md. Composite-looking AI mockups do not authorize unsupported nested structured blocks or change message semantics.

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

The seven supplied ChatGPT images dated September 5, 2026 at 09:38:57, 09:38:49, 09:37:08, 09:37:01, 09:36:46, 09:36:26, and 09:02:17 are mood and layout references. Their sage palette, font-role split, literal labels, navigation inconsistencies, and AI artifacts are not requirements. The explicit decisions above take precedence.

The supplied Gemini logo is exploratory, not final. Its circular seal and intertwined S fit the correspondence direction. Further work should assess the eye-like association and small-size legibility, including whether a simpler compact mark is necessary. Do not treat the raster reference as a finished production asset.

## Review before production styling is accepted

Review synthetic inbox, conversation, structured builder, call, and administrative screens in both font choices and both appearance modes. Include a neutral default, a custom global accent, Android dynamic colors, and independently themed conversations with solid, gradient, and image backgrounds.

Verify inheritance and resets, viewer-private ownership, light/dark transitions, legibility under extreme accent choices, large text, RTL, keyboard focus, and screen-reader navigation. Validate platform behavior on each actual target; visual approval does not resolve the outstanding framework accessibility or Apple-platform checks.

Still to decide: final logo, exact default neutral values and component measurements, final navigation, private theme synchronization, and the scope of advanced color controls beyond accent-derived palettes.
