import QtQuick
import QtQuick.Controls as QQC
import qs.Commons
import qs.Ui
import ".."
import "."

// Sign-in page: homeserver + "Sign in with SSO" (opens the browser; the engine
// finishes the login on its localhost redirect). Engine-missing state offers setup.
Item {
  id: root
  property var svc: null
  property color fg: Color.menu.text
  readonly property bool pending: svc && svc.authState === "loginPending"
  readonly property bool restoring: svc && svc.authState === "restoring"

  Column {
    anchors.centerIn: parent
    width: Math.min(parent.width - Style.space(80), Style.space(440))
    spacing: Style.spacing.xl

    IconLabel { icon: Icons.chat; color: root.fg; anchors.horizontalCenter: parent.horizontalCenter; size: Style.space(44) }
    Text { text: "Sign in to Sigil"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.heading; font.bold: true; anchors.horizontalCenter: parent.horizontalCenter }

    Column {
      width: parent.width
      spacing: Style.spacing.sm
      visible: !root.svc || !root.svc.connected || root.svc.engineMissing
      Text {
        width: parent.width; wrapMode: Text.Wrap; horizontalAlignment: Text.AlignHCenter
        color: Util.alpha(root.fg, 0.7); font.family: Fonts.ui; font.pixelSize: Style.font.body
        text: root.svc && root.svc.engineMissing ? "The Sigil engine (sigil-engine) is not installed."
              : (root.svc && root.svc.engineError ? "Engine: " + root.svc.engineError : "Connecting to the Sigil engine…")
      }
      Button {
        visible: root.svc && root.svc.engineMissing
        anchors.horizontalCenter: parent.horizontalCenter
        text: root.svc && root.svc.engineSetupRunning ? "Building engine…" : "Set up engine"
        enabled: root.svc && !root.svc.engineSetupRunning
        foreground: root.fg
        bordered: true
        onClicked: root.svc.installEngine()
      }
      Text {
        visible: root.svc && root.svc.engineSetupError !== ""
        width: parent.width; wrapMode: Text.Wrap
        color: Color.urgent; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
        text: root.svc ? root.svc.engineSetupError : ""
      }
    }

    Column {
      width: parent.width
      spacing: Style.spacing.md
      visible: root.svc && root.svc.connected && !root.svc.engineMissing && !root.restoring

      Text { text: "Homeserver"; color: Util.alpha(root.fg, 0.6); font.family: Fonts.ui; font.pixelSize: Style.font.caption }
      Rectangle {
        width: parent.width; height: Style.space(36); radius: Style.cornerRadius / 2
        color: Util.alpha(root.fg, 0.06)
        border.width: 1; border.color: hsField.activeFocus ? Util.alpha(Color.accent, 0.6) : Util.alpha(root.fg, 0.12)
        QQC.TextField {
          id: hsField
          anchors.fill: parent
          anchors.leftMargin: Style.space(10); anchors.rightMargin: Style.space(10)
          text: ""
          color: root.fg
          font.family: Fonts.ui; font.pixelSize: Style.font.body
          placeholderText: "example.org"
          placeholderTextColor: Util.alpha(root.fg, 0.4)
          background: Item {}
          enabled: !root.pending
          QQC.ContextMenu.menu: null
          TextContextMenu { editor: parent }
          onAccepted: root.start()
        }
      }
      Button {
        anchors.horizontalCenter: parent.horizontalCenter
        text: root.pending ? "Waiting for the browser…" : "Sign in with SSO"
        iconText: root.pending ? "" : Icons.login
        enabled: !root.pending && hsField.text.trim() !== ""
        foreground: root.fg
        bordered: true
        onClicked: root.start()
      }
      Row {
        visible: root.pending
        anchors.horizontalCenter: parent.horizontalCenter
        spacing: Style.spacing.lg
        Button { text: "Open link again"; foreground: root.fg; onClicked: Qt.openUrlExternally(root.svc.ssoUrl) }
        Button { text: "Cancel"; foreground: root.fg; onClicked: root.svc.ssoCancel() }
      }
      Text {
        visible: root.pending
        width: parent.width; wrapMode: Text.Wrap; horizontalAlignment: Text.AlignHCenter
        color: Util.alpha(root.fg, 0.55); font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
        text: "Complete the sign-in in your browser. This window updates automatically."
      }
      Text {
        visible: root.svc && root.svc.authError !== ""
        width: parent.width; wrapMode: Text.Wrap; horizontalAlignment: Text.AlignHCenter
        color: Color.urgent; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
        text: root.svc ? root.svc.authError : ""
      }
    }
    Row {
      visible: root.restoring
      anchors.horizontalCenter: parent.horizontalCenter
      spacing: Style.spacing.md
      Spinner { color: root.fg }
      Text { text: "Restoring session…"; color: Util.alpha(root.fg, 0.7); font.family: Fonts.ui; font.pixelSize: Style.font.body }
    }
  }

  function start() {
    if (!root.svc || root.pending) return
    root.svc.ssoStart(hsField.text.trim())
  }
  function focusInput() { hsField.forceActiveFocus() }
}
