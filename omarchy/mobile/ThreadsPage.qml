import QtQuick
import QtQuick.Controls as QQC
import qs.Commons
import qs.Ui
import "../components"

// Every thread in a room as a list, most recently active first (server order).
// Opening one is not a special view: the engine builds a thread-focused
// timeline and the ordinary chat page renders it.
Item {
  id: root
  property var svc: null
  property string roomId: ""
  property color fg: Color.menu.text
  /// The room's own tint, so Threads does not open on a differently-coloured page.
  property var chatTheme: ({})
  signal closed()
  signal threadPicked(string rootId)

  // Tones are mixed from the single chat accent, exactly as ChatPage does.
  readonly property bool themed: (root.chatTheme.accent || "") !== ""
  readonly property color accC: root.themed ? Qt.color(root.chatTheme.accent) : Color.accent
  function mixc(a, b, t) { return Qt.rgba(a.r * (1 - t) + b.r * t, a.g * (1 - t) + b.g * t, a.b * (1 - t) + b.b * t, 1) }
  readonly property real tintAmt: 0.35
  readonly property color surfaceC: root.themed ? root.mixc(Qt.lighter(Color.menu.background, 1.35), root.accC, root.tintAmt) : Color.popups.background
  readonly property color chromeC: root.themed ? root.surfaceC : Qt.lighter(Color.menu.background, 1.35)
  readonly property color convoC: {
    var d = Qt.darker(Color.menu.background, 1.35)
    if (!root.themed) return d
    var a = root.accC
    return Qt.rgba(d.r * 0.82 + a.r * 0.18, d.g * 0.82 + a.g * 0.18, d.b * 0.82 + a.b * 0.18, 1)
  }

  readonly property var room: root.svc ? root.svc.room(root.roomId) : null
  property var threads: []
  property bool loading: false
  property bool loaded: false

  function reset() {
    root.threads = []
    root.loaded = false
    root.load()
  }
  function load() {
    if (!root.svc || !root.roomId) return
    root.loading = true
    root.svc.listThreads(root.roomId, function (r, e) {
      root.loading = false
      root.loaded = true
      root.threads = (r && r.threads) ? r.threads : []
    })
  }

  // Same clock format as the room list this page is shaped after.
  // Live: the list re-reads when anything happens in a thread of this room.
  Timer { id: reloadTimer; interval: 700; onTriggered: root.load() }
  Connections {
    target: root.svc
    ignoreUnknownSignals: true
    function onThreadsChanged(roomId) { if (roomId === root.roomId) reloadTimer.restart() }
  }

  function when(ts) {
    if (!ts) return ""
    var d = new Date(ts), now = new Date()
    var start = new Date(now.getFullYear(), now.getMonth(), now.getDate())
    var diff = Math.floor((start - new Date(d.getFullYear(), d.getMonth(), d.getDate())) / 86400000)
    if (diff === 0) return Qt.formatTime(d, "HH:mm")
    if (diff === 1) return "Yesterday"
    if (diff < 7) return Qt.formatDate(d, "ddd")
    return Qt.formatDate(d, "d MMM")
  }

  Rectangle { anchors.fill: parent; color: root.chromeC }

  Column {
    anchors.fill: parent
    spacing: 0

    Item {
      width: parent.width; height: Style.space(56)
      PanelActionButton {
        id: backBtn
        fontFamily: Fonts.icon
        anchors.left: parent.left; anchors.leftMargin: Style.space(6)
        anchors.verticalCenter: parent.verticalCenter
        iconText: Icons.back; foreground: root.fg; onClicked: root.closed()
      }
      Column {
        anchors.left: backBtn.right; anchors.leftMargin: Style.space(6)
        anchors.right: parent.right; anchors.rightMargin: Style.space(14)
        anchors.verticalCenter: parent.verticalCenter
        Text {
          width: parent.width; elide: Text.ElideRight
          text: "Threads"
          color: root.fg
          font.family: Fonts.ui; font.pixelSize: Style.font.heading; font.bold: true
        }
        Text {
          // Unconditional, like the Pins page: a subtitle that appears only once
          // the count lands makes the title jump on load.
          width: parent.width; elide: Text.ElideRight
          text: {
            var n = root.room ? (root.room.name || root.room.id) : ""
            if (root.threads.length === 0) return n
            return n + " · " + root.threads.length + (root.threads.length === 1 ? " thread" : " threads")
          }
          color: Util.alpha(root.fg, 0.55)
          font.family: Fonts.ui; font.pixelSize: Style.font.caption
        }
      }
    }

    Item {
      width: parent.width
      height: parent.height - y

      Rectangle {
        anchors.fill: parent
        topLeftRadius: Style.space(24); topRightRadius: Style.space(24)
        antialiasing: true
        color: root.convoC
      }

      // Empty and loading say different things.
      Column {
        anchors.centerIn: parent
        width: parent.width - Style.space(60)
        spacing: Style.space(10)
        visible: root.threads.length === 0
        IconLabel { anchors.horizontalCenter: parent.horizontalCenter
          icon: Icons.thread
          color: Util.alpha(root.accC, 0.8); size: Style.space(44) }
        Text {
          width: parent.width
          horizontalAlignment: Text.AlignHCenter
          text: root.loading || !root.loaded ? "Looking for threads…" : "No threads yet"
          color: root.fg
          font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: true
        }
        Text {
          width: parent.width
          horizontalAlignment: Text.AlignHCenter
          wrapMode: Text.Wrap
          visible: root.loaded && !root.loading
          text: "Reply in thread from a message's menu to start one."
          color: Util.alpha(root.fg, 0.6)
          font.family: Fonts.ui; font.pixelSize: Style.font.caption
        }
      }

      ListView {
        id: list
        anchors.fill: parent
        anchors.topMargin: Style.space(6)
        visible: root.threads.length > 0
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        QQC.ScrollBar.vertical: ScrollBarStyle {}
        model: root.threads

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
            source: row.modelData.avatarPath || ""
            name: row.modelData.senderName || ""
            userId: row.modelData.sender || ""
          }

          Column {
            anchors.left: av.right; anchors.leftMargin: Style.space(12)
            anchors.right: stamp.left; anchors.rightMargin: Style.space(10)
            anchors.verticalCenter: parent.verticalCenter
            spacing: Style.space(2)
            Text {
              width: parent.width; elide: Text.ElideRight
              text: row.modelData.senderName || row.modelData.sender || ""
              color: root.fg
              font.family: Fonts.ui; font.pixelSize: Style.font.subtitle; font.bold: true
            }
            Row {
              width: parent.width
              spacing: Style.space(5)
              Text {
                anchors.verticalCenter: parent.verticalCenter
                visible: (row.modelData.count || 0) > 0
                text: {
                  var n = row.modelData.count || 0
                  return n === 1 ? "1 reply" : n + " replies"
                }
                color: Util.alpha(root.accC, 0.9)
                font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall; font.bold: true
              }
              Text {
                anchors.verticalCenter: parent.verticalCenter
                width: parent.width - x
                elide: Text.ElideRight
                maximumLineCount: 1
                text: row.modelData.body || "Thread"
                color: Util.alpha(root.fg, 0.55)
                font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
              }
            }
          }

          Text {
            id: stamp
            anchors.right: parent.right; anchors.rightMargin: Style.space(16)
            anchors.verticalCenter: parent.verticalCenter
            text: root.when(row.modelData.ts)
            color: Util.alpha(root.fg, 0.45)
            font.family: Fonts.ui; font.pixelSize: Style.font.caption
          }

          MouseArea {
            id: rh
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: root.threadPicked(row.modelData.rootId)
          }
        }
      }
    }
  }
}
