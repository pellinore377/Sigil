import QtQuick
import Quickshell
import Quickshell.Wayland
import qs.Commons
import qs.Ui
import "../components"

// Persistent mini control while a call is active and the main window is hidden.
PanelWindow {
  id: root
  property var svc: null
  readonly property var call: svc ? svc.call : ({})
  property real now: Date.now()   // int is 32-bit; an epoch ms overflows it
  Timer { interval: 1000; running: root.visible; repeat: true; onTriggered: root.now = Date.now() }
  visible: svc ? svc.inCall : false
  anchors { top: true }
  margins.top: Style.gapsOut
  implicitWidth: Style.space(300)
  implicitHeight: Style.space(44)
  color: "transparent"
  WlrLayershell.namespace: "omarchy-sigil-call"
  WlrLayershell.layer: WlrLayer.Overlay
  WlrLayershell.keyboardFocus: WlrKeyboardFocus.None
  exclusionMode: ExclusionMode.Ignore

  BorderSurface {
    anchors.fill: parent
    radius: height / 2
    color: Color.popups.background
    borderSpec: Border.surfaceSpec("popups", "border", Color.popups.border, 1)
    Row {
      anchors.centerIn: parent; spacing: Style.space(10)
      IconLabel { icon: Icons.phone; color: Color.accent; anchors.verticalCenter: parent.verticalCenter; filled: true; size: Style.font.icon }
      Text { text: root.call.state === "connected" && root.call.since ? fmt(root.now - root.call.since) : (root.call.state || ""); color: Color.popups.text; font.family: Fonts.ui; font.pixelSize: Style.font.body; anchors.verticalCenter: parent.verticalCenter }
      Rectangle { width: Style.space(30); height: Style.space(30); radius: height / 2; color: (root.call.local && root.call.local.micMuted) ? Util.alpha(Color.urgent, 0.3) : Util.alpha(Color.popups.text, 0.1); anchors.verticalCenter: parent.verticalCenter
        IconLabel { anchors.centerIn: parent; icon: (root.call.local && root.call.local.micMuted) ? Icons.micOff : Icons.micOn; color: Color.popups.text; filled: true; size: Style.font.icon }
        MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.svc.callSetMic(!!(root.call.local && root.call.local.micMuted)) } }
      Rectangle { width: Style.space(30); height: Style.space(30); radius: height / 2; color: Util.alpha(Color.popups.text, 0.1); anchors.verticalCenter: parent.verticalCenter
        IconLabel { anchors.centerIn: parent; icon: Icons.windowed; color: Color.popups.text; filled: true; size: Style.font.icon }
        MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.svc.openRoomAfterAccept(root.call.roomId) } }
      Rectangle { width: Style.space(30); height: Style.space(30); radius: height / 2; color: Util.alpha(Color.urgent, 0.85); anchors.verticalCenter: parent.verticalCenter
        IconLabel { anchors.centerIn: parent; icon: Icons.callEnd; color: Color.popups.text; filled: true; size: Style.font.icon }
        MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.svc.callHangup() } }
    }
  }
  function fmt(ms) { var s = Math.max(0, Math.floor(ms / 1000)), m = Math.floor(s / 60), sec = s % 60; return m + ":" + (sec < 10 ? "0" : "") + sec }
}
