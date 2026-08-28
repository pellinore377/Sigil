import QtQuick
import Quickshell
import Quickshell.Wayland
import qs.Commons
import qs.Ui
import "../components"

// Incoming-call banner (service-owned, top centre, never takes keyboard focus).
PanelWindow {
  id: root
  property var svc: null
  readonly property var inc: svc && svc.call ? svc.call.incoming : null
  visible: inc !== null && inc !== undefined
  anchors { top: true }
  margins.top: Style.gapsOut
  implicitWidth: Style.space(440)
  implicitHeight: Style.space(76)
  color: "transparent"
  WlrLayershell.namespace: "omarchy-sigil-call"
  WlrLayershell.layer: WlrLayer.Overlay
  WlrLayershell.keyboardFocus: WlrKeyboardFocus.None
  exclusionMode: ExclusionMode.Ignore

  BorderSurface {
    anchors.fill: parent
    radius: Style.cornerRadius
    color: Color.popups.background
    borderSpec: Border.surfaceSpec("popups", "border", Color.popups.border, 1)
    Avatar { id: av; anchors.left: parent.left; anchors.leftMargin: Style.space(14); anchors.verticalCenter: parent.verticalCenter; size: Style.space(44); source: ""; name: root.inc ? root.inc.callerName : ""; userId: root.inc ? root.inc.callerId : "" }
    Column {
      anchors.left: av.right; anchors.leftMargin: Style.space(12); anchors.right: btns.left; anchors.verticalCenter: parent.verticalCenter
      Text { text: root.inc ? root.inc.callerName + " is calling" : ""; color: Color.popups.text; font.family: Fonts.ui; font.pixelSize: Style.font.title; font.bold: true; elide: Text.ElideRight; width: parent.width }
      Text { text: root.inc ? ((root.inc.intent === "video" ? "Video call" : "Voice call") + " · " + root.inc.roomName) : ""; color: Util.alpha(Color.popups.text, 0.6); font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall; elide: Text.ElideRight; width: parent.width }
    }
    Row {
      id: btns
      anchors.right: parent.right; anchors.rightMargin: Style.space(12); anchors.verticalCenter: parent.verticalCenter
      spacing: Style.space(6)
      Rectangle { width: Style.space(40); height: Style.space(40); radius: height / 2; color: Util.alpha(Color.urgent, 0.85)
        IconLabel { anchors.centerIn: parent; icon: Icons.callEnd; color: Color.popups.text; filled: true; size: Style.font.iconLarge }
        MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.svc.callDecline() } }
      Rectangle { width: Style.space(40); height: Style.space(40); radius: height / 2; color: Util.alpha(Color.accent, 0.35); border.width: 1; border.color: Color.accent
        IconLabel { anchors.centerIn: parent; icon: Icons.phone; color: Color.popups.text; filled: true; size: Style.font.iconLarge }
        MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: { root.svc.callJoin(root.inc.roomId, false); root.svc.openRoomAfterAccept(root.inc.roomId) } } }
      Rectangle { width: Style.space(40); height: Style.space(40); radius: height / 2; color: Util.alpha(Color.accent, 0.35); border.width: 1; border.color: Color.accent
        IconLabel { anchors.centerIn: parent; icon: Icons.videoOn; color: Color.popups.text; filled: true; size: Style.font.iconLarge }
        MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: { root.svc.callJoin(root.inc.roomId, true); root.svc.openRoomAfterAccept(root.inc.roomId) } } }
    }
  }
}
