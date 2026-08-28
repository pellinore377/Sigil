import QtQuick
import QtQuick.Controls as QQC
import qs.Commons
import qs.Ui
import "../components"
import ".."

// Forward flow: search + chat list; pick a destination for the payload.
Item {
  id: root
  property var svc: null
  property color fg: Color.menu.text
  property var payload: null
  /// "forward" sends the payload straight away; "attach" hands the chosen room
  /// back so the caller can stage something in its composer instead.
  property string mode: "forward"
  property string title: root.mode === "attach" ? "Send to" : "Forward to"
  signal closed()
  signal forwarded(string roomId)
  signal picked(string roomId)

  function reset() { search.text = "" }
  function focusSearch() { search.forceActiveFocus() }

  readonly property var chats: {
    if (!svc) return []
    var q = search.text.trim().toLowerCase()
    var out = []
    for (var i = 0; i < svc.rooms.length; i++) {
      var r = svc.rooms[i]
      if (r.isSpace || r.isInvite) continue
      if (q !== "" && (r.name || "").toLowerCase().indexOf(q) < 0) continue
      out.push(r)
    }
    out.sort(function(a, b) { return (b.lastActivityTs || 0) - (a.lastActivityTs || 0) })
    return out
  }

  function doForward(rid) {
    if (root.mode === "attach") { root.picked(rid); return }
    var it = root.payload
    if (!it || !root.svc) return
    if (it.kind === "image" && it.media && (it.media.path || it.media.thumbnailPath)) root.svc.sendFiles(rid, [it.media.path || it.media.thumbnailPath])
    else root.svc.sendText(rid, it.body, {})
    root.forwarded(rid)
  }

  Column {
    anchors.fill: parent
    spacing: 0
    Item {
      width: parent.width; height: Style.space(54)
      PanelActionButton { id: xBtn; anchors.left: parent.left; anchors.leftMargin: Style.space(6); anchors.verticalCenter: parent.verticalCenter; fontFamily: Fonts.icon; iconText: Icons.close; foreground: root.fg; onClicked: root.closed() }
      Text { anchors.left: xBtn.right; anchors.leftMargin: Style.space(6); anchors.verticalCenter: parent.verticalCenter; text: root.mode === "attach" ? "Send contact" : "Forward"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.heading; font.bold: true }
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
          placeholderText: "Search chats"
          placeholderTextColor: Util.alpha(root.fg, 0.45)
          background: Item {}
          QQC.ContextMenu.menu: null
          TextContextMenu { editor: parent }
          Keys.onPressed: function(e) { if (e.key === Qt.Key_Escape) { root.closed(); e.accepted = true } }
        }
      }
    }
    ListView {
      id: people
      width: parent.width
      height: parent.height - y
      clip: true
      boundsBehavior: Flickable.StopAtBounds
      QQC.ScrollBar.vertical: ScrollBarStyle {}
      model: root.chats
      delegate: Item {
        required property var modelData
        width: people.width; height: Style.space(56)
        Rectangle { anchors.fill: parent; anchors.margins: Style.space(4); anchors.leftMargin: Style.space(10); anchors.rightMargin: Style.space(10); radius: Style.space(12); color: ph.containsMouse ? Util.alpha(root.fg, 0.05) : "transparent" }
        Avatar { id: pav; anchors.left: parent.left; anchors.leftMargin: Style.space(16); anchors.verticalCenter: parent.verticalCenter; size: Style.space(38); source: modelData.avatarPath || ""; name: modelData.name; userId: modelData.isDm ? (modelData.dmUserId || modelData.id) : modelData.id }
        Text { anchors.left: pav.right; anchors.leftMargin: Style.space(12); anchors.right: parent.right; anchors.rightMargin: Style.space(12); anchors.verticalCenter: parent.verticalCenter; elide: Text.ElideRight; text: modelData.name || modelData.id; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: true }
        MouseArea { id: ph; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: root.doForward(modelData.id) }
      }
    }
  }
}
