import QtQuick
import QtQuick.Controls as QQC
import qs.Commons
import qs.Ui
import "../components"

// The people in a room or a space, optionally narrowed to one role.
// `filterLevel` is what makes "Admins 1" on the roles page a link: the same
// list, filtered.
Item {
  id: root
  property var svc: null
  property string roomId: ""
  property color fg: Color.menu.text
  /// -1 shows everyone; 100 or 50 narrows to that role.
  property int filterLevel: -1

  signal closed()
  signal invite()

  readonly property var all: (root.svc && root.roomId) ? (root.svc.membersByRoom[root.roomId] || []) : []
  readonly property var members: {
    if (root.filterLevel < 0) return root.all
    if (root.filterLevel >= 100) return root.all.filter(function (m) { return (m.powerLevel || 0) >= 100 })
    return root.all.filter(function (m) { return (m.powerLevel || 0) >= 50 && (m.powerLevel || 0) < 100 })
  }
  readonly property string title: root.filterLevel >= 100 ? "Admins"
                                : (root.filterLevel >= 50 ? "Moderators" : "Members")

  function roleName(l) { return l >= 100 ? "Admin" : (l >= 50 ? "Moderator" : "") }

  function reset() { root.load() }
  function load() {
    if (!root.svc || !root.roomId) return
    root.svc.fetchMembers(root.roomId, function () {})
  }
  onRoomIdChanged: root.load()
  onVisibleChanged: if (visible) root.load()

  Rectangle { anchors.fill: parent; color: Qt.lighter(Color.menu.background, 1.35) }

  Column {
    anchors.fill: parent
    spacing: 0

    Item {
      width: parent.width; height: Style.space(56)
      PanelActionButton {
        id: backBtn
        anchors.left: parent.left; anchors.leftMargin: Style.space(6)
        anchors.verticalCenter: parent.verticalCenter
        fontFamily: Fonts.icon; iconText: Icons.back; foreground: root.fg
        onClicked: root.closed()
      }
      Column {
        anchors.left: backBtn.right; anchors.leftMargin: Style.space(6)
        anchors.right: inviteBtn.left; anchors.rightMargin: Style.space(8)
        anchors.verticalCenter: parent.verticalCenter
        Text {
          width: parent.width; elide: Text.ElideRight
          text: root.title; color: root.fg
          font.family: Fonts.ui; font.pixelSize: Style.font.heading; font.bold: true
        }
        Text {
          width: parent.width; elide: Text.ElideRight
          text: root.members.length === 1 ? "1 person" : root.members.length + " people"
          color: Util.alpha(root.fg, 0.55)
          font.family: Fonts.ui; font.pixelSize: Style.font.caption
        }
      }
      PanelActionButton {
        id: inviteBtn
        anchors.right: parent.right; anchors.rightMargin: Style.space(6)
        anchors.verticalCenter: parent.verticalCenter
        visible: root.filterLevel < 0
        fontFamily: Fonts.icon; iconText: Icons.personAdd; foreground: root.fg
        tooltipText: "Invite"
        onClicked: root.invite()
      }
    }

    Item {
      width: parent.width
      height: parent.height - y

      Rectangle {
        anchors.fill: parent
        topLeftRadius: Style.space(24); topRightRadius: Style.space(24)
        antialiasing: true
        color: Qt.darker(Color.menu.background, 1.35)
      }

      Text {
        anchors.centerIn: parent
        visible: root.members.length === 0
        text: root.all.length === 0 ? "Loading…" : ("Nobody is " + (root.filterLevel >= 100 ? "an admin" : "a moderator") + " here")
        color: Util.alpha(root.fg, 0.6)
        font.family: Fonts.ui; font.pixelSize: Style.font.body
      }

      ListView {
        anchors.fill: parent
        anchors.topMargin: Style.space(6)
        visible: root.members.length > 0
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        QQC.ScrollBar.vertical: ScrollBarStyle {}
        model: root.members

        delegate: Item {
          id: row
          required property var modelData
          width: ListView.view.width
          height: Style.space(58)

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
            size: Style.space(40)
            source: row.modelData.avatarPath || ""
            name: row.modelData.displayName || row.modelData.userId
            userId: row.modelData.userId
          }

          Column {
            anchors.left: av.right; anchors.leftMargin: Style.space(12)
            anchors.right: role.left; anchors.rightMargin: Style.space(10)
            anchors.verticalCenter: parent.verticalCenter
            spacing: Style.space(2)
            Text {
              width: parent.width; elide: Text.ElideRight
              text: row.modelData.displayName || row.modelData.userId
              color: root.fg
              font.family: Fonts.ui; font.pixelSize: Style.font.body
            }
            Text {
              width: parent.width; elide: Text.ElideRight
              visible: !!row.modelData.isNameAmbiguous || !row.modelData.displayName
              text: row.modelData.userId
              color: Util.alpha(root.fg, 0.5)
              font.family: Fonts.ui; font.pixelSize: Style.font.caption
            }
          }

          Text {
            id: role
            anchors.right: parent.right; anchors.rightMargin: Style.space(20)
            anchors.verticalCenter: parent.verticalCenter
            visible: text !== "" && root.filterLevel < 0
            text: root.roleName(row.modelData.powerLevel || 0)
            color: Util.alpha(Color.accent, 0.9)
            font.family: Fonts.ui; font.pixelSize: Style.font.caption; font.bold: true
          }

          MouseArea { id: rh; anchors.fill: parent; hoverEnabled: true }
        }
      }
    }
  }
}
