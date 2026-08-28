import QtQuick
import qs.Commons
import qs.Ui
import "../components"

// Roles & permissions: who holds power here, and the bar for each capability.
// Matrix has no roles, only integers; the three names are the shared client
// convention — 100 admin, 50 moderator, anything else a member.
Item {
  id: root
  property var svc: null
  property string roomId: ""
  property color fg: Color.menu.text

  signal closed()
  signal openPermissions()
  signal openRole(int level)     // show the people at this level

  property var settings: ({})
  property bool busy: false
  property string note: ""
  Timer { id: noteTimer; interval: 2600; onTriggered: root.note = "" }

  readonly property var can: root.settings.can || ({})
  readonly property int myLevel: root.settings.myPowerLevel || 0
  readonly property var users: (root.settings.powerLevels || {}).users || ({})

  function countAt(min, max) {
    var n = 0
    for (var u in root.users) {
      var l = root.users[u]
      if (l >= min && (max === undefined || l < max)) n++
    }
    return n
  }
  readonly property int admins: root.countAt(100)
  readonly property int moderators: root.countAt(50, 100)

  function roleName(l) { return l >= 100 ? "Admin" : (l >= 50 ? "Moderator" : "Member") }

  function reset() { root.note = ""; root.load() }
  function load() {
    if (!root.svc || !root.roomId) return
    root.svc.roomSettings(root.roomId, function (r) { if (r) root.settings = r })
  }
  onRoomIdChanged: root.load()

  function setMyRole(level) {
    if (!root.svc) return
    root.busy = true
    root.svc.setPowerLevel(root.roomId, { userId: root.svc.userId, level: level }, function (r, e) {
      root.busy = false
      if (e) { root.note = "Could not change your role"; noteTimer.restart(); return }
      root.load()
    })
  }

  Rectangle { anchors.fill: parent; color: Qt.lighter(Color.menu.background, 1.35) }

  Column {
    anchors.fill: parent
    spacing: 0

    SettingsHeader { fg: root.fg; title: "Roles & permissions"; onBack: root.closed() }

    SettingsGroup {
      fg: root.fg
      title: "Roles"
      divided: false
      SettingsRow {
        fg: root.fg
        icon: Icons.shield; label: "Admins"
        trailing: "count"; value: String(root.admins)
        onClicked: root.openRole(100)
      }
      SettingsRow {
        fg: root.fg
        icon: Icons.moderator; label: "Moderators"
        trailing: "count"; value: String(root.moderators)
        onClicked: root.openRole(50)
      }
      SettingsRow {
        fg: root.fg
        icon: Icons.edit; label: "Change my role"
        // Only ever downward: Matrix will not let anyone raise themselves.
        enabled: root.myLevel > 0 && !root.busy
        onClicked: rolePick.open = true
      }
    }

    SettingsGroup {
      fg: root.fg
      SettingsRow {
        fg: root.fg
        icon: Icons.settings; label: "Permissions"
        trailing: "chevron"
        onClicked: root.openPermissions()
      }
    }

    SettingsGroup {
      fg: root.fg
      SettingsRow {
        fg: root.fg
        icon: Icons.trash; label: "Reset permissions"; danger: true
        enabled: !!root.can.setPowerLevels && !root.busy
        onClicked: confirmReset.open = true
      }
    }
  }

  ChoiceSheet {
    id: rolePick
    fg: root.fg
    title: "Change my role"
    value: root.myLevel >= 100 ? 100 : (root.myLevel >= 50 ? 50 : 0)
    model: {
      var out = []
      if (root.myLevel >= 100) out.push({ t: "Admin", v: 100, sub: "Can do everything, including changing roles." })
      if (root.myLevel >= 50) out.push({ t: "Moderator", v: 50, sub: "Can remove people and messages." })
      out.push({ t: "Member", v: 0, sub: "Can send messages." })
      return out
    }
    onChose: function (v) {
      if (v === root.myLevel) return
      demote.level = v
      demote.open = true
    }
  }

  // Demoting yourself is one-way when you are the only admin, so it asks.
  Item {
    id: demote
    property bool open: false
    property int level: 0
    anchors.fill: parent
    visible: open
    Rectangle { anchors.fill: parent; color: Qt.rgba(0, 0, 0, 0.5); MouseArea { anchors.fill: parent; onClicked: demote.open = false } }
    Rectangle {
      anchors.centerIn: parent
      width: Math.min(parent.width - Style.space(60), Style.space(320))
      height: dCol.implicitHeight + Style.space(32)
      radius: Style.space(18)
      color: Color.popups.background
      Column {
        id: dCol
        anchors.centerIn: parent
        width: parent.width - Style.space(36)
        spacing: Style.space(10)
        Text {
          width: parent.width; wrapMode: Text.Wrap
          text: "Become " + root.roleName(demote.level) + "?"
          color: root.fg
          font.family: Fonts.ui; font.pixelSize: Style.font.subtitle; font.bold: true
        }
        Text {
          width: parent.width; wrapMode: Text.Wrap
          text: root.admins <= 1 && root.myLevel >= 100
                ? "You are the only admin. Nobody will be able to give this back to you."
                : "Another admin will have to give this back to you."
          color: Util.alpha(root.fg, 0.6)
          font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
        }
        Row {
          anchors.right: parent.right
          spacing: Style.space(8)
          Rectangle {
            width: dc.implicitWidth + Style.space(26); height: Style.space(34); radius: height / 2
            color: dch.containsMouse ? Util.alpha(root.fg, 0.12) : Util.alpha(root.fg, 0.07)
            Text { id: dc; anchors.centerIn: parent; text: "Cancel"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall }
            MouseArea { id: dch; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: demote.open = false }
          }
          Rectangle {
            width: dk.implicitWidth + Style.space(26); height: Style.space(34); radius: height / 2
            color: dkh.containsMouse ? Qt.lighter(Color.urgent, 1.1) : Color.urgent
            Text { id: dk; anchors.centerIn: parent; text: "Change"; color: Color.background; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall; font.bold: true }
            MouseArea {
              id: dkh; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor
              onClicked: { demote.open = false; root.setMyRole(demote.level) }
            }
          }
        }
      }
    }
  }

  // Reset restores the Matrix defaults (50 for state, 0 to speak).
  Item {
    id: confirmReset
    property bool open: false
    anchors.fill: parent
    visible: open
    Rectangle { anchors.fill: parent; color: Qt.rgba(0, 0, 0, 0.5); MouseArea { anchors.fill: parent; onClicked: confirmReset.open = false } }
    Rectangle {
      anchors.centerIn: parent
      width: Math.min(parent.width - Style.space(60), Style.space(320))
      height: rCol.implicitHeight + Style.space(32)
      radius: Style.space(18)
      color: Color.popups.background
      Column {
        id: rCol
        anchors.centerIn: parent
        width: parent.width - Style.space(36)
        spacing: Style.space(10)
        Text {
          width: parent.width; wrapMode: Text.Wrap
          text: "Reset permissions?"; color: root.fg
          font.family: Fonts.ui; font.pixelSize: Style.font.subtitle; font.bold: true
        }
        Text {
          width: parent.width; wrapMode: Text.Wrap
          text: "Every capability goes back to the default level. Who holds which role stays as it is."
          color: Util.alpha(root.fg, 0.6)
          font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
        }
        Row {
          anchors.right: parent.right
          spacing: Style.space(8)
          Rectangle {
            width: rc.implicitWidth + Style.space(26); height: Style.space(34); radius: height / 2
            color: rch.containsMouse ? Util.alpha(root.fg, 0.12) : Util.alpha(root.fg, 0.07)
            Text { id: rc; anchors.centerIn: parent; text: "Cancel"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall }
            MouseArea { id: rch; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: confirmReset.open = false }
          }
          Rectangle {
            width: rk.implicitWidth + Style.space(26); height: Style.space(34); radius: height / 2
            color: rkh.containsMouse ? Qt.lighter(Color.urgent, 1.1) : Color.urgent
            Text { id: rk; anchors.centerIn: parent; text: "Reset"; color: Color.background; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall; font.bold: true }
            MouseArea {
              id: rkh; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor
              onClicked: {
                confirmReset.open = false
                if (!root.svc) return
                // Matrix defaults, sent one key at a time as the Permissions grid does.
                var defaults = { invite: 0, kick: 50, ban: 50, redact: 50,
                                 eventsDefault: 0, stateDefault: 50,
                                 name: 50, avatar: 50, topic: 50, liveLocation: 50 }
                var keys = Object.keys(defaults)
                var left = keys.length
                root.busy = true
                keys.forEach(function (k) {
                  root.svc.setPowerLevel(root.roomId, { key: k, level: defaults[k] }, function () {
                    if (--left === 0) { root.busy = false; root.load() }
                  })
                })
              }
            }
          }
        }
      }
    }
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
