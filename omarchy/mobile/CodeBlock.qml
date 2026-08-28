import QtQuick
import QtQuick.Effects
import qs.Commons
import qs.Ui
import "../components"

// A fenced code block, laid out like a picture inside the bubble. The ground is
// a neutral dark grey from the engine, not the room's colour: the highlighter's
// palette is designed against a dark background.
Item {
  id: root
  /// Pre-highlighted, pre-escaped markup from the engine. Nothing is parsed here.
  property string html: ""
  /// The fence's language tag, for the corner badge. Empty is fine.
  property string lang: ""
  property color fg: Color.menu.text
  property real topLeftRadius: 0
  property real topRightRadius: 0
  property real bottomLeftRadius: 0
  property real bottomRightRadius: 0

  readonly property color ground: Qt.rgba(0.141, 0.141, 0.157, 1)   // #242428
  readonly property real pad: Style.space(11)

  implicitHeight: Math.max(Style.space(34), codeText.implicitHeight + root.pad * 2)

  // `clip` on a Rectangle follows its bounding box, not its rounded shape, so
  // the corners come from a layer mask.
  Item {
    id: mask
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
      maskSource: mask
      maskThresholdMin: 0.5
      maskSpreadAtMin: 1.0
    }

    Rectangle { anchors.fill: parent; color: root.ground }

    TextEdit {
      id: codeText
      anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
      anchors.margins: root.pad
      anchors.bottomMargin: root.pad + (root.lang !== "" ? Style.space(14) : 0)
      readOnly: true
      selectByMouse: true
      textFormat: TextEdit.RichText
      // Wrapped in `<pre>` here, not in the engine: rich text collapses runs of
      // whitespace, turning every newline into a space.
      text: "<pre style=\"white-space:pre-wrap\">" + root.html + "</pre>"
      // Long lines wrap rather than scroll: a horizontal scroll region inside a
      // vertical one is unusable.
      wrapMode: TextEdit.Wrap
      color: Qt.rgba(0.86, 0.87, 0.89, 1)
      selectionColor: Util.alpha(Color.accent, 0.45)
      font.family: Fonts.ui
      font.pixelSize: Style.font.caption
    }

    Rectangle {
      visible: root.lang !== ""
      anchors.right: parent.right; anchors.bottom: parent.bottom
      anchors.margins: Style.space(6)
      width: langText.implicitWidth + Style.space(10); height: Style.space(15)
      radius: Style.space(4)
      color: Util.alpha("#ffffff", 0.10)
      Text {
        id: langText
        anchors.centerIn: parent
        text: root.lang
        color: Util.alpha("#ffffff", 0.55)
        font.family: Fonts.ui; font.pixelSize: Style.space(8); font.bold: true
      }
    }
  }
}
