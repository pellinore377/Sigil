import QtQuick
import qs.Commons
import qs.Ui
import "../components"

// Create a space: picture, name, topic, and who can get in. Access is asked
// here because a space created private and opened later keeps its old history
// rules.
Item {
  id: root
  property var svc: null
  property color fg: Color.menu.text
  /// Set by the panel once the portal file chooser returns.
  property string avatarPath: ""

  signal closed()
  signal pickAvatar()
  signal created(string spaceId)

  property bool isPrivate: true
  property bool busy: false
  property string error: ""

  readonly property bool valid: nameField.text.trim() !== "" && !root.busy

  function reset() {
    nameField.text = ""
    topicField.text = ""
    root.isPrivate = true
    root.avatarPath = ""
    root.busy = false
    root.error = ""
  }
  function focusInput() { nameField.forceActiveFocus() }

  function create() {
    if (!root.valid || !root.svc) return
    root.busy = true
    root.error = ""
    root.svc.createSpace({
      name: nameField.text.trim(),
      topic: topicField.text.trim(),
      private: root.isPrivate
    }, function (r, e) {
      root.busy = false
      if (e || !r || !r.roomId) { root.error = (e && e.message) ? e.message : "Could not create the space"; return }
      // The avatar is a second call on purpose: create_room has no avatar field,
      // and uploading first leaves an orphan upload when creation fails.
      if (root.avatarPath !== "") root.svc.setRoomAvatar(r.roomId, root.avatarPath, function () {})
      root.created(r.roomId)
    })
  }

  Rectangle { anchors.fill: parent; color: Qt.lighter(Color.menu.background, 1.35) }

  Column {
    anchors.fill: parent
    spacing: 0

    SettingsHeader {
      fg: root.fg
      title: "New space"
      action: root.busy ? "Creating…" : "Create"
      actionEnabled: root.valid
      onBack: root.closed()
      onActioned: root.create()
    }

    // Picture + name
    Item {
      width: parent.width
      height: Style.space(96)

      Item {
        id: avPick
        anchors.left: parent.left; anchors.leftMargin: Style.space(22)
        anchors.top: parent.top; anchors.topMargin: Style.space(26)
        width: Style.space(64); height: width

        Rectangle {
          anchors.fill: parent
          radius: Style.space(20)
          color: "transparent"
          border.width: Math.max(1, Style.space(1))
          border.color: Util.alpha(root.fg, 0.3)
          visible: root.avatarPath === ""
          IconLabel {
            anchors.centerIn: parent
            icon: Icons.camera; color: Util.alpha(root.fg, 0.7); size: Style.font.iconLarge
          }
        }
        Avatar {
          anchors.fill: parent
          visible: root.avatarPath !== ""
          cornerRadius: Style.space(20)
          size: avPick.width
          source: root.avatarPath
          name: nameField.text
        }
        MouseArea {
          anchors.fill: parent; cursorShape: Qt.PointingHandCursor
          onClicked: root.pickAvatar()
        }
      }

      Column {
        anchors.left: avPick.right; anchors.leftMargin: Style.space(16)
        anchors.right: parent.right; anchors.rightMargin: Style.space(22)
        anchors.top: parent.top; anchors.topMargin: Style.space(14)
        spacing: Style.space(6)
        Text {
          text: "Name"; color: root.fg
          font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
        }
        Rectangle {
          width: parent.width; height: Style.space(44)
          radius: Style.space(10)
          color: "transparent"
          border.width: Math.max(1, Style.space(1))
          border.color: nameField.activeFocus ? Util.alpha(Color.accent, 0.8) : Util.alpha(root.fg, 0.2)
          TextInput {
            id: nameField
            anchors.fill: parent
            anchors.leftMargin: Style.space(12); anchors.rightMargin: Style.space(12)
            verticalAlignment: TextInput.AlignVCenter
            color: root.fg; clip: true
            font.family: Fonts.ui; font.pixelSize: Style.font.body
            onAccepted: root.create()
            Text {
              anchors.fill: parent
              verticalAlignment: Text.AlignVCenter
              visible: nameField.text === ""
              text: "Add name…"; color: Util.alpha(root.fg, 0.4)
              font.family: Fonts.ui; font.pixelSize: Style.font.body
            }
          }
        }
      }
    }

    // Topic
    Column {
      width: parent.width
      spacing: Style.space(6)
      Item { width: parent.width; height: Style.space(10) }
      Text {
        x: Style.space(22)
        text: "Topic (optional)"; color: root.fg
        font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
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
          color: root.fg; clip: true
          wrapMode: TextEdit.Wrap
          font.family: Fonts.ui; font.pixelSize: Style.font.body
          Text {
            visible: topicField.text === ""
            text: "Add description…"; color: Util.alpha(root.fg, 0.4)
            font.family: Fonts.ui; font.pixelSize: Style.font.body
          }
        }
      }
      Item { width: parent.width; height: Style.space(16) }
    }

    // Access
    SettingsGroup {
      fg: root.fg
      title: "Who has access"
      SettingsRow {
        fg: root.fg
        icon: Icons.globe; filled: false
        label: "Public"; sublabel: "Anyone can join."
        trailing: "radio"; on: !root.isPrivate
        onClicked: root.isPrivate = false
      }
      SettingsRow {
        fg: root.fg
        icon: Icons.lock; filled: false
        label: "Private"; sublabel: "Only people invited can join."
        trailing: "radio"; on: root.isPrivate
        onClicked: root.isPrivate = true
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
