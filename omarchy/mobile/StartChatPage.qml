import QtQuick
import QtQuick.Controls as QQC
import qs.Commons
import qs.Ui
import "../components"
import ".."

// Start chat: search people, quick actions, suggestions.
Item {
  id: root
  property var svc: null
  property color fg: Color.menu.text
  signal closed()
  signal roomOpened(string id)
  property string mode: ""          // "" | "create" | "join"
  property var results: []
  property bool busy: false
  property string error: ""

  function reset() { root.mode = ""; root.results = []; root.busy = false; root.error = ""; search.text = ""; extra.text = "" }
  function focusSearch() { search.forceActiveFocus() }

  readonly property var suggestions: {
    if (!svc) return []
    var out = []
    for (var i = 0; i < svc.rooms.length; i++) {
      var r = svc.rooms[i]
      if (r.isDm && r.dmUserId) out.push({ userId: r.dmUserId, displayName: r.name, avatarPath: r.avatarPath || "" })
    }
    return out
  }

  Timer { id: searchTimer; interval: 300; onTriggered: if (root.svc && search.text.trim().length >= 2) root.svc.searchUsers(search.text.trim(), function(r, e) { root.results = r ? r.users : [] }) }

  function startDm(uid) {
    if (root.busy || !root.svc) return
    root.busy = true
    root.svc.createDm(uid, function(r, e) { root.busy = false; if (r && r.roomId) root.roomOpened(r.roomId); else root.error = e && e.message ? e.message : "Could not start chat" })
  }
  function submitExtra() {
    var v = extra.text.trim()
    if (v === "" || !root.svc || root.busy) return
    root.busy = true
    var done = function(r, e) { root.busy = false; if (r && r.roomId) root.roomOpened(r.roomId); else root.error = e && e.message ? e.message : "Failed" }
    if (root.mode === "create") root.svc.createRoom({ name: v, topic: "", private: true, encrypted: true }, done)
    // A space holds rooms, not messages: never encrypted, and opening it would
    // show an empty timeline, so it just closes.
    else if (root.mode === "space") root.svc.createSpace(v, function (r, e) {
      root.busy = false
      if (r && r.roomId) root.closed()
      else root.error = e && e.message ? e.message : "Failed"
    })
    else root.svc.joinRoom(v, done)
  }

  Column {
    anchors.fill: parent
    spacing: 0
    Item {
      width: parent.width; height: Style.space(54)
      PanelActionButton { id: xBtn; anchors.left: parent.left; anchors.leftMargin: Style.space(6); anchors.verticalCenter: parent.verticalCenter; fontFamily: Fonts.icon; iconText: Icons.close; foreground: root.fg; onClicked: root.closed() }
      Text { anchors.left: xBtn.right; anchors.leftMargin: Style.space(6); anchors.verticalCenter: parent.verticalCenter; text: "Start chat"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.heading; font.bold: true }
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
          onTextChanged: { root.error = ""; if (text.trim().length >= 2) searchTimer.restart(); else root.results = [] }
          Keys.onPressed: function(e) { if (e.key === Qt.Key_Escape) { root.closed(); e.accepted = true } }
        }
      }
    }
    Column {
      width: parent.width
      visible: search.text.trim() === ""
      Repeater {
        model: [ { t: "New room", icon: Icons.plus, m: "create" },
                 { t: "New space", icon: Icons.space, m: "space" },
                 { t: "Join room by address", icon: "#", m: "join" } ]
        delegate: Item {
          required property var modelData
          width: parent.width; height: Style.space(48)
          Rectangle { anchors.fill: parent; anchors.margins: Style.space(4); anchors.leftMargin: Style.space(10); anchors.rightMargin: Style.space(10); radius: Style.space(12); color: qah.containsMouse ? Util.alpha(root.fg, 0.05) : "transparent" }
          IconLabel { anchors.left: parent.left; anchors.leftMargin: Style.space(22); anchors.verticalCenter: parent.verticalCenter; icon: modelData.icon; color: Util.alpha(root.fg, 0.7); size: Style.font.icon }
          Text { anchors.left: parent.left; anchors.leftMargin: Style.space(56); anchors.verticalCenter: parent.verticalCenter; text: modelData.t; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body }
          MouseArea { id: qah; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: { root.mode = root.mode === modelData.m ? "" : modelData.m; if (root.mode !== "") Qt.callLater(extra.forceActiveFocus) } }
        }
      }
      Item {
        width: parent.width; height: root.mode !== "" ? Style.space(52) : 0
        visible: root.mode !== ""
        Rectangle {
          anchors.fill: parent; anchors.leftMargin: Style.space(14); anchors.rightMargin: Style.space(14); anchors.topMargin: Style.space(4); anchors.bottomMargin: Style.space(8)
          radius: Style.space(12); color: Util.alpha(root.fg, 0.06)
          border.width: 1; border.color: extra.activeFocus ? Util.alpha(Color.accent, 0.4) : Util.alpha(root.fg, 0.1)
          QQC.TextField {
            id: extra
            anchors.fill: parent; anchors.leftMargin: Style.space(10); anchors.rightMargin: Style.space(80)
            color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body
            placeholderText: root.mode === "create" ? "Room name"
                           : root.mode === "space" ? "Space name"
                           : "#room:server.tld"
            placeholderTextColor: Util.alpha(root.fg, 0.4)
            background: Item {}
            QQC.ContextMenu.menu: null
            TextContextMenu { editor: parent }
            Keys.onPressed: function(e) { if (e.key === Qt.Key_Return || e.key === Qt.Key_Enter) { root.submitExtra(); e.accepted = true } else if (e.key === Qt.Key_Escape) { root.mode = ""; e.accepted = true } }
          }
          Button { anchors.right: parent.right; anchors.rightMargin: Style.space(6); anchors.verticalCenter: parent.verticalCenter; text: root.mode === "create" ? "Create" : "Join"; foreground: root.fg; bordered: true; enabled: !root.busy; onClicked: root.submitExtra() }
        }
      }
    }
    Text { visible: root.error !== ""; width: parent.width; horizontalAlignment: Text.AlignHCenter; text: root.error; color: Color.urgent; font.family: Fonts.ui; font.pixelSize: Style.font.caption; topPadding: Style.space(4) }
    Text { text: search.text.trim().length >= 2 ? "Results" : "Suggestions"; color: Util.alpha(root.fg, 0.55); font.family: Fonts.ui; font.pixelSize: Style.font.caption; font.bold: true; leftPadding: Style.space(18); topPadding: Style.space(10); bottomPadding: Style.space(4) }
    ListView {
      id: people
      width: parent.width
      height: parent.height - y
      clip: true
      boundsBehavior: Flickable.StopAtBounds
      QQC.ScrollBar.vertical: ScrollBarStyle {}
      model: search.text.trim().length >= 2 ? root.results : root.suggestions
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
        MouseArea { id: ph; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: root.startDm(modelData.userId) }
      }
    }
  }
}
