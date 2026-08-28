import QtQuick
import qs.Commons
import qs.Ui

// The ⋮ menu that drops from a page header's top-right corner. `model` is a list
// of `{ t, a, icon, danger }`; picking one closes the menu and emits `picked(a)`.
// The scrim belongs to the menu, so the menu can always be dismissed.
Item {
  id: root
  property var model: []
  property color fg: Color.menu.text
  property color surface: Util.alpha(Color.popups.background, 0.98)
  property bool open: false
  /// Inset from the top-right corner of whatever this fills.
  property real topInset: Style.space(48)

  signal picked(string action)

  anchors.fill: parent
  visible: open || card.opacity > 0.01

  MouseArea {
    anchors.fill: parent
    enabled: root.open
    onClicked: root.open = false
    // Right-click must dismiss too, or it falls through and opens the menu underneath.
    acceptedButtons: Qt.LeftButton | Qt.RightButton
  }

  Rectangle {
    id: card
    anchors.right: parent.right; anchors.rightMargin: Style.space(10)
    anchors.top: parent.top; anchors.topMargin: root.topInset
    width: Style.space(210)
    height: col.implicitHeight + Style.space(12)
    radius: Style.space(14)
    color: root.surface
    opacity: root.open ? 1 : 0
    scale: root.open ? 1 : 0.92
    transformOrigin: Item.TopRight
    Behavior on opacity { NumberAnimation { duration: 120 } }
    Behavior on scale { NumberAnimation { duration: 120; easing.type: Easing.OutCubic } }

    Column {
      id: col
      anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
      anchors.margins: Style.space(6)
      Repeater {
        model: root.model
        delegate: Rectangle {
          required property var modelData
          width: parent.width; height: Style.space(34); radius: Style.space(9)
          color: mh.containsMouse ? Util.alpha(root.fg, 0.08) : "transparent"
          readonly property color ink: modelData.danger ? Color.urgent : root.fg
          IconLabel {
            anchors.verticalCenter: parent.verticalCenter
            anchors.left: parent.left; anchors.leftMargin: Style.space(12)
            icon: modelData.icon; color: parent.ink; opacity: 0.85
            filled: true; size: Style.font.icon
          }
          Text {
            anchors.verticalCenter: parent.verticalCenter
            anchors.left: parent.left; anchors.leftMargin: Style.space(38)
            anchors.right: parent.right; anchors.rightMargin: Style.space(8)
            text: modelData.t; color: parent.ink; elide: Text.ElideRight
            font.family: Fonts.ui; font.pixelSize: Style.font.body
          }
          MouseArea {
            id: mh
            anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor
            onClicked: { root.open = false; root.picked(modelData.a) }
          }
        }
      }
    }
  }
}
