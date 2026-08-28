import QtQuick
import QtQuick.Controls as QQC
import qs.Commons
import qs.Ui
import ".."
import "."

// Enter the Element "Security Key" (or passphrase) to unlock secret storage.
Item {
  id: root
  property var svc: null
  property color fg: Color.menu.text
  property bool busy: false
  property bool reveal: false

  Column {
    anchors.centerIn: parent
    width: Math.min(parent.width - Style.space(80), Style.space(480))
    spacing: Style.spacing.xl

    IconLabel { icon: Icons.recoveryKey; color: root.fg; anchors.horizontalCenter: parent.horizontalCenter; size: Style.space(40) }
    Text { text: "Restore encrypted history"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.heading; font.bold: true; anchors.horizontalCenter: parent.horizontalCenter }
    Text {
      width: parent.width; wrapMode: Text.Wrap; horizontalAlignment: Text.AlignHCenter
      color: Util.alpha(root.fg, 0.7); font.family: Fonts.ui; font.pixelSize: Style.font.body
      text: "Enter the recovery key (Security Key) from Element to verify this device and decrypt your message history. You can skip this and do it later."
    }
    Rectangle {
      width: parent.width; height: Style.space(36); radius: Style.cornerRadius / 2
      color: Util.alpha(root.fg, 0.06)
      border.width: 1; border.color: keyField.activeFocus ? Util.alpha(Color.accent, 0.6) : Util.alpha(root.fg, 0.12)
      QQC.TextField {
        id: keyField
        anchors.fill: parent
        anchors.leftMargin: Style.space(10); anchors.rightMargin: Style.space(36)
        color: root.fg
        echoMode: root.reveal ? TextInput.Normal : TextInput.Password
        font.family: Fonts.ui; font.pixelSize: Style.font.body
        placeholderText: "EsTc 1234 …"
        placeholderTextColor: Util.alpha(root.fg, 0.4)
        background: Item {}
        enabled: !root.busy
        QQC.ContextMenu.menu: null
        TextContextMenu { editor: parent }
        onAccepted: root.submit()
      }
      PanelActionButton {
        fontFamily: Fonts.icon
        anchors.right: parent.right; anchors.rightMargin: Style.space(6); anchors.verticalCenter: parent.verticalCenter
        iconText: root.reveal ? Icons.eye : Icons.eyeOff
        foreground: Util.alpha(root.fg, 0.7)
        tooltipText: root.reveal ? "Hide" : "Show"
        onClicked: root.reveal = !root.reveal
      }
    }
    Row {
      anchors.horizontalCenter: parent.horizontalCenter
      spacing: Style.spacing.lg
      Button { text: root.busy ? "Verifying…" : "Verify"; enabled: !root.busy && keyField.text.trim() !== ""; foreground: root.fg; bordered: true; onClicked: root.submit() }
      Button { text: "Skip for now"; enabled: !root.busy; foreground: root.fg; onClicked: root.svc.skipRecovery() }
    }
    Text {
      visible: root.svc && root.svc.recoveryError !== ""
      width: parent.width; wrapMode: Text.Wrap; horizontalAlignment: Text.AlignHCenter
      color: Color.urgent; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
      text: root.svc ? root.svc.recoveryError : ""
    }
  }

  function submit() {
    if (!root.svc || root.busy || keyField.text.trim() === "") return
    root.busy = true
    root.svc.submitRecoveryKey(keyField.text, function(r, e) { root.busy = false; if (!e) keyField.text = "" })
  }
  function focusInput() { keyField.forceActiveFocus() }
}
