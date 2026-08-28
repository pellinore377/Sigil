import QtQuick
import QtQuick.Effects
import qs.Commons
import qs.Ui
import "../components"

// A music file on a bubble: cover, play button, and a chrome strip tinted from
// the art. With no art the chrome takes the whole card.
Item {
  id: root
  /// `{artPath, accent, waveform, duration}` from `audio.info`, or null while pending.
  property var info: null
  property string title: ""
  property string durationLabel: ""
  property color fg: Color.menu.text
  property color accent: Color.accent
  property real topLeftRadius: Style.space(16)
  property real topRightRadius: Style.space(16)
  property real bottomLeftRadius: Style.space(16)
  property real bottomRightRadius: Style.space(16)
  signal openRequested()

  readonly property string art: info ? (info.artPath || "") : ""
  readonly property bool haveArt: root.art !== ""
  readonly property color tone: (info && info.accent) ? Qt.color(info.accent) : root.accent
  readonly property color blankArt: Util.alpha(root.fg, 0.13)
  /// Perceived brightness of the strip colour, 0..1.
  readonly property real toneLum: 0.299 * root.tone.r + 0.587 * root.tone.g + 0.114 * root.tone.b
  /// White or near-black over that colour, whichever reads.
  ///
  /// **Not** named `onTone`: QML treats a member beginning with `on` as a signal
  /// handler, so the binding is silently dropped and the default colour sticks.
  readonly property color labelInk: root.toneLum > 0.62 ? Qt.rgba(0.09, 0.09, 0.09, 1) : Qt.rgba(1, 1, 1, 0.96)

  /// Test hook: the two colours that decide whether the strip is readable.
  function debugTone() {
    return JSON.stringify({ tone: String(root.tone), lum: Math.round(root.toneLum * 100) / 100,
                            toneR: root.tone.r, toneG: root.tone.g,
                            inkR: root.labelInk.r, inkA: root.labelInk.a,
                            haveArt: root.haveArt })
  }

  readonly property real stripH: Style.space(52)
  /// Square either way, so a run of tracks does not jump about.
  readonly property real artH: root.width
  implicitHeight: root.artH + root.stripH

  // A rounded clip must be a layer mask: a Rectangle's `clip` follows its
  // bounding box and lets the art paint over the corners.
  Item {
    id: cardMask
    anchors.fill: parent
    visible: false
    layer.enabled: true
    Rectangle {
      anchors.fill: parent
      topLeftRadius: root.topLeftRadius
      topRightRadius: root.topRightRadius
      bottomLeftRadius: root.bottomLeftRadius
      bottomRightRadius: root.bottomRightRadius
      antialiasing: true
      color: "black"
    }
  }

  Item {
    anchors.fill: parent
    layer.enabled: true
    layer.smooth: true
    layer.effect: MultiEffect {
      maskEnabled: true
      maskSource: cardMask
      maskThresholdMin: 0.5
      maskSpreadAtMin: 1.0
    }

    // Art
    Item {
      id: artBox
      anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
      height: root.artH

      Rectangle {
        anchors.fill: parent
        color: root.haveArt ? Qt.darker(root.tone, 1.3) : root.blankArt
      }
      Image {
        anchors.fill: parent
        visible: root.haveArt
        source: root.haveArt ? "file://" + root.art : ""
        fillMode: Image.PreserveAspectCrop
        asynchronous: true
        cache: true
      }
      Rectangle {
        anchors.fill: parent
        visible: root.haveArt
        color: Util.alpha("#000000", 0.18)
      }

      Rectangle {
        id: noteBtn
        anchors.centerIn: parent
        width: Style.space(56); height: width; radius: width / 2
        color: root.haveArt ? Util.alpha("#000000", 0.55) : root.tone
        antialiasing: true
        Text {
          anchors.centerIn: parent
          text: Icons.music
          color: root.haveArt ? "#ffffff" : root.labelInk
          font.family: Fonts.icon; renderType: Text.NativeRendering
          font.pixelSize: Style.space(26)
        }
        scale: tap.pressed ? 0.92 : 1.0
        Behavior on scale { NumberAnimation { duration: 90 } }
      }

      MouseArea {
        id: tap
        anchors.fill: parent
        cursorShape: Qt.PointingHandCursor
        onClicked: root.openRequested()
      }
    }

    // Chrome
    Rectangle {
      anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom
      height: root.stripH
      color: root.tone

      Column {
        anchors.left: parent.left; anchors.right: parent.right
        anchors.leftMargin: Style.space(12); anchors.rightMargin: Style.space(12)
        anchors.verticalCenter: parent.verticalCenter
        spacing: Style.space(1)
        Text {
          width: parent.width; elide: Text.ElideMiddle
          text: root.title
          color: root.labelInk
          font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: true
        }
        Text {
          width: parent.width; elide: Text.ElideRight
          visible: text !== ""
          text: root.durationLabel
          color: Util.alpha(root.labelInk, 0.75)
          font.family: Fonts.ui; font.pixelSize: Style.font.caption
        }
      }
      MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.openRequested() }
    }
  }
}
