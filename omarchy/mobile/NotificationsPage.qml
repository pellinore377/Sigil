import QtQuick
import qs.Commons
import qs.Ui
import "../components"

// Per-room notifications. Matrix has three states here, not two: a room can
// follow the account default or carry its own rule. "Allow custom setting" is
// that distinction — off means no per-room push rule exists at all, so turning
// it off deletes the rule rather than setting it to "all".
Item {
  id: root
  property var svc: null
  property string roomId: ""
  property color fg: Color.menu.text

  signal closed()

  property var settings: ({})
  property bool busy: false
  property string error: ""

  /// null from the engine means "no per-room rule" — the account default.
  readonly property var mode: root.settings.notificationMode !== undefined
                              ? root.settings.notificationMode : null
  readonly property bool custom: root.mode !== null && root.mode !== undefined

  function reset() { root.error = ""; root.load() }
  function load() {
    if (!root.svc || !root.roomId) return
    root.svc.roomSettings(root.roomId, function (r, e) {
      if (r) root.settings = r
      else if (e) root.error = "Could not read notification settings"
    })
  }
  onRoomIdChanged: root.load()

  function set(m) {
    if (!root.svc || root.busy) return
    root.busy = true
    root.svc.setRoomSettings(root.roomId, { notificationMode: m }, function (r, e) {
      root.busy = false
      if (e) { root.error = "Could not change notifications"; return }
      root.error = ""
      // Show what we just wrote: an immediate re-read still returns the old value.
      var next = JSON.parse(JSON.stringify(root.settings))
      next.notificationMode = (m === "default") ? null : m
      root.settings = next
      confirm.restart()
    })
  }
  /// Re-read a beat later, so a write the server refused is not left on screen.
  Timer { id: confirm; interval: 1200; onTriggered: root.load() }

  Rectangle { anchors.fill: parent; color: Qt.lighter(Color.menu.background, 1.35) }

  Column {
    anchors.fill: parent
    spacing: 0

    SettingsHeader { fg: root.fg; title: "Notifications"; onBack: root.closed() }

    SettingsRow {
      fg: root.fg
      label: "Allow custom setting"
      sublabel: "Turning this on will override your default setting"
      trailing: "toggle"
      on: root.custom
      enabled: !root.busy
      // Off deletes the room's rule; on starts from "all messages".
      onClicked: root.set(root.custom ? "default" : "all")
    }

    // Only one of these two bands is ever on screen.
    SettingsGroup {
      fg: root.fg
      visible: !root.custom
      title: "Default setting"
      Text {
        x: Style.space(22)
        width: parent.width - Style.space(44)
        wrapMode: Text.Wrap
        text: "This chat follows your account-wide setting."
        color: Util.alpha(root.fg, 0.55)
        font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
        bottomPadding: Style.space(8)
      }
      SettingsRow {
        fg: root.fg
        label: "All messages"
        trailing: "radio"; on: true; enabled: false
      }
    }

    SettingsGroup {
      fg: root.fg
      visible: root.custom
      title: "Notify me in this chat for"
      SettingsRow {
        fg: root.fg
        label: "All messages"
        trailing: "radio"; on: root.mode === "all"; enabled: !root.busy
        onClicked: root.set("all")
      }
      SettingsRow {
        fg: root.fg
        label: "Mentions and replies only"
        // Server-side push rules cannot see inside an encrypted message, so this
        // mode is best-effort in encrypted rooms.
        sublabel: root.settings.isEncrypted
                  ? "Encrypted messages are matched on the device, so some may not notify."
                  : ""
        trailing: "radio"; on: root.mode === "mentions"; enabled: !root.busy
        onClicked: root.set("mentions")
      }
      SettingsRow {
        fg: root.fg
        label: "Mute"
        trailing: "radio"; on: root.mode === "mute"; enabled: !root.busy
        onClicked: root.set("mute")
      }
    }

    Item { width: parent.width; height: Style.space(10) }
    Text {
      x: Style.space(22)
      width: parent.width - Style.space(44)
      visible: root.error !== ""
      text: root.error; color: Color.urgent; wrapMode: Text.Wrap
      font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
    }
  }
}
