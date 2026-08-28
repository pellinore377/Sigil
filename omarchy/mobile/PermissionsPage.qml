import QtQuick
import qs.Commons
import qs.Ui
import "../components"

// The permissions grid: the minimum role for each capability. Every row writes
// one key of the same `m.room.power_levels` event, sent one at a time through
// `room.setPowerLevel` — a batch that half-applies leaves a state nobody chose.
Item {
  id: root
  property var svc: null
  property string roomId: ""
  property color fg: Color.menu.text

  signal closed()

  property var settings: ({})
  property bool busy: false
  property string note: ""
  Timer { id: noteTimer; interval: 2600; onTriggered: root.note = "" }

  readonly property var levels: root.settings.powerLevels || ({})
  readonly property bool mayEdit: !!(root.settings.can && root.settings.can.setPowerLevels)

  function roleName(l) { return l >= 100 ? "Admin" : (l >= 50 ? "Moderator" : "Member") }

  function reset() { root.note = ""; root.load() }
  function load() {
    if (!root.svc || !root.roomId) return
    root.svc.roomSettings(root.roomId, function (r) { if (r) root.settings = r })
  }
  onRoomIdChanged: root.load()

  property string pendingKey: ""
  function choose(key) {
    if (!root.mayEdit) return
    root.pendingKey = key
    pick.value = root.levels[key] !== undefined ? root.roleName(root.levels[key]) : "Member"
    pick.open = true
  }
  function apply(level) {
    if (!root.svc || root.pendingKey === "") return
    root.busy = true
    root.svc.setPowerLevel(root.roomId, { key: root.pendingKey, level: level }, function (r, e) {
      root.busy = false
      root.pendingKey = ""
      if (e) { root.note = "Could not change that permission"; noteTimer.restart(); return }
      root.load()
    })
  }

  Rectangle { anchors.fill: parent; color: Qt.lighter(Color.menu.background, 1.35) }

  Flickable {
    anchors.fill: parent
    contentWidth: width
    contentHeight: col.implicitHeight + Style.space(24)
    clip: true
    boundsBehavior: Flickable.StopAtBounds

    Column {
      id: col
      width: parent.width
      spacing: 0

      SettingsHeader { fg: root.fg; title: "Permissions"; onBack: root.closed() }

      SettingsGroup {
        fg: root.fg
        title: "Manage members"
        divided: false
        Repeater {
          model: [ { t: "Invite people", k: "invite" },
                   { t: "Remove people", k: "kick" },
                   { t: "Ban people",    k: "ban" } ]
          delegate: SettingsRow {
            required property var modelData
            fg: root.fg
            label: modelData.t
            trailing: "value"
            value: root.roleName(root.levels[modelData.k] || 0)
            enabled: root.mayEdit && !root.busy
            onClicked: root.choose(modelData.k)
          }
        }
      }

      SettingsGroup {
        fg: root.fg
        title: "Edit details"
        Repeater {
          model: [ { t: "Change name",   k: "name" },
                   { t: "Change avatar", k: "avatar" },
                   { t: "Change topic",  k: "topic" } ]
          delegate: SettingsRow {
            required property var modelData
            fg: root.fg
            label: modelData.t
            trailing: "value"
            value: root.roleName(root.levels[modelData.k] || 0)
            enabled: root.mayEdit && !root.busy
            onClicked: root.choose(modelData.k)
          }
        }
      }

      SettingsGroup {
        fg: root.fg
        title: "Messages and content"
        Repeater {
          model: [ { t: "Send messages",      k: "eventsDefault" },
                   { t: "Remove messages",    k: "redact" },
                   { t: "Share live location", k: "liveLocation" } ]
          delegate: SettingsRow {
            required property var modelData
            fg: root.fg
            label: modelData.t
            trailing: "value"
            value: root.roleName(root.levels[modelData.k] || 0)
            enabled: root.mayEdit && !root.busy
            onClicked: root.choose(modelData.k)
          }
        }
      }

      Item { width: parent.width; height: Style.space(10) }
      Text {
        x: Style.space(22)
        width: parent.width - Style.space(44)
        visible: !root.mayEdit
        text: "Only an admin can change permissions."
        color: Util.alpha(root.fg, 0.5); wrapMode: Text.Wrap
        font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
      }
    }
  }

  ChoiceSheet {
    id: pick
    fg: root.fg
    title: "Minimum role"
    model: [ { t: "Member", v: "Member", sub: "Everyone in the room." },
             { t: "Moderator", v: "Moderator", sub: "Level 50 and above." },
             { t: "Admin", v: "Admin", sub: "Level 100 only." } ]
    onChose: function (v) { root.apply(v === "Admin" ? 100 : (v === "Moderator" ? 50 : 0)) }
  }

  Rectangle {
    anchors.horizontalCenter: parent.horizontalCenter
    anchors.bottom: parent.bottom; anchors.bottomMargin: Style.space(20)
    width: nT.implicitWidth + Style.space(28); height: Style.space(34)
    radius: height / 2
    color: Color.popups.background
    opacity: root.note !== "" ? 1 : 0
    visible: opacity > 0.01
    Behavior on opacity { NumberAnimation { duration: 150 } }
    Text { id: nT; anchors.centerIn: parent; text: root.note; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall }
  }
}
