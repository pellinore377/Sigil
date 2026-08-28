import QtQuick
import qs.Commons
import qs.Ui
import "../components"

// Strip above the timeline when a call is active in a different room.
Rectangle {
  id: root
  property var svc: null
  property color fg: Color.menu.text
  signal returnToCall()
  readonly property var call: svc ? svc.call : ({})
  readonly property var callRoom: (svc && call && call.roomId) ? svc.room(call.roomId) : null
  height: Style.space(36)
  color: Util.alpha(Color.accent, 0.12)
  Row {
    anchors.left: parent.left; anchors.leftMargin: Style.space(14); anchors.verticalCenter: parent.verticalCenter; spacing: Style.space(10)
    IconLabel { icon: Icons.phone; color: Color.accent; anchors.verticalCenter: parent.verticalCenter; filled: true; size: Style.font.icon }
    Text { text: "In a call in " + (root.callRoom ? root.callRoom.name : "another room"); color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body; anchors.verticalCenter: parent.verticalCenter }
  }
  Row {
    anchors.right: parent.right; anchors.rightMargin: Style.space(10); anchors.verticalCenter: parent.verticalCenter; spacing: Style.space(6)
    Button { text: "Return"; foreground: root.fg; bordered: true; onClicked: root.returnToCall() }
    Button { text: "Hang up"; foreground: Color.urgent; onClicked: root.svc.callHangup() }
  }
}
