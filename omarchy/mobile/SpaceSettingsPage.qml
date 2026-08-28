import QtQuick
import qs.Commons
import qs.Ui
import "../components"

// A space's settings: its identity, and the four pages a room shares with it.
// Name, topic and picture are edited in place; Save appears once something differs.
Item {
  id: root
  property var svc: null
  property string roomId: ""
  property color fg: Color.menu.text

  signal closed()
  signal pickAvatar()
  signal openNotifications()
  signal openSecurity()
  signal openRoles()
  signal openMembers()

  property var settings: ({})
  property bool busy: false
  property string error: ""
  /// Set by the panel once the portal file chooser returns.
  property string newAvatarPath: ""

  readonly property var can: root.settings.can || ({})
  readonly property bool dirty: nameField.text.trim() !== (root.settings.name || "")
                             || topicField.text.trim() !== (root.settings.topic || "")
                             || root.newAvatarPath !== ""

  function reset() { root.error = ""; root.newAvatarPath = ""; root.load() }
  function load() {
    if (!root.svc || !root.roomId) return
    root.svc.roomSettings(root.roomId, function (r, e) {
      if (!r) { root.error = "Could not read settings"; return }
      root.settings = r
      nameField.text = r.name || ""
      topicField.text = r.topic || ""
      root.newAvatarPath = ""
    })
  }
  onRoomIdChanged: root.load()

  function save() {
    if (!root.dirty || root.busy || !root.svc) return
    root.busy = true
    var fields = {}
    if (nameField.text.trim() !== (root.settings.name || "")) fields.name = nameField.text.trim()
    if (topicField.text.trim() !== (root.settings.topic || "")) fields.topic = topicField.text.trim()
    var avatar = root.newAvatarPath
    var finish = function () { root.busy = false; root.load() }
    var afterFields = function (r, e) {
      if (e) { root.busy = false; root.error = e.message || "Could not save"; return }
      root.error = ""
      // The avatar is a separate state event with its own upload, so it is a
      // second call: doing it first orphans the upload if the name change fails.
      if (avatar !== "") root.svc.setRoomAvatar(root.roomId, avatar, finish)
      else finish()
    }
    if (Object.keys(fields).length > 0) root.svc.setRoomSettings(root.roomId, fields, afterFields)
    else afterFields(null, null)
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

      SettingsHeader {
        fg: root.fg
        title: "Space settings"
        action: root.busy ? "Saving…" : "Save"
        actionEnabled: root.dirty && !root.busy
        onBack: root.closed()
        onActioned: root.save()
      }

      Item { width: parent.width; height: Style.space(14) }

      Item {
        width: parent.width; height: Style.space(80)
        Avatar {
          id: av
          anchors.centerIn: parent
          size: Style.space(76)
          cornerRadius: Style.space(22)
          source: root.newAvatarPath !== "" ? root.newAvatarPath : (root.settings.avatarPath || "")
          name: nameField.text
          userId: root.roomId
        }
        Rectangle {
          visible: !!root.can.setAvatar
          anchors.right: av.right; anchors.bottom: av.bottom
          anchors.rightMargin: -Style.space(2); anchors.bottomMargin: -Style.space(2)
          width: Style.space(28); height: width; radius: width / 2
          color: Color.accent
          IconLabel { anchors.centerIn: parent; icon: Icons.camera; color: Color.background; filled: true; size: Style.font.bodySmall }
        }
        MouseArea {
          anchors.centerIn: parent
          width: av.width + Style.space(10); height: av.height + Style.space(10)
          enabled: !!root.can.setAvatar
          cursorShape: Qt.PointingHandCursor
          onClicked: root.pickAvatar()
        }
      }

      Item { width: parent.width; height: Style.space(10) }

      // Identity
      Column {
        width: parent.width
        spacing: Style.space(6)
        Text {
          x: Style.space(22)
          text: "Name"; color: Util.alpha(root.fg, 0.55)
          font.family: Fonts.ui; font.pixelSize: Style.font.caption; font.bold: true
        }
        Rectangle {
          x: Style.space(22)
          width: parent.width - Style.space(44); height: Style.space(44)
          radius: Style.space(10)
          color: "transparent"
          border.width: Math.max(1, Style.space(1))
          border.color: nameField.activeFocus ? Util.alpha(Color.accent, 0.8) : Util.alpha(root.fg, 0.2)
          TextInput {
            id: nameField
            anchors.fill: parent
            anchors.leftMargin: Style.space(12); anchors.rightMargin: Style.space(12)
            verticalAlignment: TextInput.AlignVCenter
            enabled: !!root.can.setName
            color: root.can.setName ? root.fg : Util.alpha(root.fg, 0.5)
            clip: true
            font.family: Fonts.ui; font.pixelSize: Style.font.body
          }
        }
        Item { width: 1; height: Style.space(6) }
        Text {
          x: Style.space(22)
          text: "Topic"; color: Util.alpha(root.fg, 0.55)
          font.family: Fonts.ui; font.pixelSize: Style.font.caption; font.bold: true
        }
        Rectangle {
          x: Style.space(22)
          width: parent.width - Style.space(44); height: Style.space(64)
          radius: Style.space(10)
          color: "transparent"
          border.width: Math.max(1, Style.space(1))
          border.color: topicField.activeFocus ? Util.alpha(Color.accent, 0.8) : Util.alpha(root.fg, 0.2)
          TextEdit {
            id: topicField
            anchors.fill: parent
            anchors.margins: Style.space(12)
            enabled: !!root.can.setTopic
            color: root.can.setTopic ? root.fg : Util.alpha(root.fg, 0.5)
            clip: true; wrapMode: TextEdit.Wrap
            font.family: Fonts.ui; font.pixelSize: Style.font.body
            Text {
              visible: topicField.text === ""
              text: "What is this space about?"; color: Util.alpha(root.fg, 0.4)
              font.family: Fonts.ui; font.pixelSize: Style.font.body
            }
          }
        }
        Item { width: 1; height: Style.space(14) }
      }

      SettingsGroup {
        fg: root.fg
        SettingsRow {
          fg: root.fg
          icon: Icons.person; label: "People"
          trailing: "count"; value: String(root.settings.memberCount || 0)
          onClicked: root.openMembers()
        }
        SettingsRow {
          fg: root.fg
          icon: Icons.shield; label: "Roles & permissions"
          trailing: "chevron"
          onClicked: root.openRoles()
        }
      }

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
}
