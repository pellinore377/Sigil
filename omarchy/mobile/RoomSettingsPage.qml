import QtQuick
import qs.Commons
import qs.Ui
import "../components"
import ".."

// Room settings: identity, quick toggles, add people, leave.
Item {
  id: root
  property var svc: null
  property color fg: Color.menu.text
  property string roomId: ""
  readonly property int pinnedCount: {
    if (!root.svc) return 0
    var n = 0
    for (var i = 0; i < root.svc.rooms.length; i++) if (root.svc.rooms[i].isFavourite) n++
    return n
  }
  property string note: ""
  Timer { id: noteTimer; interval: 2600; onTriggered: root.note = "" }
  readonly property var room: svc ? svc.room(roomId) : null
  signal closed()
  signal addPeople()
  signal leftRoom()
  signal openNotifications()
  signal openSecurity()
  signal openRoles()

  /// Read once for the value labels below; the pages themselves re-read on open.
  property var settings: ({})
  function loadSettings() {
    if (!root.svc || !root.roomId) return
    root.svc.roomSettings(root.roomId, function (r) { if (r) root.settings = r })
  }
  property bool confirmLeave: false
  function reset() { root.confirmLeave = false; root.loadMembers(); root.loadSettings() }

  /// Everyone in the room. Group rooms only — a DM has two people and one is you.
  readonly property bool isGroup: !!(root.room && !root.room.isDm)
  readonly property var members: (root.svc && root.roomId) ? (root.svc.membersByRoom[root.roomId] || []) : []
  function loadMembers() {
    if (!root.svc || !root.roomId || !root.isGroup) return
    if (root.members.length > 0) return
    root.svc.fetchMembers(root.roomId, function () {})
  }
  onRoomIdChanged: { root.loadMembers(); root.loadSettings() }
  // Also on show: `roomId` has usually settled before Settings is opened, so
  // its change alone left the member list empty.
  onVisibleChanged: if (visible) { root.loadMembers(); root.loadSettings() }

  Flickable {
    anchors.fill: parent
    contentWidth: width
    contentHeight: settingsCol.implicitHeight + Style.space(20)
    clip: true
    boundsBehavior: Flickable.StopAtBounds

    Column {
      id: settingsCol
      width: parent.width
      Item {
        width: parent.width; height: Style.space(54)
        PanelActionButton { id: backBtn; anchors.left: parent.left; anchors.leftMargin: Style.space(6); anchors.verticalCenter: parent.verticalCenter; fontFamily: Fonts.icon; iconText: Icons.back; foreground: root.fg; onClicked: root.closed() }
        Text { anchors.left: backBtn.right; anchors.leftMargin: Style.space(6); anchors.verticalCenter: parent.verticalCenter; text: "Settings"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.heading; font.bold: true }
      }
      Item { width: parent.width; height: Style.space(16) }
      Avatar { anchors.horizontalCenter: parent.horizontalCenter; size: Style.space(72); source: root.room ? (root.room.avatarPath || "") : ""; name: root.room ? root.room.name : ""; userId: root.room ? (root.room.isDm ? (root.room.dmUserId || root.room.id) : root.room.id) : "" }
      Item { width: parent.width; height: Style.space(8) }
      Text { width: parent.width; horizontalAlignment: Text.AlignHCenter; elide: Text.ElideRight; text: root.room ? (root.room.name || root.room.id) : ""; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.title; font.bold: true }
      Text { width: parent.width; horizontalAlignment: Text.AlignHCenter; elide: Text.ElideRight; visible: text !== ""; text: root.room ? (root.room.isDm ? (root.room.dmUserId || "") : ((root.room.joinedMembers || 0) + " members")) : ""; color: Util.alpha(root.fg, 0.55); font.family: Fonts.ui; font.pixelSize: Style.font.caption; topPadding: Style.space(2) }
      Text { width: parent.width - Style.space(60); anchors.horizontalCenter: parent.horizontalCenter; horizontalAlignment: Text.AlignHCenter; visible: text !== ""; text: root.room ? (root.room.topic || "") : ""; color: Util.alpha(root.fg, 0.6); font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall; wrapMode: Text.Wrap; maximumLineCount: 3; elide: Text.ElideRight; topPadding: Style.space(6) }
      Item { width: parent.width; height: Style.space(18) }
      // rows
      Repeater {
        model: [
          { t: "Add people", icon: Icons.plus, a: "add", danger: false, toggled: false, hasToggle: false },
          { t: "Pin", icon: Icons.pin, a: "fav", danger: false, hasToggle: true },
          { t: "Low priority", icon: Icons.lowPriority, a: "low", danger: false, hasToggle: true }
        ]
        delegate: Item {
          required property var modelData
          width: parent.width; height: Style.space(48)
          readonly property bool on: root.room ? (modelData.a === "fav" ? !!root.room.isFavourite : !!root.room.isLowPriority) : false
          Rectangle { anchors.fill: parent; anchors.margins: Style.space(4); anchors.leftMargin: Style.space(10); anchors.rightMargin: Style.space(10); radius: Style.space(12); color: rh.containsMouse ? Util.alpha(root.fg, 0.05) : "transparent" }
          IconLabel { anchors.left: parent.left; anchors.leftMargin: Style.space(22); anchors.verticalCenter: parent.verticalCenter; icon: modelData.icon; color: Util.alpha(root.fg, 0.75); size: Style.font.icon }
          Text { anchors.left: parent.left; anchors.leftMargin: Style.space(56); anchors.verticalCenter: parent.verticalCenter; text: modelData.t; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body }
          Rectangle {
            visible: modelData.hasToggle
            anchors.right: parent.right; anchors.rightMargin: Style.space(20); anchors.verticalCenter: parent.verticalCenter
            width: Style.space(38); height: Style.space(22); radius: height / 2
            color: on ? Util.alpha(Color.accent, 0.8) : Util.alpha(root.fg, 0.15)
            Rectangle { width: Style.space(16); height: Style.space(16); radius: width / 2; anchors.verticalCenter: parent.verticalCenter; x: on ? parent.width - width - Style.space(3) : Style.space(3); color: root.fg; Behavior on x { NumberAnimation { duration: 120 } } }
          }
          MouseArea {
            id: rh; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor
            onClicked: {
              if (!root.svc) return
              if (modelData.a === "add") root.addPeople()
              else if (modelData.a === "fav") {
                // Five pins max: refuse the sixth rather than silently dropping one.
                if (!root.room.isFavourite && root.pinnedCount >= 5) { root.note = "Five conversations are already pinned"; noteTimer.restart(); return }
                root.svc.setFavourite(root.roomId, !root.room.isFavourite)
              }
              else if (modelData.a === "low") root.svc.request("room.setLowPriority", { roomId: root.roomId, lowPriority: !root.room.isLowPriority })
            }
          }
        }
      }
      // Members
      Item { width: parent.width; height: root.isGroup ? Style.space(16) : 0; visible: root.isGroup }
      Text {
        visible: root.isGroup
        x: Style.space(22)
        text: root.members.length > 0 ? root.members.length + (root.members.length === 1 ? " member" : " members")
                                      : "Members"
        color: Util.alpha(root.fg, 0.55)
        font.family: Fonts.ui; font.pixelSize: Style.font.caption; font.bold: true
      }
      Item { width: parent.width; height: root.isGroup ? Style.space(6) : 0; visible: root.isGroup }
      Repeater {
        model: root.isGroup ? root.members : []
        delegate: Item {
          required property var modelData
          width: parent.width; height: Style.space(52)
          Avatar {
            id: mFace
            anchors.left: parent.left; anchors.leftMargin: Style.space(24)
            anchors.verticalCenter: parent.verticalCenter
            size: Style.space(34)
            source: modelData.avatarPath || ""
            name: modelData.displayName || ""
            userId: modelData.userId || ""
            status: root.svc ? root.svc.presenceOf(modelData.userId) : ""
          }
          Column {
            anchors.left: mFace.right; anchors.leftMargin: Style.space(12)
            anchors.right: mPower.left; anchors.rightMargin: Style.space(10)
            anchors.verticalCenter: parent.verticalCenter
            spacing: Style.space(1)
            Text {
              width: parent.width; elide: Text.ElideRight
              text: modelData.displayName || modelData.userId || ""
              color: root.fg
              font.family: Fonts.ui; font.pixelSize: Style.font.body
            }
            Text {
              width: parent.width; elide: Text.ElideMiddle
              visible: text !== "" && text !== (modelData.displayName || "")
              text: modelData.userId || ""
              color: Util.alpha(root.fg, 0.5)
              font.family: Fonts.ui; font.pixelSize: Style.font.caption
            }
          }
          Text {
            id: mPower
            anchors.right: parent.right; anchors.rightMargin: Style.space(24)
            anchors.verticalCenter: parent.verticalCenter
            readonly property int lvl: modelData.powerLevel || 0
            visible: lvl >= 50
            text: lvl >= 100 ? "Admin" : "Mod"
            color: Util.alpha(root.fg, 0.55)
            font.family: Fonts.ui; font.pixelSize: Style.font.caption
          }
        }
      }

      // Shared settings pages — the same four a space reaches from its Settings.
      Item { width: parent.width; height: Style.space(10) }
      SettingsGroup {
        fg: root.fg
        SettingsRow {
          fg: root.fg
          icon: Icons.bell; label: "Notifications"
          trailing: "value"
          // Blank until the read lands: a default caption asserts facts it lacks.
          value: {
            if (!root.settings.id) return ""
            var m = root.settings.notificationMode
            return m === "all" ? "All messages" : m === "mentions" ? "Mentions only" : m === "mute" ? "Muted" : "Default"
          }
          onClicked: root.openNotifications()
        }
        SettingsRow {
          fg: root.fg
          icon: Icons.lock; label: "Security & privacy"
          trailing: "value"
          value: !root.settings.id ? ""
               : root.settings.joinRule === "public" ? "Public"
               : root.settings.joinRule === "restricted" ? "Space members" : "Invite only"
          onClicked: root.openSecurity()
        }
        SettingsRow {
          fg: root.fg
          icon: Icons.shield; label: "Roles & permissions"
          trailing: "chevron"
          onClicked: root.openRoles()
        }
      }

      Item { width: parent.width; height: Style.space(14) }
      // leave
      // Spaces — a space's contents are state on the SPACE, so a room lists here.
      Item {
        width: parent.width
        height: spaceCol.implicitHeight + Style.space(10)
        visible: !!(root.svc && root.svc.spaces.length > 0) && !(root.room && root.room.isDm)
        Column {
          id: spaceCol
          anchors.left: parent.left; anchors.right: parent.right
          spacing: Style.space(4)
          Text {
            x: Style.space(22)
            text: "Spaces"
            color: Util.alpha(root.fg, 0.55)
            font.family: Fonts.ui; font.pixelSize: Style.font.caption; font.bold: true
          }
          Repeater {
            model: root.svc ? root.svc.spaces : []
            delegate: Item {
              required property var modelData
              width: spaceCol.width
              height: Style.space(44)
              readonly property bool inSpace: (modelData.children || []).indexOf(root.roomId) >= 0
              Rectangle {
                anchors.fill: parent
                anchors.margins: Style.space(2)
                anchors.leftMargin: Style.space(10); anchors.rightMargin: Style.space(10)
                radius: Style.space(12)
                color: sph.containsMouse ? Util.alpha(root.fg, 0.05) : "transparent"
              }
              Avatar {
                id: spAv
                anchors.left: parent.left; anchors.leftMargin: Style.space(18)
                anchors.verticalCenter: parent.verticalCenter
                size: Style.space(28)
                source: modelData.avatarPath || ""
                name: modelData.name || ""
                userId: modelData.id
              }
              Text {
                anchors.left: spAv.right; anchors.leftMargin: Style.space(10)
                anchors.right: spTick.left; anchors.rightMargin: Style.space(8)
                anchors.verticalCenter: parent.verticalCenter
                elide: Text.ElideRight
                text: modelData.name || modelData.id
                color: root.fg
                font.family: Fonts.ui; font.pixelSize: Style.font.body
              }
              IconLabel { id: spTick
                anchors.right: parent.right; anchors.rightMargin: Style.space(22)
                anchors.verticalCenter: parent.verticalCenter
                icon: parent.inSpace ? Icons.check : Icons.plus
                color: parent.inSpace ? Color.accent : Util.alpha(root.fg, 0.45); size: Style.font.icon }
              MouseArea {
                id: sph
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: {
                  if (!root.svc) return
                  if (parent.inSpace) root.svc.removeRoomFromSpace(modelData.id, root.roomId, function (r, e) {})
                  else root.svc.addRoomToSpace(modelData.id, root.roomId, function (r, e) {})
                }
              }
            }
          }
        }
      }

      Rectangle {
        anchors.horizontalCenter: parent.horizontalCenter
        width: parent.width - Style.space(40); height: Style.space(44); radius: Style.space(14)
        antialiasing: true
        color: root.confirmLeave ? Util.alpha(Color.urgent, 0.85) : Util.alpha(Color.urgent, 0.16)
        Text { anchors.centerIn: parent; text: root.confirmLeave ? "Tap again to confirm" : (root.room && root.room.isDm ? "Leave chat" : "Leave room"); color: root.confirmLeave ? Color.background : Color.urgent; font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: true }
        MouseArea {
          anchors.fill: parent; cursorShape: Qt.PointingHandCursor
          onClicked: {
            if (!root.confirmLeave) { root.confirmLeave = true; confirmTimer.restart(); return }
            if (root.svc) root.svc.leaveRoom(root.roomId, function(r, e) {})
            root.confirmLeave = false
            root.leftRoom()
          }
        }
      }
      Timer { id: confirmTimer; interval: 4000; onTriggered: root.confirmLeave = false }
    }
  }

  // Transient notice (pin cap, mostly).
  Rectangle {
    visible: root.note !== ""
    anchors.horizontalCenter: parent.horizontalCenter
    anchors.bottom: parent.bottom
    anchors.bottomMargin: Style.space(22)
    width: noteText.implicitWidth + Style.space(26)
    height: Style.space(34)
    radius: height / 2
    color: Util.alpha(Color.popups.background, 0.98)
    border.width: 1; border.color: Util.alpha(root.fg, 0.12)
    opacity: root.note !== "" ? 1 : 0
    Behavior on opacity { NumberAnimation { duration: 140 } }
    Text { id: noteText; anchors.centerIn: parent; text: root.note; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.caption }
  }
}
