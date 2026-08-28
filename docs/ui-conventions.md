# Sigil UI conventions

How this app is built, and the mistakes already paid for. Read before designing
anything new. Cross-platform rules live in [`portability.md`](portability.md);
this is about look, structure and the traps.

## Tokens, never numbers

**1566 `Style.space()` calls, 765 `Style.font.*` references, 20 bare numbers in
19,000 lines.** Keep that ratio.

- Sizes: `Style.space(px)` (integer, snapped) or `Style.spaceReal(px)` for
  fractional geometry. Both scale with the host's spacing/font scale, which is
  the entire DPI story — a hardcoded number is wrong on every other display.
- Text: `Style.font.family` and `caption | bodySmall | body | subtitle | title |
  heading`. Never a raw `pixelSize`.
- Colour: `Color.accent | background | foreground | muted | urgent`,
  `Color.menu.{text,background,border}`, `Color.popups.{text,background,border}`,
  and `Util.alpha(c, a)` for transparency. Never a hex literal in a component.

The two legitimate bare numbers in the tree are inside `MapView.qml`, where a
marker's pixel geometry is fixed by the map's own coordinate space.

## Theme derivation

A chat theme is **one accent colour**; every other tone is mixed from it. Sibling
pages must derive tones *identically*, or opening Threads from a themed room
drops you onto a differently-coloured page. The canonical derivation is in
`ChatPage.qml` and is copied verbatim into `ThreadsPage.qml` and `PinsPage.qml`:

```qml
readonly property bool themed: (chatTheme.accent || "") !== ""
readonly property color accC: themed ? Qt.color(chatTheme.accent) : Color.accent
readonly property real tintAmt: 0.35
readonly property color surfaceC: themed ? mixc(Qt.lighter(Color.menu.background, 1.35), accC, tintAmt)
                                         : Color.popups.background
readonly property color chromeC: themed ? surfaceC : Qt.lighter(Color.menu.background, 1.35)
readonly property color convoC: /* menu.background darkened 1.35, then 18% accent */
```

Guessing at `chatTheme.background` instead produces a page that does not match
the room it was opened from.

## Page anatomy

- **Header**: 52 for one line, 56 for two. Back glyph is `󰁍` in a
  `PanelActionButton`, left margin `Style.space(6)`. Title at `Style.font.heading`
  bold; subtitle at `caption` in `Util.alpha(fg, 0.55)`.
- **Counts go in the subtitle**, never in a trailing header slot — "N members",
  "Project Room · 3 threads". A subtitle that appears only once a count loads makes
  the title jump; word it unconditionally.
- **Conversation ground**: a container whose top corners are cut away
  (`topLeftRadius`/`topRightRadius` `Style.space(24)`), revealing the header tone
  behind. Sibling pages off the same menu must not have opposite grounds.
- **Icon actions** use `PanelActionButton`. Empty states say *which* empty they
  are — "No threads yet" is a fact about the room, "Looking for threads…" is a
  fact about us; never show one while the other is true.

## Comments

Comments explain **why**, not what — and especially why something is *not* done
the obvious way. Most comments in this tree are load-bearing: they are the only
record that an obvious-looking simplification was tried and measured and was
worse. If you find one, do not "clean it up" without reproducing the measurement.

---

# Pitfalls already paid for

## Layout and bindings

- **Anchoring a child inside a `Row`/`Column`** disables the positioner. Qt logs
  "Row will not function" and the layout silently dies. Put anchored children in
  a plain `Item`.
- **Sizing a container from an anchored child is a binding loop.** Use the
  child's `implicitWidth` (unwrapped, independent of assigned width), not its
  `width`.
- **A binding read mid-propagation sees stale siblings.** Rebuilding on
  `onLatChanged` read `lon` before its binding re-evaluated and put the map in
  the Gulf of Guinea. Rebuild through `Qt.callLater`.
- **`Loader.setSource`'s property map is a one-time snapshot.** Anything that
  changes later must be re-bound in `onLoaded` with `Qt.binding`.
- **Rebuild a `Loader` only on properties that must exist before construction.**
  Rebuilding on any changing property tore down the map every few seconds during
  a live share, taking the in-flight drag with it.

## Delegates and lists

- **`visible: false` does not stop a `Repeater` instantiating its delegate.**
  Gate the *model* (`model: cond ? realModel : null`) or the `Loader.active`.
- **Model roles shadow same-named properties on the delegate.** `participant` /
  `track` / `isLocal` as roles silently shadowed `ParticipantTile`'s own
  properties and every tile drew "?". Prefix roles (`pPart`, `pTrack`, `pLocal`).
- **`reuseItems` is off, permanently.** It leaked stale content across rooms and
  stale geometry. Gating body `Loader`s was not enough — sender name, avatar and
  text are plain bindings that survive reuse.
- **Zero-height delegates wreck `ListView`'s size estimate**, which is what makes
  scrolling jump. If an item can be hidden, give it zero `implicitHeight`
  deliberately and expect the estimate to suffer.
- **Filter at the source.** Diff ops are index-based, so filtering downstream
  desynchronises the engine's view from the QML model. The timeline filter lives
  in the `TimelineBuilder` for exactly this reason — and see
  `core/src/timeline/mod.rs::keeps_state`, which twice ate events it shouldn't
  have by judging them on namespace instead of on whether a person sent them.

## Text

- **`TextEdit` with `TextFormat.RichText` reports `contentWidth` as the *layout*
  width**, not the width of the text. Sizing a bubble from it makes every bubble
  full-width.
- `RichText.text` should be empty when the body is not rich; binding it anyway
  costs a full document parse per delegate.

## Shared page furniture

Full-screen pages are built from five components rather than per-page copies.
Each of them exists because the hand-written copies drifted:

- **`PageHolder.qml`** — the slide-in layer: frame-driven animation, an opaque
  ground so the pages stacked underneath never show through, and an event sink
  so clicks and wheel never reach them either. Forty lines per page before.
  Frame-driven, not a timed `Behavior`: opening a room stalls the UI thread
  while its timeline builds, and a time-based animation spends that stall and
  arrives already finished — the page appeared with no slide at all.
- **`SettingsHeader/Group/Row.qml`** — back, title, one trailing action; a
  titled band; and one line with a leading icon at 22, text at 56 and trailing
  furniture inset 20. Those three numbers are the reason this is a component:
  pages one tap apart visibly failed to line up when each drew its own rows.
- **`OverflowMenu.qml`** — the ⋮ drop from a header's top-right, scrim
  included. Every copy that owned its own scrim eventually grew a way to leave
  the menu open with nothing to dismiss it.
- **`ChoiceSheet.qml`** — one question, a handful of answers, from the bottom.
  Used instead of a dropdown, which would have to position against a row
  inside a scrolling list and can land offscreen on touch.

**A space is a room.** `m.space` in the creation content is the only
difference, so `room.settings`, `room.setSettings`, `room.setAvatar` and
`room.setPowerLevel` all take a plain `roomId`, and one set of settings pages
serves both. The pages are told which page to go back to (`settingsReturn`,
`membersReturn`) rather than assuming one — that is the only thing that
differs between the two entry points.

## Icons and centring

- **Do not "ink-centre" Material Symbols.** Every glyph in the set is drawn on a
  960-unit em whose ascent/descent are chosen so that plain vertical centring of
  the line box lands the icon dead centre. Measured from the outlines at 14 px in
  a 22 px `PanelActionButton`: `back`, `phone`, `videoOn`, `people`, `personAdd`,
  `close`, `more`, `send`, `attach` all give an ink centre of exactly 11.00 —
  the button's centre. `search`, `mic` and `check` are within 0.3 px, and that
  residual is deliberate optical balance in the icon, not an error.
- **`TextMetrics.tightBoundingRect` is the *outline* bbox, not the rendered
  ink**, and it is already centred (see above). Subtracting it from an
  already-centred layout moves the icon *off* centre — measured at `dy -2.50`
  against `dy -0.50` for plain alignment. This cost several rounds of "the icons
  still aren't centred". Plain `anchors.centerIn` is correct; `IconLabel` does
  exactly that.
- **Emoji are the opposite case** and *do* need ink-centring: they carry
  variation selectors and surrogate pairs, so the advance width exceeds the
  visible glyph and line-box centring leaves them left of centre (measured
  −5.5 px in the reaction row). `MessageSheet`, `ImageViewer` and `CallPage`
  centre them on `tightBoundingRect` for that reason.
- **Never draw an `Icons.*` codepoint in a non-icon font.** They are PUA
  codepoints, so a wrong family does not give tofu — it gives *a different
  icon*. `Icons.attach` (U+E226) is `fae-moon_cloud` in JetBrainsMono Nerd Font;
  `Icons.phone` (U+E0B0) is a Powerline divider; `Icons.search` (U+E8B6) is the
  Supabase logo. `PanelActionButton` defaults `fontFamily` to `Style.font.family`
  (the *UI* font), so every call site must pass `fontFamily: Fonts.icon`.
  Prefer `IconLabel`, which cannot get this wrong.
- **Buttons carry no internal padding.** `PanelActionButton` is a bare
  `size × size` surface with `anchors.centerIn` on the label. If an icon looks
  off-centre, the cause is the font family or the surrounding layout — measure
  before adjusting the glyph.

## Input — see also `portability.md`

- **Never build interaction on a contested exclusive pointer grab.** This is the
  big one. A `DragHandler` with `grabPermissions` tuned to steal from items will
  fight every neighbouring region and die whenever anything revokes the grab.
  Use a passive `PointHandler`, or give handlers their own geometry.
- **A handler that can steal from items will steal from the item you put there
  to block it.** The map's sheet had a MouseArea specifically to stop drags; the
  map's `CanTakeOverFromItems` defeated it by design. Regions are separated by
  *geometry*, not by stacking another blocker on top.
- **A tap-away `MouseArea` inside a control cluster** makes every control need
  two taps — the first is eaten dismissing. Put it at page level.
- **Refuse actions on pages that are not visible.** A right-click and a left-
  click together can navigate away *and* request a menu, with the menu landing
  after the navigation. Guard on the page's own `visibleToUser`.
- **Do not compensate `contentY` when `originY` changes.** Three attempts, all
  measured worse — 297ms frames from re-entrant layout.

## Effects and layers

- **A `MapQuickItem`'s `sourceItem` is rendered to a texture**, and a
  `ClippingRectangle`'s custom node does not survive that. Use a layer mask.
- **MapLibre draws through its own scene-graph node** and ignores a parent's
  rounded clip. Corners come from a `MultiEffect` layer mask, and page
  transitions must scale the *holder*, not anything inside it.
- **Check `z` before adding it.** A `floatLayer` at `z: 30` painted above the
  controls and sheet that were both at `z: 0`.

## Instrumentation

Three separate times this session an instrument manufactured its own conclusion.
Do not trust a measurement until you have asked how it could lie.

- **A test hook that silently does nothing looks like a clean result.**
  `sigilui mapInput` called a forwarder that did not exist, threw a TypeError and
  returned empty for an entire debugging session. Verify a hook returns real data
  before reasoning from its silence.
- **A `Connections` with a null `target` emits nothing**, which reads exactly like
  "the event never happened". `Window.window` is null at `Component.onCompleted`.
  Log whether the watcher attached.
- **`point.position` is relative to the item the event is being delivered to**,
  which differs between grab and cancel. Comparing them invented a 363-pixel jump
  and a boundary that did not exist. Use `scenePosition` when comparing.
- **A `FrameAnimation` only ticks when something renders**, so "milliseconds
  since the last frame" counts idle time as a stall. Measure *input latency* —
  from the pointer moving to the pixels landing — not frame gaps.
- Sanity-check against a second signal. Event counts disproved the stall theory:
  a live pointer at ~700 events/s would have logged tens of thousands of events
  across those gaps, and the trace held four thousand in total.
