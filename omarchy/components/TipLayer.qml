import QtQuick
import qs.Commons
import qs.Ui
import "."

// In-card hover tips. Qt 6.9 renders QQC ToolTip in its own popup *window* by
// default, so the shared PanelToolTip drew outside the panel; this keeps tips
// inside the card where they belong.
Item {
  id: root
  property color fg: Color.menu.text
  property string text: ""
  property real tipX: 0
  property real tipY: 0
  property bool shown: false

  // Scene coordinates in, layer coordinates out.
  function show(t, sceneX, sceneY) {
    var p = root.mapFromItem(null, sceneX, sceneY)
    root.text = t
    root.tipX = p.x
    root.tipY = p.y
    root.shown = true
  }
  function hide() { root.shown = false }

  visible: opacity > 0.01
  opacity: root.shown && root.text !== "" ? 1 : 0
  Behavior on opacity { NumberAnimation { duration: 120 } }

  Rectangle {
    id: bubble
    width: label.implicitWidth + Style.space(16)
    height: label.implicitHeight + Style.space(10)
    x: Math.max(Style.space(6), Math.min(root.tipX - width / 2, root.width - width - Style.space(6)))
    y: Math.max(Style.space(6), Math.min(root.tipY, root.height - height - Style.space(6)))
    radius: Style.space(8)
    antialiasing: true
    color: Util.alpha(Qt.lighter(Color.menu.background, 1.5), 0.98)
    border.width: 1
    border.color: Util.alpha(root.fg, 0.12)
    scale: root.shown ? 1 : 0.94
    transformOrigin: Item.Top
    Behavior on scale { NumberAnimation { duration: 130; easing.type: Easing.OutCubic } }

    Text {
      id: label
      anchors.centerIn: parent
      text: root.text
      color: root.fg
      font.family: Fonts.ui
      font.pixelSize: Style.font.caption
    }
  }
}
