import QtQuick
import QtQuick.Controls as QQC
import qs.Commons
import qs.Ui
import "../components"

// A room's pinned messages as a timeline. Pinned events are fetched by id
// (`pins.items`); `TimelineFocus::PinnedEvents` is not usable here.
Item {
  id: root
  property var svc: null
  property string roomId: ""
  property color fg: Color.menu.text
  property var chatTheme: ({})
  signal closed()
  /// Go to this message in the conversation.
  signal jumpRequested(string eventId)

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

  /// The exact fill BubbleDelegate uses; an approximation does not match the room.
  function mix(src, a) {
    var bg = Color.popups.background
    return Qt.rgba(src.r * a + bg.r * (1 - a), src.g * a + bg.g * (1 - a), src.b * a + bg.b * (1 - a), 1)
  }
  function bubbleFill(own) {
    return own ? root.mix(root.accC, 0.42)
               : (root.themed ? root.surfaceC : root.mix(root.fg, 0.22))
  }

  readonly property var room: root.svc ? root.svc.room(root.roomId) : null
  property var items: []
  property bool loading: false
  property bool loaded: false

  function reset() { root.items = []; root.loaded = false; root.load() }
  function load() {
    if (!root.svc || !root.roomId) return
    root.loading = true
    root.svc.pinnedItems(root.roomId, function (r, e) {
      root.loading = false
      root.loaded = true
      root.items = (r && r.items) ? r.items : []
    })
  }
  // Unpinning from here removes the row, so the list has to follow the set.
  Connections {
    target: root.svc
    function onPinnedByRoomChanged() { if (root.loaded) root.load() }
  }

  function stamp(ts) {
    if (!ts) return ""
    var d = new Date(ts), now = new Date()
    if (d.toDateString() === now.toDateString()) return "Today · " + Qt.formatTime(d, "h:mm AP")
    var y = new Date(now.getTime() - 86400000)
    if (d.toDateString() === y.toDateString()) return "Yesterday · " + Qt.formatTime(d, "h:mm AP")
    return Qt.formatDate(d, "d MMM") + " · " + Qt.formatTime(d, "h:mm AP")
  }
  // Icon and words are separate Text elements: the icon font carries no letters
  // and the text font no icons, and QML's `font` group has no `families` list.
  function labelIcon(kind) {
    switch (kind) {
    case "image": return Icons.image
    case "video": return Icons.videoOn
    case "audio": return Icons.audioNote
    case "file": return Icons.file
    case "location": return Icons.location
    case "sticker": return Icons.react
    default: return ""
    }
  }
  function labelWords(kind) {
    switch (kind) {
    case "image": return "Photo"
    case "video": return "Video"
    case "audio": return "Audio"
    case "file": return "File"
    case "location": return "Location"
    case "sticker": return "Sticker"
    default: return ""
    }
  }

  Rectangle { anchors.fill: parent; color: root.chromeC }

  Column {
    anchors.fill: parent
    spacing: 0

    // Header
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
          text: "Pinned"
          color: root.fg
          font.family: Fonts.ui; font.pixelSize: Style.font.heading; font.bold: true
        }
        Text {
          width: parent.width; elide: Text.ElideRight
          text: {
            var n = root.room ? (root.room.name || root.room.id) : ""
            return root.items.length > 0 ? n + " · " + root.items.length + " pinned" : n
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

      // Empty
      Column {
        anchors.centerIn: parent
        width: parent.width - Style.space(60)
        spacing: Style.space(10)
        visible: root.items.length === 0
        IconLabel { renderMode: Text.QtRendering; anchors.horizontalCenter: parent.horizontalCenter
          icon: Icons.pin
          color: Util.alpha(root.accC, 0.8); size: Style.space(44)
          rotation: 35 }
        Text {
          width: parent.width
          horizontalAlignment: Text.AlignHCenter
          text: root.loading || !root.loaded ? "Loading pinned messages…" : "Nothing pinned yet"
          color: root.fg
          font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: true
        }
        Text {
          width: parent.width
          horizontalAlignment: Text.AlignHCenter
          wrapMode: Text.Wrap
          visible: root.loaded && !root.loading
          text: "Pin a message from its menu to keep it here."
          color: Util.alpha(root.fg, 0.6)
          font.family: Fonts.ui; font.pixelSize: Style.font.caption
        }
      }

      // List
      ListView {
        id: list
        anchors.fill: parent
        anchors.topMargin: Style.space(10)
        clip: true
        visible: root.items.length > 0
        boundsBehavior: Flickable.StopAtBounds
        QQC.ScrollBar.vertical: ScrollBarStyle {}
        model: root.items
        spacing: Style.space(4)

        delegate: Item {
          id: row
          required property var modelData
          readonly property bool own: !!row.modelData.isOwn
          width: list.width
          height: card.height + stampText.height + senderRow.height + Style.space(row.own ? 14 : 20)

          Text {
            id: stampText
            anchors.top: parent.top
            anchors.horizontalCenter: parent.horizontalCenter
            text: root.stamp(row.modelData.ts)
            color: Util.alpha(root.fg, 0.55)
            font.family: Fonts.ui; font.pixelSize: Style.font.caption; font.bold: true
          }

          Row {
            id: senderRow
            // A DM shows no sender header in the room, so none here either.
            visible: !row.own && !(root.room && root.room.isDm)
            height: visible ? Style.space(24) : 0
            anchors.top: stampText.bottom; anchors.topMargin: Style.space(4)
            anchors.left: parent.left; anchors.leftMargin: Style.space(14)
            spacing: Style.space(7)
            Avatar {
              anchors.verticalCenter: parent.verticalCenter
              size: Style.space(20)
              source: row.modelData.avatarPath || ""
              name: row.modelData.senderName || ""
              userId: row.modelData.sender || ""
            }
            Text {
              anchors.verticalCenter: parent.verticalCenter
              text: row.modelData.senderName || row.modelData.sender || ""
              color: Util.alpha(root.fg, 0.6)
              font.family: Fonts.ui; font.pixelSize: Style.font.caption; font.bold: true
            }
          }

          Rectangle {
            id: card
            // Room for the pin's overhang; the room reserves the same with `pinLift`.
            anchors.top: senderRow.bottom
            anchors.topMargin: Style.space(row.own ? 2 : 8)
            anchors.left: row.own ? undefined : parent.left
            anchors.right: row.own ? parent.right : undefined
            anchors.leftMargin: Style.space(14)
            anchors.rightMargin: Style.space(14)
            // Widest natural line + padding. NOT `inner.implicitWidth`: the Column
            // is anchored to this card, so card and Column widths would form a
            // loop. `Text.implicitWidth` is unwrapped and safe to measure.
            width: Math.min(list.width * 0.78,
                            Math.max(Style.space(40),
                                     Math.max(body.implicitWidth, kindLabel.implicitWidth) + Style.space(22)))
            height: inner.implicitHeight + Style.space(20)
            radius: Style.space(16)
            antialiasing: true
            color: root.bubbleFill(row.own)
            Item {
              width: Style.space(20); height: Style.space(20)
              z: 2
              anchors.top: parent.top; anchors.topMargin: -Style.space(5)
              anchors.left: row.own ? undefined : parent.left
              anchors.right: row.own ? parent.right : undefined
              anchors.leftMargin: -Style.space(5); anchors.rightMargin: -Style.space(5)
              Rectangle {
                anchors.fill: parent; radius: width / 2; antialiasing: true
                color: root.accC
                border.width: Math.max(1, Style.space(1.5))
                border.color: Color.popups.background
              }
              Text {
                anchors.centerIn: parent
                text: Icons.pin
                // Tracks a light theme; a hardcoded near-black does not.
                color: Color.background
                font.family: Fonts.icon; renderType: Text.NativeRendering; font.pixelSize: Style.font.caption
                rotation: row.own ? 35 : -35
              }
            }

            Column {
              id: inner
              anchors.left: parent.left; anchors.right: parent.right
              anchors.top: parent.top
              anchors.margins: Style.space(10)
              spacing: Style.space(3)

              Row {
                id: kindLabel
                width: parent.width
                spacing: Style.space(4)
                visible: root.labelWords(row.modelData.kind) !== ""
                IconLabel { anchors.verticalCenter: parent.verticalCenter
                  icon: root.labelIcon(row.modelData.kind)
                  color: Util.alpha(root.fg, 0.6); size: Style.font.caption }
                Text {
                  anchors.verticalCenter: parent.verticalCenter
                  width: parent.width - x; elide: Text.ElideRight
                  text: root.labelWords(row.modelData.kind)
                  color: Util.alpha(root.fg, 0.6)
                  font.family: Fonts.ui; font.pixelSize: Style.font.caption
                }
              }
              Text {
                id: body
                width: parent.width
                wrapMode: Text.Wrap
                maximumLineCount: 6
                elide: Text.ElideRight
                text: row.modelData.body || ""
                color: root.fg
                font.family: Fonts.ui; font.pixelSize: Style.font.body
              }
            }

            MouseArea {
              anchors.fill: parent
              cursorShape: Qt.PointingHandCursor
              onClicked: root.jumpRequested(row.modelData.eventId)
            }
          }

          // Unpin sits outside the bubble, away from the tail.
          PanelActionButton {
            fontFamily: Fonts.icon
            anchors.verticalCenter: card.verticalCenter
            anchors.right: row.own ? card.left : undefined
            anchors.left: row.own ? undefined : card.right
            anchors.rightMargin: Style.space(4); anchors.leftMargin: Style.space(4)
            iconText: Icons.close
            foreground: root.fg
            onClicked: if (root.svc) root.svc.unpinMessage(root.roomId, row.modelData.eventId)
          }
        }
      }
    }
  }
}
