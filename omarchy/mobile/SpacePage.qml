import QtQuick
import QtQuick.Controls as QQC
import Quickshell
import qs.Commons
import qs.Ui
import "../components"

// One space: its contents and the actions on it. Rows come from
// `space.hierarchy`, not the room list — a space's children include rooms this
// account has not joined, which have no `rooms` entry at all.
Item {
  id: root
  property var svc: null
  property string spaceId: ""
  property color fg: Color.menu.text

  /// Exposed so the panel's test hooks can raise the ⋮ menu without a mouse.
  property alias menuOpen: menu.open

  signal closed()
  signal openRoom(string roomId)
  signal createRoom()
  signal addExisting()
  signal manageRooms()
  signal viewMembers()
  signal openSettings()
  signal leftSpace()

  readonly property var space: {
    if (!root.svc) return null
    for (var i = 0; i < root.svc.spaces.length; i++)
      if (root.svc.spaces[i].id === root.spaceId) return root.svc.spaces[i]
    return null
  }
  /// The space's own room record once joined; the tree entry carries no join
  /// rule or member count.
  readonly property var room: root.svc ? root.svc.room(root.spaceId) : null
  readonly property string spaceName: root.space ? (root.space.name || root.spaceId)
                                     : (root.room ? (root.room.name || root.spaceId) : root.spaceId)

  property var children_: []
  property bool loading: false
  property bool loaded: false
  property string note: ""
  Timer { id: noteTimer; interval: 2600; onTriggered: root.note = "" }

  property var settings: ({})
  readonly property bool isPublic: (root.settings.joinRule || "") === "public"
  readonly property int memberCount: root.settings.memberCount !== undefined ? root.settings.memberCount
                                   : (root.room ? (root.room.joinedMembers || 0) : 0)

  function reset() {
    root.children_ = []
    root.loaded = false
    root.note = ""
    menu.open = false
    root.load()
  }

  function load() {
    if (!root.svc || !root.spaceId) return
    root.loading = true
    root.svc.spaceHierarchy(root.spaceId, function (r, e) {
      root.loading = false
      root.loaded = true
      root.children_ = (r && r.rooms) ? r.rooms : []
      if (e) { root.note = "Could not load this space"; noteTimer.restart() }
    })
    root.svc.roomSettings(root.spaceId, function (r, e) { if (r) root.settings = r })
  }

  onSpaceIdChanged: root.reset()

  // Adding or removing a child is a state event on the space and lands as a
  // spaces.tree push. Debounced: a bulk remove sends one per room.
  Timer { id: reload; interval: 700; onTriggered: root.load() }
  Connections {
    target: root.svc
    ignoreUnknownSignals: true
    function onSpacesChanged() { reload.restart() }
  }

  Rectangle { anchors.fill: parent; color: Qt.lighter(Color.menu.background, 1.35) }

  Column {
    anchors.fill: parent
    spacing: 0

    // Header
    Item {
      width: parent.width; height: Style.space(56)
      PanelActionButton {
        id: backBtn
        anchors.left: parent.left; anchors.leftMargin: Style.space(6)
        anchors.verticalCenter: parent.verticalCenter
        fontFamily: Fonts.icon; iconText: Icons.back; foreground: root.fg
        onClicked: root.closed()
      }
      Avatar {
        id: hdrAv
        anchors.left: backBtn.right; anchors.leftMargin: Style.space(4)
        anchors.verticalCenter: parent.verticalCenter
        size: Style.space(28)
        cornerRadius: Style.space(8)
        source: root.space ? (root.space.avatarPath || "") : ""
        name: root.spaceName
        userId: root.spaceId
      }
      Text {
        anchors.left: hdrAv.right; anchors.leftMargin: Style.space(10)
        anchors.right: moreBtn.left; anchors.rightMargin: Style.space(6)
        anchors.verticalCenter: parent.verticalCenter
        text: root.spaceName; color: root.fg; elide: Text.ElideRight
        font.family: Fonts.ui; font.pixelSize: Style.font.heading; font.bold: true
      }
      PanelActionButton {
        id: moreBtn
        anchors.right: parent.right; anchors.rightMargin: Style.space(6)
        anchors.verticalCenter: parent.verticalCenter
        fontFamily: Fonts.icon; iconText: Icons.moreVertical; foreground: root.fg
        onClicked: menu.open = !menu.open
      }
    }

    // Hero
    Item {
      width: parent.width
      height: heroCol.implicitHeight + Style.space(28)
      Column {
        id: heroCol
        anchors.centerIn: parent
        width: parent.width
        spacing: Style.space(6)
        Avatar {
          anchors.horizontalCenter: parent.horizontalCenter
          size: Style.space(72)
          cornerRadius: Style.space(20)
          source: root.space ? (root.space.avatarPath || "") : ""
          name: root.spaceName
          userId: root.spaceId
        }
        Text {
          width: parent.width - Style.space(48)
          anchors.horizontalCenter: parent.horizontalCenter
          horizontalAlignment: Text.AlignHCenter
          text: root.spaceName; color: root.fg; elide: Text.ElideRight
          font.family: Fonts.ui; font.pixelSize: Style.font.display; font.bold: true
        }
        Row {
          anchors.horizontalCenter: parent.horizontalCenter
          spacing: Style.space(5)
          IconLabel {
            anchors.verticalCenter: parent.verticalCenter
            icon: root.isPublic ? Icons.globe : Icons.lock
            color: Util.alpha(root.fg, 0.55); filled: true; size: Style.font.bodySmall
          }
          Text {
            anchors.verticalCenter: parent.verticalCenter
            text: root.isPublic ? "Public" : "Private"
            color: Util.alpha(root.fg, 0.55)
            font.family: Fonts.ui; font.pixelSize: Style.font.body
          }
        }
        Rectangle {
          anchors.horizontalCenter: parent.horizontalCenter
          width: chipRow.implicitWidth + Style.space(18); height: Style.space(28)
          radius: height / 2
          color: memHover.containsMouse ? Util.alpha(root.fg, 0.12) : Util.alpha(root.fg, 0.07)
          Row {
            id: chipRow
            anchors.centerIn: parent
            spacing: Style.space(5)
            IconLabel {
              anchors.verticalCenter: parent.verticalCenter
              icon: Icons.person; color: Util.alpha(root.fg, 0.7)
              filled: true; size: Style.font.bodySmall
            }
            Text {
              anchors.verticalCenter: parent.verticalCenter
              text: String(root.memberCount)
              color: Util.alpha(root.fg, 0.7)
              font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
            }
          }
          MouseArea {
            id: memHover
            anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor
            onClicked: root.viewMembers()
          }
        }
      }
    }

    // Rooms
    Item {
      width: parent.width
      height: parent.height - y

      Rectangle {
        anchors.fill: parent
        topLeftRadius: Style.space(24); topRightRadius: Style.space(24)
        antialiasing: true
        color: Qt.darker(Color.menu.background, 1.35)
      }

      // Empty and loading say different things.
      Column {
        anchors.centerIn: parent
        width: parent.width - Style.space(60)
        spacing: Style.space(10)
        visible: root.children_.length === 0
        IconLabel {
          anchors.horizontalCenter: parent.horizontalCenter
          icon: Icons.space; color: Util.alpha(Color.accent, 0.8); size: Style.space(44)
        }
        Text {
          width: parent.width; horizontalAlignment: Text.AlignHCenter
          text: root.loading || !root.loaded ? "Looking inside…" : "No rooms in this space"
          color: root.fg
          font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: true
        }
        Text {
          width: parent.width; horizontalAlignment: Text.AlignHCenter; wrapMode: Text.Wrap
          visible: root.loaded && !root.loading
          text: "Create a room here, or add one you are already in."
          color: Util.alpha(root.fg, 0.6)
          font.family: Fonts.ui; font.pixelSize: Style.font.caption
        }
      }

      ListView {
        id: list
        anchors.fill: parent
        anchors.topMargin: Style.space(6)
        visible: root.children_.length > 0
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        QQC.ScrollBar.vertical: ScrollBarStyle {}
        model: root.children_

        delegate: Item {
          id: row
          required property var modelData
          width: list.width
          height: Style.space(64)

          Rectangle {
            anchors.fill: parent
            anchors.margins: Style.space(4)
            anchors.leftMargin: Style.space(8); anchors.rightMargin: Style.space(8)
            radius: Style.space(14)
            color: rh.containsMouse ? Util.alpha(root.fg, 0.05) : "transparent"
          }

          Avatar {
            id: av
            anchors.left: parent.left; anchors.leftMargin: Style.space(16)
            anchors.verticalCenter: parent.verticalCenter
            size: Style.space(44)
            cornerRadius: row.modelData.isSpace ? Style.space(12) : -1
            source: row.modelData.avatarPath || ""
            name: row.modelData.name || row.modelData.id
            userId: row.modelData.id
          }

          Column {
            anchors.left: av.right; anchors.leftMargin: Style.space(12)
            anchors.right: joinBtn.visible ? joinBtn.left : parent.right
            anchors.rightMargin: Style.space(12)
            anchors.verticalCenter: parent.verticalCenter
            spacing: Style.space(2)
            Text {
              width: parent.width; elide: Text.ElideRight
              text: row.modelData.name || row.modelData.id
              color: root.fg
              font.family: Fonts.ui; font.pixelSize: Style.font.subtitle; font.bold: true
            }
            Row {
              spacing: Style.space(5)
              IconLabel {
                anchors.verticalCenter: parent.verticalCenter
                icon: row.modelData.worldReadable ? Icons.globe : Icons.lock
                color: Util.alpha(root.fg, 0.45); filled: true; size: Style.font.caption
              }
              Text {
                anchors.verticalCenter: parent.verticalCenter
                text: row.modelData.worldReadable ? "Public" : "Private"
                color: Util.alpha(root.fg, 0.55)
                font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
              }
            }
            Text {
              text: {
                var n = row.modelData.memberCount || 0
                return n === 1 ? "1 Member" : n + " Members"
              }
              color: Util.alpha(root.fg, 0.55)
              font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
            }
          }

          Rectangle {
            id: joinBtn
            visible: !row.modelData.joined
            anchors.right: parent.right; anchors.rightMargin: Style.space(16)
            anchors.verticalCenter: parent.verticalCenter
            width: joinText.implicitWidth + Style.space(22); height: Style.space(30)
            radius: height / 2
            color: jh.containsMouse ? Qt.lighter(Color.accent, 1.1) : Color.accent
            Text {
              id: joinText
              anchors.centerIn: parent
              text: "Join"; color: Color.background
              font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall; font.bold: true
            }
            MouseArea {
              id: jh
              anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor
              onClicked: {
                if (!root.svc) return
                root.svc.joinRoom(row.modelData.id, function (r, e) {
                  if (e) { root.note = "Could not join " + (row.modelData.name || "that room"); noteTimer.restart() }
                  else root.load()
                })
              }
            }
          }

          MouseArea {
            id: rh
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            enabled: !!row.modelData.joined
            onClicked: root.openRoom(row.modelData.id)
          }
        }
      }
    }
  }

  OverflowMenu {
    id: menu
    fg: root.fg
    topInset: Style.space(48)
    model: [
      { t: "Create room",       a: "create",  icon: Icons.plus },
      { t: "Add existing rooms", a: "add",    icon: Icons.hash },
      { t: "Manage rooms",      a: "manage",  icon: Icons.edit },
      { t: "View members",      a: "members", icon: Icons.person },
      { t: "Share",             a: "share",   icon: Icons.share },
      { t: "Settings",          a: "settings", icon: Icons.settings },
      { t: "Leave space",       a: "leave",   icon: Icons.leave, danger: true }
    ]
    onPicked: function (a) {
      if (a === "create") root.createRoom()
      else if (a === "add") root.addExisting()
      else if (a === "manage") root.manageRooms()
      else if (a === "members") root.viewMembers()
      else if (a === "settings") root.openSettings()
      else if (a === "share") {
        // Shareable identity: the alias when there is one, else a matrix.to room id.
        var alias = root.settings.canonicalAlias || root.spaceId
        Quickshell.execDetached(["sh", "-c", 'printf "%s" "$1" | wl-copy', "copy", "https://matrix.to/#/" + alias])
        root.note = "Link copied"; noteTimer.restart()
      }
      else if (a === "leave") confirmLeave.open = true
    }
  }

  // Leaving is irreversible, so it asks, and the wording names the space.
  Item {
    id: confirmLeave
    property bool open: false
    anchors.fill: parent
    visible: open
    Rectangle { anchors.fill: parent; color: Qt.rgba(0, 0, 0, 0.5); MouseArea { anchors.fill: parent; onClicked: confirmLeave.open = false } }
    Rectangle {
      anchors.centerIn: parent
      width: Math.min(parent.width - Style.space(60), Style.space(320))
      height: cCol.implicitHeight + Style.space(32)
      radius: Style.space(18)
      color: Color.popups.background
      Column {
        id: cCol
        anchors.centerIn: parent
        width: parent.width - Style.space(36)
        spacing: Style.space(10)
        Text {
          width: parent.width; wrapMode: Text.Wrap
          text: "Leave " + root.spaceName + "?"
          color: root.fg
          font.family: Fonts.ui; font.pixelSize: Style.font.subtitle; font.bold: true
        }
        Text {
          width: parent.width; wrapMode: Text.Wrap
          text: "You will stay in the rooms inside it that you have joined."
          color: Util.alpha(root.fg, 0.6)
          font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
        }
        Row {
          anchors.right: parent.right
          spacing: Style.space(8)
          Rectangle {
            width: cancelT.implicitWidth + Style.space(26); height: Style.space(34); radius: height / 2
            color: ch.containsMouse ? Util.alpha(root.fg, 0.12) : Util.alpha(root.fg, 0.07)
            Text { id: cancelT; anchors.centerIn: parent; text: "Cancel"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall }
            MouseArea { id: ch; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: confirmLeave.open = false }
          }
          Rectangle {
            width: leaveT.implicitWidth + Style.space(26); height: Style.space(34); radius: height / 2
            color: lh.containsMouse ? Qt.lighter(Color.urgent, 1.1) : Color.urgent
            Text { id: leaveT; anchors.centerIn: parent; text: "Leave"; color: Color.background; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall; font.bold: true }
            MouseArea {
              id: lh
              anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor
              onClicked: {
                confirmLeave.open = false
                if (!root.svc) return
                root.svc.request("room.leave", { roomId: root.spaceId }, function (r, e) {
                  if (e) { root.note = "Could not leave"; noteTimer.restart() }
                  else root.leftSpace()
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
    width: noteT.implicitWidth + Style.space(28); height: Style.space(34)
    radius: height / 2
    color: Color.popups.background
    opacity: root.note !== "" ? 1 : 0
    visible: opacity > 0.01
    Behavior on opacity { NumberAnimation { duration: 150 } }
    Text { id: noteT; anchors.centerIn: parent; text: root.note; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall }
  }
}
