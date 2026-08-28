import QtQuick
import QtQuick.Controls as QQC
import qs.Commons
import qs.Ui
import "../components"
import ".."

// Invite people to the current room (start-chat style user search).
Item {
  id: root
  property var svc: null
  property color fg: Color.menu.text
  property string roomId: ""
  signal closed()
  property var results: []
  property string note: ""

  function reset() { search.text = ""; root.results = []; root.note = "" }
  function focusSearch() { search.forceActiveFocus() }
  Timer { id: searchTimer; interval: 300; onTriggered: if (root.svc && search.text.trim().length >= 2) root.svc.searchUsers(search.text.trim(), function(r, e) { root.results = r ? r.users : [] }) }

  Column {
    anchors.fill: parent
    spacing: 0
    Item {
      width: parent.width; height: Style.space(54)
      PanelActionButton { id: backBtn; anchors.left: parent.left; anchors.leftMargin: Style.space(6); anchors.verticalCenter: parent.verticalCenter; fontFamily: Fonts.icon; iconText: Icons.back; foreground: root.fg; onClicked: root.closed() }
      Text { anchors.left: backBtn.right; anchors.leftMargin: Style.space(6); anchors.verticalCenter: parent.verticalCenter; text: "Add people"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.heading; font.bold: true }
    }
    Item {
      width: parent.width; height: Style.space(46)
      Rectangle {
        anchors.fill: parent; anchors.leftMargin: Style.space(14); anchors.rightMargin: Style.space(14); anchors.bottomMargin: Style.space(8)
        radius: height / 2; color: Util.alpha(root.fg, 0.07)
        border.width: 1; border.color: search.activeFocus ? Util.alpha(Color.accent, 0.4) : "transparent"
        IconLabel { anchors.left: parent.left; anchors.leftMargin: Style.space(14); anchors.verticalCenter: parent.verticalCenter; icon: Icons.search; color: Util.alpha(root.fg, 0.5); size: Style.font.icon }
        QQC.TextField {
          id: search
          anchors.fill: parent; anchors.leftMargin: Style.space(36); anchors.rightMargin: Style.space(12)
          color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body
          placeholderText: "Search for someone"
          placeholderTextColor: Util.alpha(root.fg, 0.45)
          background: Item {}
          QQC.ContextMenu.menu: null
          TextContextMenu { editor: parent }
          onTextChanged: { root.note = ""; if (text.trim().length >= 2) searchTimer.restart(); else root.results = [] }
          Keys.onPressed: function(e) { if (e.key === Qt.Key_Escape) { root.closed(); e.accepted = true } }
        }
      }
    }
    Text { visible: root.note !== ""; width: parent.width; horizontalAlignment: Text.AlignHCenter; text: root.note; color: Color.accent; font.family: Fonts.ui; font.pixelSize: Style.font.caption; topPadding: Style.space(4) }
    ListView {
      id: people
      width: parent.width
      height: parent.height - y
      clip: true
      boundsBehavior: Flickable.StopAtBounds
      QQC.ScrollBar.vertical: ScrollBarStyle {}
      model: root.results
      delegate: Item {
        required property var modelData
        width: people.width; height: Style.space(56)
        Rectangle { anchors.fill: parent; anchors.margins: Style.space(4); anchors.leftMargin: Style.space(10); anchors.rightMargin: Style.space(10); radius: Style.space(12); color: ph.containsMouse ? Util.alpha(root.fg, 0.05) : "transparent" }
        Avatar { id: pav; anchors.left: parent.left; anchors.leftMargin: Style.space(16); anchors.verticalCenter: parent.verticalCenter; size: Style.space(38); source: modelData.avatarPath || ""; name: modelData.displayName || modelData.userId; userId: modelData.userId
          status: root.svc ? root.svc.presenceOf(modelData.userId) : ""; statusBackdrop: Color.menu.background }
        Column {
          anchors.left: pav.right; anchors.leftMargin: Style.space(12); anchors.right: parent.right; anchors.rightMargin: Style.space(12); anchors.verticalCenter: parent.verticalCenter
          Text { width: parent.width; elide: Text.ElideRight; text: modelData.displayName || modelData.userId; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: true }
          Text { width: parent.width; elide: Text.ElideRight; text: modelData.userId; color: Util.alpha(root.fg, 0.5); font.family: Fonts.ui; font.pixelSize: Style.font.caption }
        }
        MouseArea { id: ph; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: { if (root.svc) root.svc.inviteUser(root.roomId, modelData.userId, function(r, e) { root.note = e ? ("Invite failed: " + (e.message || "")) : ("Invited " + (modelData.displayName || modelData.userId)) }) } }
      }
    }
  }
}
