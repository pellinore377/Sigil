import QtQuick
import QtQuick.Controls as QQC
import qs.Commons
import qs.Ui
import "../components"
import ".."

// Home: header + search + segmented tabs (Chats | Spaces) + room/space list.
Item {
  id: root
  property var svc: null
  property color fg: Color.menu.text
  property int tab: 0                 // 0 chats, 1 spaces
  property string spaceFilter: ""     // non-empty: showing one space's rooms
  property string spaceFilterName: ""
  property var drafts: ({})       // roomId -> unsent composer text
  // Hover tips go through the panel's in-card layer: the shared tooltip is a
  // QQC ToolTip, which Qt 6.9 renders in its own window outside the app.
  property var tipLayer: null
  function showTip(item, on, text) {
    if (!root.tipLayer || !item) return
    if (on) {
      var p = item.mapToItem(null, item.width / 2, item.height + Style.space(6))
      root.tipLayer.show(text, p.x, p.y)
    } else {
      root.tipLayer.hide()
    }
  }

  property bool accountOpen: false    // account menu is drawn in-page, not as a popup
  signal roomSelected(string id)
  signal newAction(string what)
  signal newChat()
  signal newSpace()
  signal spaceOpened(string spaceId)
  signal maximizeRequested()

  function focusSearch() { searchField.forceActiveFocus() }
  function reset() { root.spaceFilter = ""; searchField.text = "" }

  readonly property var chats: {
    if (!svc) return []
    var q = searchField.text.trim().toLowerCase()
    var out = []
    var children = null
    if (root.spaceFilter !== "") {
      for (var s = 0; s < svc.spaces.length; s++) if (svc.spaces[s].id === root.spaceFilter) { children = {}; (svc.spaces[s].children || []).forEach(function(c) { children[c] = true }); break }
    }
    for (var i = 0; i < svc.rooms.length; i++) {
      var r = svc.rooms[i]
      if (r.isSpace) continue
      if (children && !children[r.id]) continue
      if (q !== "" && (r.name || "").toLowerCase().indexOf(q) < 0) continue
      out.push(r)
    }
    out.sort(function(a, b) {
      // Pinned first. This re-sorts what the engine sends, so the pin rule must
      // exist on both sides.
      var ap = !!a.isFavourite, bp = !!b.isFavourite
      if (ap !== bp) return ap ? -1 : 1
      var ah = (a.highlights || 0) > 0, bh = (b.highlights || 0) > 0
      if (ah !== bh) return ah ? -1 : 1
      var au = Math.max(a.unread || 0, a.unreadMessages || 0) > 0, bu = Math.max(b.unread || 0, b.unreadMessages || 0) > 0
      if (au !== bu) return au ? -1 : 1
      return (b.lastActivityTs || 0) - (a.lastActivityTs || 0)
    })
    return out
  }
  readonly property var spaceRows: {
    if (!svc) return []
    var q = searchField.text.trim().toLowerCase()
    return svc.spaces.filter(function(s) { return (s.level || 0) === 0 && (q === "" || (s.name || "").toLowerCase().indexOf(q) >= 0) })
  }

  Column {
    anchors.fill: parent
    spacing: 0

    // Header
    Item {
      width: parent.width; height: Style.space(54)
      Text { anchors.left: parent.left; anchors.leftMargin: Style.space(18); anchors.verticalCenter: parent.verticalCenter; text: "Sigil"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.heading; font.bold: true }
      Row {
        anchors.right: parent.right; anchors.rightMargin: Style.space(10); anchors.verticalCenter: parent.verticalCenter; spacing: Style.space(2)
        PanelActionButton { anchors.verticalCenter: parent.verticalCenter; fontFamily: Fonts.iconFilled; iconText: Icons.maximize; foreground: root.fg; id: maxBtn; tooltipText: ""; onClicked: root.maximizeRequested()
            HoverHandler { onHoveredChanged: root.showTip(maxBtn, hovered, "Open as window") } }
        Item {
          anchors.verticalCenter: parent.verticalCenter
          width: Style.space(36); height: Style.space(36)
          Avatar { anchors.centerIn: parent; size: Style.space(28); source: root.svc ? root.svc.avatarPath : ""; name: root.svc ? root.svc.displayName : ""; userId: root.svc ? root.svc.userId : ""
            status: root.svc ? root.svc.presenceOf(root.svc.userId) : ""
            statusBackdrop: Color.menu.background }
          MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.accountOpen = !root.accountOpen }
        }
      }
    }

    Item {
      width: parent.width; height: Style.space(46)
      Rectangle {
        anchors.fill: parent; anchors.leftMargin: Style.space(14); anchors.rightMargin: Style.space(14); anchors.topMargin: Style.space(2); anchors.bottomMargin: Style.space(8)
        radius: height / 2
        color: Util.alpha(root.fg, 0.07)
        border.width: 1; border.color: searchField.activeFocus ? Util.alpha(Color.accent, 0.4) : "transparent"
        IconLabel { anchors.left: parent.left; anchors.leftMargin: Style.space(14); anchors.verticalCenter: parent.verticalCenter; icon: Icons.search; color: Util.alpha(root.fg, 0.5); filled: true; size: Style.font.icon }
        QQC.TextField {
          id: searchField
          anchors.fill: parent; anchors.leftMargin: Style.space(36); anchors.rightMargin: Style.space(12)
          color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body
          placeholderText: root.tab === 0 ? "Search chats" : "Search spaces"
          placeholderTextColor: Util.alpha(root.fg, 0.45)
          background: Item {}
          QQC.ContextMenu.menu: null
          TextContextMenu { editor: parent }
        }
      }
    }

    // Segmented tabs
    Item {
      width: parent.width; height: Style.space(44)
      Rectangle {
        id: seg
        anchors.centerIn: parent
        width: parent.width - Style.space(28); height: Style.space(34); radius: height / 2
        color: Util.alpha(root.fg, 0.06)
        Rectangle {
          width: parent.width / 2 - Style.space(3); height: parent.height - Style.space(6); radius: height / 2
          x: Style.space(3) + (root.tab === 0 ? 0 : parent.width / 2)
          y: Style.space(3)
          color: Util.alpha(Color.accent, 0.22)
          border.width: 1; border.color: Util.alpha(Color.accent, 0.35)
          Behavior on x { NumberAnimation { duration: 160; easing.type: Easing.OutCubic } }
        }
        Row {
          anchors.fill: parent
          Repeater {
            model: ["Chats", "Spaces"]
            delegate: Item {
              required property var modelData
              required property int index
              width: seg.width / 2; height: seg.height
              Text { anchors.centerIn: parent; text: modelData; color: root.tab === index ? root.fg : Util.alpha(root.fg, 0.55); font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: root.tab === index }
              MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: { root.tab = index; root.spaceFilter = "" } }
            }
          }
        }
      }
    }

    Item {
      width: parent.width; height: root.spaceFilter !== "" ? Style.space(36) : 0
      visible: root.spaceFilter !== ""
      Rectangle {
        anchors.left: parent.left; anchors.leftMargin: Style.space(14); anchors.verticalCenter: parent.verticalCenter
        width: chipRow.implicitWidth + Style.space(18); height: Style.space(26); radius: height / 2
        color: Util.alpha(Color.accent, 0.18); border.width: 1; border.color: Util.alpha(Color.accent, 0.3)
        Row { id: chipRow; anchors.centerIn: parent; spacing: Style.space(6)
          IconLabel { icon: Icons.home; color: root.fg; filled: true; size: Style.font.caption }
          Text { text: root.spaceFilterName; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall }
          IconLabel { icon: Icons.close; color: Util.alpha(root.fg, 0.6); filled: true; size: Style.font.caption } }
        MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: { root.spaceFilter = ""; root.tab = 1 } }
      }
    }

    // Lists
    ListView {
      id: list
      width: parent.width
      height: parent.height - y
      clip: true
      boundsBehavior: Flickable.StopAtBounds
      QQC.ScrollBar.vertical: ScrollBarStyle {}
      onMovementStarted: root.returning = false
      model: root.tab === 0 || root.spaceFilter !== "" ? root.chats : root.spaceRows
      delegate: root.tab === 0 || root.spaceFilter !== "" ? chatRow : spaceRow
      header: (root.tab === 1 && root.spaceFilter === "") ? spacesHero : null
    }
  }

  // New-chat FAB (bottom-right)
  Rectangle {
    id: fab
    anchors.right: parent.right; anchors.bottom: parent.bottom; anchors.margins: Style.space(14)
    width: Style.space(48); height: Style.space(48); radius: Style.space(16)
    color: Util.alpha(Color.accent, 0.9)
    IconLabel { anchors.centerIn: parent; icon: Icons.plus; color: Color.background; filled: true; size: Style.font.iconLarge }
    // On the Spaces tab the plus makes a space, not a chat.
    MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: (root.tab === 1 && root.spaceFilter === "") ? root.newSpace() : root.newChat() }
  }

  Component {
    id: chatRow
    Item {
      required property var modelData
      // Server notification counts are 0 for E2EE rooms; combine with the client count.
      readonly property int nUnread: Math.max(modelData.unread || 0, modelData.unreadMessages || 0)
      width: list.width; height: Style.space(64)
      Rectangle { anchors.fill: parent; anchors.margins: Style.space(4); anchors.leftMargin: Style.space(8); anchors.rightMargin: Style.space(8); radius: Style.space(14); color: h.containsMouse ? Util.alpha(root.fg, 0.05) : "transparent" }
      Avatar { id: av; anchors.left: parent.left; anchors.leftMargin: Style.space(16); anchors.verticalCenter: parent.verticalCenter; size: Style.space(44); source: modelData.avatarPath || ""; name: modelData.name; userId: modelData.isDm ? (modelData.dmUserId || modelData.id) : modelData.id
        // A room has no presence; only the person on the other end of a DM does.
        status: (modelData.isDm && root.svc) ? root.svc.presenceOf(modelData.dmUserId || "") : ""
        statusBackdrop: Color.menu.background }
      Column {
        anchors.left: av.right; anchors.leftMargin: Style.space(12); anchors.right: meta.left; anchors.rightMargin: Style.space(8); anchors.verticalCenter: parent.verticalCenter
        spacing: Style.space(2)
        Row { spacing: Style.space(5)
          Text { text: modelData.name || modelData.id; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.subtitle; font.bold: nUnread > 0 || (modelData.highlights || 0) > 0; elide: Text.ElideRight }
          IconLabel { visible: !!modelData.isEncrypted; icon: Icons.lock; color: Util.alpha(root.fg, 0.35); anchors.verticalCenter: parent.verticalCenter; filled: true; size: Style.font.caption }
          IconLabel { visible: !!modelData.hasActiveCall; icon: Icons.phone; color: Color.accent; anchors.verticalCenter: parent.verticalCenter; filled: true; size: Style.font.caption } }
        Row {
          width: parent.width
          spacing: Style.space(4)
          readonly property var typing: root.svc && root.svc.typingByRoom[modelData.id] ? root.svc.typingByRoom[modelData.id] : []
          readonly property string draft: (root.drafts[modelData.id] || "").trim()
          readonly property bool showDraft: typing.length === 0 && !modelData.isInvite && draft !== ""
          // An unsent message is marked with a red "Draft:" ahead of the text.
          Text {
            visible: parent.showDraft
            text: "Draft:"
            color: Color.urgent
            font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall; font.italic: true
          }
          // The leading mark is an icon and needs its own Text: one family per Text.
          Text {
            id: previewMark
            readonly property var lm: modelData.lastMessage
            visible: text !== ""
            text: {
              if (parent.showDraft || parent.typing.length > 0 || modelData.isInvite) return ""
              if (modelData.hasActiveCall) return Icons.phone
              if (!previewMark.lm) return ""
              if (previewMark.lm.hasCode) return Icons.codeBlocks
              switch (previewMark.lm.kind) {
              case "image": return Icons.camera
              case "video": return Icons.videoOn
              case "audio": case "voice": return Icons.micOn
              case "file": return Icons.attach
              case "call": return Icons.phone
              }
              return ""
            }
            color: modelData.hasActiveCall ? Color.accent : Util.alpha(root.fg, nUnread > 0 ? 0.8 : 0.55)
            font.family: Fonts.iconFilled; renderType: Text.NativeRendering; font.pixelSize: Style.font.bodySmall
            rightPadding: Style.space(4)
          }
          Text {
            width: parent.width - x
            elide: Text.ElideRight
            // One line, whatever the message was. `elide` alone trims only the
            // *last* line, so a pasted code block would push the rooms below off
            // screen. This is the guard the engine's newline collapsing cannot cover.
            maximumLineCount: 1
            wrapMode: Text.NoWrap
            readonly property var lm: modelData.lastMessage
            text: parent.showDraft ? parent.draft
                  : parent.typing.length > 0 ? parent.typing[0].displayName + " is typing…"
                  : modelData.isInvite ? "Invitation — tap to respond"
                  : modelData.hasActiveCall ? "Ongoing call"
                  : (lm ? ((modelData.isDm || !lm.senderName ? "" : lm.senderName + ": ")
                           + (lm.kind === "call" && !(lm.body || "") ? "Call" : (lm.body || ""))) : "")
            color: parent.typing.length > 0 ? Color.accent
                 : modelData.hasActiveCall ? Color.accent
                 : Util.alpha(root.fg, nUnread > 0 ? 0.8 : 0.55)
            font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
            font.italic: parent.showDraft
          }
        }
      }
      Column {
        id: meta
        anchors.right: parent.right; anchors.rightMargin: Style.space(16); anchors.verticalCenter: parent.verticalCenter
        spacing: Style.space(4)
        Text { anchors.right: parent.right; text: root.fmtTime(modelData.lastActivityTs); color: nUnread > 0 ? Color.accent : Util.alpha(root.fg, 0.45); font.family: Fonts.ui; font.pixelSize: Style.font.caption }
        // Pin and unread badge share one line so a pinned room grows no third row.
        Row {
          anchors.right: parent.right
          spacing: Style.space(5)
          layoutDirection: Qt.LeftToRight

          Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            visible: nUnread > 0 || (modelData.highlights || 0) > 0 || !!modelData.isInvite
            readonly property int n: (modelData.highlights || 0) > 0 ? modelData.highlights : nUnread
            width: visible ? Math.max(Style.space(19), bt.implicitWidth + Style.space(10)) : 0
            height: Style.space(19); radius: height / 2
            color: (modelData.highlights || 0) > 0 || modelData.isInvite ? Color.urgent : Util.alpha(Color.accent, 0.9)
            Text {
              id: bt
              anchors.centerIn: parent
              text: modelData.isInvite ? "!" : (parent.n > 99 ? "99+" : String(parent.n))
              color: Color.background
              font.family: Fonts.ui; font.pixelSize: Style.font.caption; font.bold: true
            }
          }

          Text {
            anchors.verticalCenter: parent.verticalCenter
            visible: !!modelData.isFavourite
            text: Icons.pin
            color: Util.alpha(root.fg, 0.55)
            font.family: Fonts.iconFilled; renderType: Text.NativeRendering
            font.pixelSize: Style.font.caption
          }
        }
      }
      MouseArea { id: h; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: root.roomSelected(modelData.id) }
    }
  }

  Component {
    id: spacesHero
    Column {
      width: list.width
      spacing: Style.space(6)
      bottomPadding: Style.space(16)
      Item { width: 1; height: Style.space(20) }
      Rectangle {
        anchors.horizontalCenter: parent.horizontalCenter
        width: Style.space(64); height: width; radius: Style.space(18)
        color: Util.alpha(root.fg, 0.07)
        IconLabel { anchors.centerIn: parent; icon: Icons.space; color: Util.alpha(root.fg, 0.55); filled: true; size: Style.space(30) }
      }
      Text {
        width: parent.width; horizontalAlignment: Text.AlignHCenter
        text: "Spaces"; color: root.fg
        font.family: Fonts.ui; font.pixelSize: Style.font.display; font.bold: true
      }
      Text {
        width: parent.width; horizontalAlignment: Text.AlignHCenter
        text: {
          var n = root.spaceRows.length
          return n === 1 ? "1 Space" : n + " Spaces"
        }
        color: Util.alpha(root.fg, 0.55)
        font.family: Fonts.ui; font.pixelSize: Style.font.body
      }
      Text {
        width: parent.width; horizontalAlignment: Text.AlignHCenter
        text: "Spaces you have created or joined."
        color: Util.alpha(root.fg, 0.6)
        font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
      }
      Item { width: 1; height: Style.space(8) }
      Rectangle { width: parent.width; height: Math.max(1, Style.space(1)); color: Util.alpha(root.fg, 0.08) }
    }
  }

  Component {
    id: spaceRow
    Item {
      required property var modelData
      width: list.width; height: Style.space(64)
      Rectangle { anchors.fill: parent; anchors.margins: Style.space(4); anchors.leftMargin: Style.space(8); anchors.rightMargin: Style.space(8); radius: Style.space(14); color: sh.containsMouse ? Util.alpha(root.fg, 0.05) : "transparent" }
      // Rounded square, not a circle: how Matrix distinguishes a space from a person.
      Avatar { id: sav; anchors.left: parent.left; anchors.leftMargin: Style.space(16); anchors.verticalCenter: parent.verticalCenter; size: Style.space(44); cornerRadius: Style.space(12); source: modelData.avatarPath || ""; name: modelData.name; userId: modelData.id }
      Column {
        anchors.left: sav.right; anchors.leftMargin: Style.space(12); anchors.right: chev.left; anchors.verticalCenter: parent.verticalCenter
        spacing: Style.space(2)
        Text { text: modelData.name || modelData.id; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.subtitle; elide: Text.ElideRight; width: parent.width }
        Row {
          spacing: Style.space(5)
          IconLabel { anchors.verticalCenter: parent.verticalCenter; icon: Icons.lock; color: Util.alpha(root.fg, 0.4); filled: true; size: Style.font.caption }
          Text { anchors.verticalCenter: parent.verticalCenter; text: "Private"; color: Util.alpha(root.fg, 0.5); font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall }
        }
        Text {
          text: {
            var rooms = modelData.children ? modelData.children.length : 0
            var rec = root.svc ? root.svc.room(modelData.id) : null
            var people = rec ? (rec.joinedMembers || 0) : 0
            var a = rooms === 1 ? "1 room" : rooms + " rooms"
            if (people <= 0) return a
            return a + " · " + (people === 1 ? "1 member" : people + " members")
          }
          color: Util.alpha(root.fg, 0.5); font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
        }
      }
      IconLabel { id: chev; anchors.right: parent.right; anchors.rightMargin: Style.space(18); anchors.verticalCenter: parent.verticalCenter; icon: Icons.chevronRight; color: Util.alpha(root.fg, 0.4); filled: true; size: Style.font.icon }
      MouseArea { id: sh; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: root.spaceOpened(modelData.id) }
    }
  }

  function fmtTime(ts) {
    if (!ts) return ""
    var d = new Date(ts), now = new Date()
    var start = new Date(now.getFullYear(), now.getMonth(), now.getDate())
    var diff = Math.floor((start - new Date(d.getFullYear(), d.getMonth(), d.getDate())) / 86400000)
    if (diff === 0) return Qt.formatTime(d, "HH:mm")
    if (diff === 1) return "Yesterday"
    if (diff < 7) return Qt.formatDate(d, "ddd")
    return Qt.formatDate(d, "d MMM")
  }

  // Account menu, drawn inside the panel card: as an xdg-popup it rendered
  // outside the window.
  MouseArea {
    anchors.fill: parent
    visible: root.accountOpen
    z: 60
    onClicked: root.accountOpen = false
  }

  Rectangle {
    id: accountMenu
    z: 61
    visible: opacity > 0.01
    opacity: root.accountOpen ? 1 : 0
    scale: root.accountOpen ? 1 : 0.94
    transformOrigin: Item.TopRight
    anchors.right: parent.right
    anchors.rightMargin: Style.space(12)
    anchors.top: parent.top
    anchors.topMargin: Style.space(48)
    width: Style.space(230)
    height: acCol.implicitHeight + Style.space(20)
    radius: Style.space(14)
    antialiasing: true
    color: Util.alpha(Color.popups.background, 0.98)
    Behavior on opacity { NumberAnimation { duration: 120 } }
    Behavior on scale { NumberAnimation { duration: 140; easing.type: Easing.OutCubic } }

    MouseArea { anchors.fill: parent }

    Column {
      id: acCol
      x: Style.space(10); y: Style.space(10)
      width: parent.width - Style.space(20)
      spacing: Style.space(4)
      Text { width: parent.width; elide: Text.ElideRight; text: root.svc ? root.svc.displayName : ""; color: Color.popups.text; font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: true }
      Text { width: parent.width; elide: Text.ElideRight; text: root.svc ? root.svc.userId : ""; color: Util.alpha(Color.popups.text, 0.6); font.family: Fonts.ui; font.pixelSize: Style.font.caption }
      Item { width: 1; height: Style.space(4) }
      Rectangle { width: parent.width; height: 1; color: Util.alpha(Color.popups.text, 0.1) }
      Item { width: 1; height: Style.space(4) }
      Rectangle {
        width: parent.width; height: Style.space(30); radius: Style.space(8)
        color: lo.containsMouse ? Util.alpha(Color.popups.text, 0.1) : "transparent"
        IconLabel { anchors.verticalCenter: parent.verticalCenter; anchors.left: parent.left; anchors.leftMargin: Style.space(8); icon: Icons.logout; color: Color.urgent; opacity: 0.85; filled: true; size: Style.font.icon }
        Text { anchors.verticalCenter: parent.verticalCenter; anchors.left: parent.left; anchors.leftMargin: Style.space(34); text: "Sign out"; color: Color.urgent; font.family: Fonts.ui; font.pixelSize: Style.font.body }
        MouseArea { id: lo; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: { root.accountOpen = false; if (root.svc) root.svc.logout() } }
      }
    }
  }

  property bool returning: false
  FrameAnimation {
    running: root.returning
    onTriggered: {
      var d = list.originY - list.contentY
      if (Math.abs(d) < 0.5) { list.contentY = list.originY; root.returning = false; list.returnToBounds(); return }
      var k = 1 - Math.pow(0.0001, Math.min(0.05, frameTime))
      list.contentY += d * k
    }
  }

  Rectangle {
    id: topBtn
    readonly property bool shown: list.count > 0 && list.contentHeight > list.height
      && (list.contentY - list.originY) > list.height * 0.75
    visible: scale > 0.01
    anchors.horizontalCenter: parent.horizontalCenter
    anchors.bottom: parent.bottom
    anchors.bottomMargin: Style.space(18)
    width: Style.space(38); height: Style.space(38); radius: height / 2
    antialiasing: true
    color: Color.popups.background
    scale: shown ? 1 : 0
    opacity: shown ? 1 : 0
    Behavior on scale { NumberAnimation { duration: 220; easing.type: Easing.OutBack; easing.overshoot: 2.2 } }
    Behavior on opacity { NumberAnimation { duration: 140 } }
    IconLabel { anchors.centerIn: parent; icon: Icons.arrowUp; color: root.fg; filled: true; size: Style.font.icon }
    MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.returning = true }
  }
}
