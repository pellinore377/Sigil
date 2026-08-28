import QtQuick
import qs.Commons
import ".."
import "../components"

FrostPopup {
  id: root
  property var svc: null
  property string kind: ""
  tint: Color.popups.background
  popupWidth: Style.space(300)
  popupHeight: col.implicitHeight + padding * 2
  padding: Style.space(8)
  readonly property var devices: !svc ? [] : (kind === "mic" ? (svc.devices.mics || []) : kind === "speaker" ? (svc.devices.speakers || []) : (svc.devices.cameras || []))
  readonly property string selected: (svc && svc.devices.selected) ? (svc.devices.selected[kind] || "") : ""

  function openFor(k, anchor) { root.kind = k; root.anchorItem = anchor; if (svc) svc.refreshDevices(); root.open() }

  Column {
    id: col; width: parent.width; spacing: Style.space(2)
    Text { text: root.kind === "mic" ? "Microphone" : root.kind === "speaker" ? "Speaker" : "Camera"; color: Util.alpha(Color.popups.text, 0.6); font.family: Fonts.ui; font.pixelSize: Style.font.caption; font.bold: true; bottomPadding: Style.space(4) }
    Repeater {
      model: root.devices
      delegate: Rectangle {
        required property var modelData
        width: parent.width; height: Style.space(30); radius: Style.cornerRadius / 2
        readonly property bool sel: root.selected === modelData.id || (root.selected === "" && modelData.default)
        color: h.containsMouse ? Util.alpha(Color.popups.text, 0.1) : "transparent"
        Row { anchors.verticalCenter: parent.verticalCenter; anchors.left: parent.left; anchors.leftMargin: Style.space(6); spacing: Style.space(8)
          Text { text: parent.parent.sel ? Icons.check : " "; color: Color.accent; font.family: Fonts.iconFilled; renderType: Text.NativeRendering; font.pixelSize: Style.font.icon; width: Style.space(14) }
          Text { text: modelData.name; color: Color.popups.text; font.family: Fonts.ui; font.pixelSize: Style.font.body; elide: Text.ElideRight; width: Style.space(250) } }
        MouseArea { id: h; anchors.fill: parent; hoverEnabled: true; onClicked: { if (root.svc) root.svc.callSelectDevice(root.kind, modelData.id); root.close() } }
      }
    }
    Text { visible: root.devices.length === 0; text: "No devices found"; color: Util.alpha(Color.popups.text, 0.5); font.family: Fonts.ui; font.pixelSize: Style.font.body }
  }
}
