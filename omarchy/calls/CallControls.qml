import QtQuick
import qs.Commons
import qs.Ui
import ".."
import "../components"

Item {
  id: root
  property var svc: null
  property color fg: Color.menu.text
  signal beforeScreenshare()
  readonly property var call: svc ? svc.call : ({})
  readonly property bool micMuted: call && call.local ? !!call.local.micMuted : false
  readonly property bool cameraOn: call && call.local ? !!call.local.cameraOn : false
  readonly property bool sharing: call && call.local ? !!call.local.screenSharing : false
  height: Style.space(56)

  Row {
    anchors.centerIn: parent
    spacing: Style.space(10)
    Repeater {
      model: [
        { icon: root.micMuted ? Icons.micOff : Icons.micOn, tip: root.micMuted ? "Unmute" : "Mute", on: !root.micMuted, a: "mic", dev: "mic" },
        { icon: root.cameraOn ? Icons.videoOn : Icons.videoOff, tip: root.cameraOn ? "Turn camera off" : "Turn camera on", on: root.cameraOn, a: "camera", dev: "camera" },
        { icon: Icons.screenShare, tip: root.sharing ? "Stop sharing" : "Share screen", on: root.sharing, a: "screen", dev: "" },
        { icon: Icons.speaker, tip: "Speaker", on: true, a: "speaker", dev: "speaker" },
        { icon: Icons.callEnd, tip: "Leave", on: false, a: "leave", dev: "" }
      ]
      delegate: Item {
        required property var modelData
        width: Style.space(modelData.dev ? 66 : 48); height: Style.space(40)
        Rectangle {
          id: btn
          anchors.left: parent.left; width: Style.space(44); height: Style.space(40); radius: height / 2
          color: modelData.a === "leave" ? Util.alpha(Color.urgent, 0.85) : (modelData.on ? Util.alpha(root.fg, 0.14) : Util.alpha(Color.urgent, 0.25))
          border.width: 1; border.color: Util.alpha(root.fg, 0.15)
          IconLabel { anchors.centerIn: parent; icon: modelData.icon; color: root.fg; filled: true; size: Style.font.iconLarge }
          MouseArea {
            anchors.fill: parent; cursorShape: Qt.PointingHandCursor
            onClicked: {
              if (!root.svc) return
              if (modelData.a === "mic") root.svc.callSetMic(root.micMuted)
              else if (modelData.a === "camera") root.svc.callSetCamera(!root.cameraOn)
              else if (modelData.a === "screen") { if (!root.sharing) root.beforeScreenshare(); root.svc.callScreenshare(!root.sharing) }
              else if (modelData.a === "leave") root.svc.callHangup()
              else if (modelData.a === "speaker") devicePicker.openFor("speaker", btn)
            }
          }
        }
        Rectangle {
          visible: modelData.dev !== ""
          anchors.right: parent.right; width: Style.space(18); height: Style.space(40)
          color: "transparent"
          IconLabel { anchors.centerIn: parent; icon: Icons.chevronDown; color: Util.alpha(root.fg, 0.7); filled: true; size: Style.font.bodySmall }
          MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: devicePicker.openFor(modelData.dev, btn) }
        }
      }
    }
  }

  DevicePicker { id: devicePicker; svc: root.svc }
}
