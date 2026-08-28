import QtQuick
import QtQuick.Effects
import qs.Commons
import qs.Ui
import "../components"

// Per-character renderer for inline text effects. Colour names resolve against
// the live palette at render time, never to hex at parse time; animation stops
// under reduced motion or off-screen.
Item {
  id: root
  /// The plain body the effect offsets index into.
  property string text: ""
  /// `[{start, end, color, animation, spoiler, underline, mark, mono, bold,
  /// italic, strike, size}]` from the engine.
  property var effects: []
  property color fg: Color.menu.text
  property real pixelSize: Style.font.body
  property var svc: null
  /// Which message this is, so a revealed spoiler stays revealed.
  property string eventId: ""
  /// Off while the bubble is outside the viewport; per-glyph animation is costly.
  property bool active: true

  /// Available width, set by the caller. Deriving it from this item's own width
  /// is a binding loop: Flow width <- item width <- Flow implicitWidth.
  property real maxWidth: 400

  /// System reduced-motion preference, with a Sigil-level override.
  readonly property bool motionOk: !(root.svc && root.svc.reduceMotion)

  // Measured off-layout so the width source does not depend on the layout.
  Text {
    id: metrics
    visible: false
    text: root.text
    wrapMode: Text.Wrap
    width: root.maxWidth
    font.family: Fonts.ui
    font.pixelSize: root.pixelSize
  }

  implicitWidth: Math.min(root.maxWidth, Math.ceil(metrics.implicitWidth) + 2)
  implicitHeight: flow.implicitHeight

  property bool hovered: false
  HoverHandler { onHoveredChanged: root.hovered = hovered }

  // Colours and size multipliers arrive resolved from the engine — see
  // core/src/timeline/palette.rs. Resolving them here would drift from every
  // other Sigil client.

  /// The engine ships a colour for each ground; a light foreground means we are
  /// drawing on a dark one.
  readonly property bool darkGround:
    (0.299 * root.fg.r + 0.587 * root.fg.g + 0.114 * root.fg.b) > 0.5

  /// One `{dark, light}` pair from the engine, or the surrounding text colour.
  function hue(pair) {
    if (!pair) return root.fg
    return Qt.color(root.darkGround ? pair.dark : pair.light)
  }

  function rainbowAt(frac, sat, lum) {
    return Qt.hsla(Math.max(0, Math.min(0.999, frac)), sat || 0.62, lum || 0.62, 1)
  }

  /// `rgb` is the engine's resolved stop list; interpolation is per-frame, so it
  /// stays here rather than on the wire.
  function gradientAt(rgb, frac) {
    if (!rgb || rgb.length === 0) return root.fg
    if (rgb.length === 1) return root.hue(rgb[0])
    var f = Math.max(0, Math.min(1, frac)) * (rgb.length - 1)
    var i = Math.min(rgb.length - 2, Math.floor(f))
    var a = root.hue(rgb[i]), b = root.hue(rgb[i + 1]), t = f - i
    return Qt.rgba(a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t, 1)
  }

  /// `small1..3` / `big1..3` as a multiplier, clamped to 0.7-1.6.
  /// The engine ships the multiplier with the span.
  function sizeScale(scale) { return scale > 0 ? scale : 1.0 }

  // Runs

  readonly property var glyphs: {
    var chars = root.text.split("")
    var out = []
    for (var i = 0; i < chars.length; i++) {
      var colour = null, anim = "", from = i, len = 1
      var spoiler = false, underline = false, mark = false, mono = false, markRgb = ""
      var bold = false, italic = false, strike = false, size = 0
      for (var e = 0; e < root.effects.length; e++) {
        var fx = root.effects[e]
        if (i < fx.start || i >= fx.end) continue
        if (fx.spoiler) spoiler = true
        if (fx.underline) underline = true
        if (fx.mark) { mark = true; markRgb = fx.markRgb || markRgb }
        if (fx.mono) mono = true
        if (fx.bold) bold = true
        if (fx.italic) italic = true
        if (fx.strike) strike = true
        if (fx.sizeScale) size = fx.sizeScale
        if (fx.color) { colour = fx.color; from = fx.start; len = Math.max(1, fx.end - fx.start) }
        if (fx.animation) anim = fx.animation
      }
      out.push({
        ch: chars[i], colour: colour,
        frac: len > 1 ? (i - from) / (len - 1) : 0,
        anim: anim, spoiler: spoiler, underline: underline, mark: mark, markRgb: markRgb,
        mono: mono, bold: bold, italic: italic, strike: strike, size: size,
        index: i
      })
    }
    // Each maximal `flip` run has its characters reversed in place; with the
    // per-glyph 180 rotation that turns the whole phrase over.
    var i0 = 0
    while (i0 < out.length) {
      if (out[i0].anim !== "flip") { i0++; continue }
      var i1 = i0
      while (i1 + 1 < out.length && out[i1 + 1].anim === "flip") i1++
      for (var a = i0, b = i1; a < b; a++, b--) {
        var t = out[a].ch; out[a].ch = out[b].ch; out[b].ch = t
      }
      i0 = i1 + 1
    }
    return out
  }

  /// Grouped into words: a Flow given raw characters breaks mid-word.
  readonly property var words: {
    var out = [], cur = []
    for (var i = 0; i < root.glyphs.length; i++) {
      var g = root.glyphs[i]
      if (g.ch === "\n") {
        if (cur.length > 0) { out.push({ glyphs: cur, brk: true }); cur = [] }
        else out.push({ glyphs: [], brk: true })
        continue
      }
      cur.push(g)
      if (g.ch === " ") { out.push({ glyphs: cur, brk: false }); cur = [] }
    }
    if (cur.length > 0) out.push({ glyphs: cur, brk: false })
    return out
  }

  // Spoilers

  readonly property bool hasSpoiler: root.effects.some(function (e) { return e.spoiler })
  /// Revealed spoilers persist for the session across room switches.
  readonly property bool alreadyRead: !!(root.svc && root.eventId !== ""
                                         && root.svc.revealedSpoilers[root.eventId])
  property bool spoiled: true
  property real reveal: 0
  Behavior on reveal {
    NumberAnimation { duration: root.motionOk ? 620 : 0; easing.type: Easing.OutCubic }
  }
  Component.onCompleted: if (root.alreadyRead) { root.spoiled = false; root.reveal = 1 }

  readonly property real spread: Math.max(4, Math.min(14, root.glyphs.length * 0.35))
  /// 0 covered, 1 clear, wiping left to right.
  function revealAt(i) {
    if (!root.spoiled && root.reveal >= 1) return 1
    var front = root.reveal * (root.glyphs.length + root.spread)
    return Math.max(0, Math.min(1, (front - i) / root.spread))
  }
  function revealAll() {
    root.spoiled = false
    root.reveal = 1
    if (root.svc && root.eventId !== "") root.svc.rememberSpoiler(root.eventId)
  }

  property int typed: 0
  readonly property bool hasTypewriter:
    root.effects.some(function (e) { return e.animation === "typewriter" })
  Timer {
    id: typeTimer
    interval: 45
    repeat: true
    running: root.active && root.motionOk && root.hasTypewriter && root.typed <= root.glyphs.length
    onTriggered: root.typed++
  }
  onTextChanged: root.typed = 0

  Flow {
    id: flow
    width: root.width > 0 ? root.width : root.maxWidth
    spacing: 0

    Repeater {
      model: root.words

      delegate: Item {
        id: wordCell
        required property var modelData
        readonly property bool isBreak: modelData.brk && modelData.glyphs.length === 0
        width: isBreak ? flow.width : wordRow.implicitWidth
        height: isBreak ? 0 : wordRow.implicitHeight

        Row {
          id: wordRow
          spacing: 0

          Repeater {
            model: wordCell.modelData.glyphs

            delegate: Item {
              id: cell
              required property var modelData
              readonly property bool anim: root.active && root.motionOk && modelData.anim !== ""
              /// 1 when this character is fully readable.
              readonly property real shown: modelData.spoiler ? root.revealAt(modelData.index) : 1
              implicitWidth: glyph.implicitWidth
              implicitHeight: glyph.implicitHeight

              // `mark` is its own rectangle, not a text background, so runs butt into one band.
              Rectangle {
                anchors.fill: parent
                anchors.topMargin: -1
                anchors.bottomMargin: -1
                visible: cell.modelData.mark
                color: {
                  var c = cell.modelData.colour
                  var base = root.hue(cell.modelData.markRgb)
                  return Util.alpha(base, 0.32)
                }
              }

              Text {
                id: glyph
                text: cell.modelData.ch === " " ? " " : cell.modelData.ch
                color: {
                  var c = cell.modelData.colour
                  // `mark` colours the highlight, not the ink.
                  if (!c || cell.modelData.mark) return root.fg
                  if (c.type === "rainbow")
                    return root.rainbowAt(cell.modelData.index / Math.max(1, root.glyphs.length - 1), c.saturation, c.lightness)
                  if (c.type === "gradient") return root.gradientAt(c.rgb, cell.modelData.frac)
                  return root.hue(c.rgb)
                }
                font.family: cell.modelData.mono ? Fonts.mono : Fonts.ui
                font.pixelSize: Math.round(root.pixelSize * root.sizeScale(cell.modelData.size))
                font.weight: cell.modelData.bold ? Font.ExtraBold : Font.Normal
                font.italic: cell.modelData.italic
                font.underline: cell.modelData.underline
                font.strikeout: cell.modelData.strike

                readonly property int phase: cell.modelData.index * 90

                // flip — glyph rotated 180 and the run reversed (see `glyphs`).
                rotation: cell.modelData.anim === "flip" ? 180 : 0

                opacity: {
                  var t = 1
                  if (cell.modelData.anim === "typewriter" && root.motionOk)
                    t = root.typed > cell.modelData.index ? 1 : 0
                  return t * cell.shown
                }

                SequentialAnimation on x {
                  running: cell.anim && cell.modelData.anim === "shake"
                  loops: Animation.Infinite
                  PauseAnimation { duration: glyph.phase % 160 }
                  NumberAnimation { to: 0.8; duration: 80 }
                  NumberAnimation { to: -0.8; duration: 80 }
                  NumberAnimation { to: 0; duration: 80 }
                }

                SequentialAnimation on y {
                  running: cell.anim && cell.modelData.anim === "wave"
                  loops: Animation.Infinite
                  PauseAnimation { duration: glyph.phase }
                  NumberAnimation { to: -1.8; duration: 520; easing.type: Easing.InOutSine }
                  NumberAnimation { to: 1.8; duration: 520; easing.type: Easing.InOutSine }
                  NumberAnimation { to: 0; duration: 260; easing.type: Easing.InOutSine }
                }

                SequentialAnimation on scale {
                  running: cell.anim && cell.modelData.anim === "pulse"
                  loops: Animation.Infinite
                  PauseAnimation { duration: glyph.phase % 300 }
                  NumberAnimation { to: 1.18; duration: 500; easing.type: Easing.InOutQuad }
                  NumberAnimation { to: 1.0; duration: 500; easing.type: Easing.InOutQuad }
                }

                RotationAnimation on rotation {
                  running: cell.anim && cell.modelData.anim === "barrel"
                  loops: Animation.Infinite
                  from: 0; to: 360
                  duration: 1600
                  easing.type: Easing.InOutQuad
                }
              }

              Text {
                anchors.centerIn: glyph
                visible: cell.anim && cell.modelData.anim === "glow"
                text: glyph.text; font: glyph.font; color: glyph.color
                z: -1
                SequentialAnimation on scale {
                  running: cell.anim && cell.modelData.anim === "glow"
                  loops: Animation.Infinite
                  NumberAnimation { to: 1.5; duration: 900; easing.type: Easing.InOutSine }
                  NumberAnimation { to: 1.1; duration: 900; easing.type: Easing.InOutSine }
                }
                SequentialAnimation on opacity {
                  running: cell.anim && cell.modelData.anim === "glow"
                  loops: Animation.Infinite
                  NumberAnimation { to: 0.45; duration: 900; easing.type: Easing.InOutSine }
                  NumberAnimation { to: 0.05; duration: 900; easing.type: Easing.InOutSine }
                }
              }

              Text {
                anchors.centerIn: glyph
                visible: cell.anim && cell.modelData.anim === "glitch"
                text: glyph.text; font: glyph.font
                color: Qt.rgba(1, 0.15, 0.25, 0.7)
                z: -2
                SequentialAnimation on anchors.horizontalCenterOffset {
                  running: cell.anim && cell.modelData.anim === "glitch"
                  loops: Animation.Infinite
                  PauseAnimation { duration: (cell.modelData.index * 53) % 400 }
                  NumberAnimation { to: -2; duration: 90 }
                  NumberAnimation { to: 1.5; duration: 70 }
                  NumberAnimation { to: 0; duration: 110 }
                  PauseAnimation { duration: 260 }
                }
              }
              Text {
                anchors.centerIn: glyph
                visible: cell.anim && cell.modelData.anim === "glitch"
                text: glyph.text; font: glyph.font
                color: Qt.rgba(0.15, 0.95, 1, 0.7)
                z: -2
                SequentialAnimation on anchors.horizontalCenterOffset {
                  running: cell.anim && cell.modelData.anim === "glitch"
                  loops: Animation.Infinite
                  PauseAnimation { duration: (cell.modelData.index * 53) % 400 }
                  NumberAnimation { to: 2; duration: 90 }
                  NumberAnimation { to: -1.5; duration: 70 }
                  NumberAnimation { to: 0; duration: 110 }
                  PauseAnimation { duration: 260 }
                }
              }

              Repeater {
                model: (cell.anim && cell.modelData.anim === "sparkle") ? 3 : 0
                delegate: Rectangle {
                  required property int index
                  width: 2; height: 2; radius: 1
                  antialiasing: true
                  color: glyph.color
                  x: glyph.implicitWidth * (0.2 + 0.3 * index)
                  SequentialAnimation on y {
                    running: true; loops: Animation.Infinite
                    PauseAnimation { duration: (cell.modelData.index * 130 + index * 420) % 1300 }
                    NumberAnimation {
                      from: glyph.implicitHeight * 0.6; to: -3
                      duration: 900; easing.type: Easing.OutCubic
                    }
                  }
                  SequentialAnimation on opacity {
                    running: true; loops: Animation.Infinite
                    PauseAnimation { duration: (cell.modelData.index * 130 + index * 420) % 1300 }
                    NumberAnimation { from: 0.9; to: 0; duration: 900 }
                  }
                }
              }

              // The blur layer stays on for the whole span; only the amount animates.
              layer.enabled: cell.modelData.anim === "blur"
              layer.effect: MultiEffect {
                blurEnabled: true
                blurMax: 20
                blur: root.hovered ? 0.0 : 1.0
                Behavior on blur { NumberAnimation { duration: 260; easing.type: Easing.OutCubic } }
              }

              Rectangle {
                anchors.fill: parent
                anchors.topMargin: 1
                anchors.bottomMargin: 1
                radius: Style.space(2)
                visible: opacity > 0.01
                opacity: 1 - cell.shown
                color: Util.alpha(root.fg, 0.34)
                SequentialAnimation on opacity {
                  running: root.active && root.motionOk && root.spoiled && cell.modelData.spoiler
                  loops: Animation.Infinite
                  PauseAnimation { duration: (cell.modelData.index * 70) % 900 }
                  NumberAnimation { to: 0.72; duration: 520; easing.type: Easing.InOutSine }
                  NumberAnimation { to: 1.0; duration: 520; easing.type: Easing.InOutSine }
                }
              }
            }
          }
        }

        // A word that ends a line breaks after itself.
        Item {
          visible: wordCell.modelData.brk && wordCell.modelData.glyphs.length > 0
          width: visible ? flow.width : 0
          height: 0
        }
      }
    }
  }

  MouseArea {
    anchors.fill: parent
    enabled: root.spoiled && root.hasSpoiler
    cursorShape: Qt.PointingHandCursor
    onClicked: root.revealAll()
  }

  /// Test hook: what the renderer thinks it is drawing.
  function debugRich() {
    return JSON.stringify({
      chars: root.glyphs.length, words: root.words.length,
      effects: root.effects.length, motionOk: root.motionOk, active: root.active,
      spoiled: root.spoiled, alreadyRead: root.alreadyRead,
      typewriter: root.hasTypewriter, typed: root.typed,
      anim: root.glyphs.length > 0 ? root.glyphs[0].anim : "",
      size: root.glyphs.length > 0 ? root.glyphs[0].size : 0
    })
  }
}
