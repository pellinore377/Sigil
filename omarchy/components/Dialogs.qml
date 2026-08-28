import QtQuick
import QtQuick.Controls as QQC
import qs.Commons
import qs.Ui
import ".."
import "."

// Modal host: "dm" | "join" | "create" | "invite" | "leave"
Rectangle {
  id: root
  property var svc: null
  property color fg: Color.menu.text
  property real scrimRadius: 0
  property string mode: ""
  property string targetRoomId: ""
  property string targetRoomName: ""
  signal roomOpened(string id)
  visible: mode !== ""
  color: Util.alpha(Color.background, 0.55)
  radius: scrimRadius
  antialiasing: true
  property string error: ""
  property bool busy: false
  property var results: []

  function open(m, roomId, roomName) { root.mode = m; root.targetRoomId = roomId || ""; root.targetRoomName = roomName || ""; root.error = ""; root.results = []; root.busy = false; f1.text = ""; f2.text = ""; Qt.callLater(function() { f1.forceActiveFocus() }) }
  function close() { root.mode = "" }

  MouseArea { anchors.fill: parent; onClicked: root.close() }
  BorderSurface {
    anchors.centerIn: parent
    width: Style.space(440); height: box.implicitHeight + Style.space(40)
    radius: Style.cornerRadius; color: Color.popups.background
    borderSpec: Border.surfaceSpec("popups", "border", Color.popups.border, 1)
    MouseArea { anchors.fill: parent; onClicked: {} }
    Column {
      id: box
      anchors.fill: parent; anchors.margins: Style.space(20)
      spacing: Style.space(10)
      Text {
        text: root.mode === "dm" ? "New direct message" : root.mode === "join" ? "Join a room" : root.mode === "create" ? "Create a room" : root.mode === "invite" ? "Invite to " + root.targetRoomName : (root.mode === "leave" ? "Leave " + root.targetRoomName + "?" : "")
        color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.title; font.bold: true
      }
      Text { visible: root.mode === "leave"; width: parent.width; wrapMode: Text.Wrap; text: "You will stop receiving messages from this room."; color: Util.alpha(root.fg, 0.7); font.family: Fonts.ui; font.pixelSize: Style.font.body }
      Rectangle {
        visible: root.mode !== "leave"
        width: parent.width; height: Style.space(34); radius: Style.cornerRadius / 2; color: Util.alpha(root.fg, 0.06); border.width: 1; border.color: f1.activeFocus ? Util.alpha(Color.accent, 0.5) : Util.alpha(root.fg, 0.1)
        QQC.TextField {
          id: f1; anchors.fill: parent; anchors.leftMargin: Style.space(10); anchors.rightMargin: Style.space(10)
          color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body; background: Item {}
          placeholderText: root.mode === "dm" || root.mode === "invite" ? "Search people or @user:server" : root.mode === "join" ? "#room:server or !roomid:server" : "Room name"
          placeholderTextColor: Util.alpha(root.fg, 0.4)
          QQC.ContextMenu.menu: null
          TextContextMenu { editor: parent }
          onTextChanged: if (root.mode === "dm" || root.mode === "invite") searchTimer.restart()
          onAccepted: root.confirm()
        }
      }
      Rectangle {
        visible: root.mode === "create"
        width: parent.width; height: Style.space(34); radius: Style.cornerRadius / 2; color: Util.alpha(root.fg, 0.06); border.width: 1; border.color: f2.activeFocus ? Util.alpha(Color.accent, 0.5) : Util.alpha(root.fg, 0.1)
        QQC.TextField {
          id: f2; anchors.fill: parent; anchors.leftMargin: Style.space(10); anchors.rightMargin: Style.space(10)
          color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body
          placeholderText: "Topic (optional)"; placeholderTextColor: Util.alpha(root.fg, 0.4)
          background: Item {}
          QQC.ContextMenu.menu: null
          TextContextMenu { editor: parent }
          onAccepted: root.confirm()
        }
      }
      Row {
        visible: root.mode === "create"; spacing: Style.space(16)
        Row { spacing: Style.space(6); ToggleSwitch { id: privateSw; checked: true } Text { anchors.verticalCenter: parent.verticalCenter; text: "Private"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body } }
        Row { spacing: Style.space(6); ToggleSwitch { id: encSw; checked: true } Text { anchors.verticalCenter: parent.verticalCenter; text: "Encrypted"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body } }
      }
      Column {
        visible: (root.mode === "dm" || root.mode === "invite") && root.results.length > 0
        width: parent.width
        Repeater {
          model: root.results
          delegate: Rectangle {
            required property var modelData
            width: parent.width; height: Style.space(36); radius: Style.cornerRadius / 2
            color: rh.containsMouse ? Util.alpha(root.fg, 0.08) : "transparent"
            Avatar { id: ra; anchors.left: parent.left; anchors.leftMargin: Style.space(6); anchors.verticalCenter: parent.verticalCenter; size: Style.space(24); name: modelData.displayName; userId: modelData.userId }
            Column { anchors.left: ra.right; anchors.leftMargin: Style.space(8); anchors.verticalCenter: parent.verticalCenter
              Text { text: modelData.displayName; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body }
              Text { text: modelData.userId; color: Util.alpha(root.fg, 0.5); font.family: Fonts.ui; font.pixelSize: Style.font.caption } }
            MouseArea { id: rh; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: root.pick(modelData.userId) }
          }
        }
      }
      Text { visible: root.error !== ""; width: parent.width; wrapMode: Text.Wrap; text: root.error; color: Color.urgent; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall }
      Row {
        anchors.right: parent.right; spacing: Style.space(8)
        Button { text: "Cancel"; foreground: root.fg; onClicked: root.close() }
        Button { text: root.busy ? "…" : (root.mode === "leave" ? "Leave" : root.mode === "join" ? "Join" : root.mode === "create" ? "Create" : root.mode === "invite" ? "Invite" : "Start chat"); foreground: root.mode === "leave" ? Color.urgent : root.fg; bordered: true; enabled: !root.busy; onClicked: root.confirm() }
      }
    }
  }

  Timer { id: searchTimer; interval: 300; repeat: false; onTriggered: if (root.svc && f1.text.trim().length >= 2) root.svc.searchUsers(f1.text.trim(), function(r, e) { root.results = r ? r.users : [] }) }

  function pick(userId) { f1.text = userId; root.results = []; root.confirm() }

  function confirm() {
    if (!root.svc || root.busy) return
    root.error = ""
    var done = function(r, e) { root.busy = false; if (e) { root.error = e.message; return } root.close(); if (r && r.roomId) root.roomOpened(r.roomId) }
    var v = f1.text.trim()
    if (root.mode === "dm") { if (v === "") return; root.busy = true; root.svc.createDm(v, done) }
    else if (root.mode === "join") { if (v === "") return; root.busy = true; root.svc.joinRoom(v, done) }
    else if (root.mode === "create") { if (v === "") return; root.busy = true; root.svc.createRoom({ name: v, topic: f2.text.trim(), private: privateSw.checked, encrypted: encSw.checked }, done) }
    else if (root.mode === "invite") { if (v === "") return; root.busy = true; root.svc.inviteUser(root.targetRoomId, v, function(r, e) { root.busy = false; if (e) root.error = e.message; else root.close() }) }
    else if (root.mode === "leave") { root.busy = true; var rid = root.targetRoomId; root.svc.leaveRoom(rid, function(r, e) { root.busy = false; if (e) root.error = e.message; else root.close() }) }
  }
}
