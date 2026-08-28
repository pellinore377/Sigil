import QtQuick
import QtQuick.Controls as QQC
import Quickshell.Io
import qs.Commons
import qs.Ui
import ".."
import "../components"

// Attachment sheet under the composer. Each category opens its own page inside
// the same panel rather than a separate popup.
Item {
  id: root
  property var svc: null
  property string roomId: ""
  property color fg: Color.menu.text
  property color accent: Color.accent
  property color surface: Util.alpha(Color.menu.text, 0.07)
  property color chip: Util.alpha(Color.background, 0.85)
  /// Opaque tone for anything that has to be read over a map.
  property color deepChip: Color.popups.background
  property string page: "grid"          // grid | poll | pin | stickers | emoji | note
  property string note: ""

  signal pickFiles()
  signal insertEmoji(string ch)
  signal closeRequested()

  implicitHeight: root.page === "poll" ? Style.space(408)
    : (root.page === "pin" || root.page === "current" || root.page === "live") ? Style.space(430)
    : (root.page === "emoji" || root.page === "stickers") ? Style.space(330)
    : Style.space(232)
  // No Behavior here: ChatPage animates the reveal, and animating this too
  // re-lays the page out at intermediate heights.

  function reset() { root.page = "grid"; root.note = "" }
  function debugPicker() { return locPicker.debugState() }

  readonly property var tiles: [
          { icon: Icons.attach, label: "Files", act: "files" },
          { icon: Icons.emoji, label: "Emojis", act: "emoji" },
          { icon: Icons.sticker, label: "Stickers", act: "stickers" },
          { icon: Icons.poll, label: "Poll", act: "poll" },
          { icon: Icons.myLocation, label: "Current\nLocation", act: "current" },
          { icon: Icons.liveLocation, label: "Live\nLocation", act: "live" },
          { icon: Icons.pinDrop, label: "Drop a Pin", act: "pin", iconScale: 1.3 }
  ]

  function activate(act) {
    if (act === "files") { root.pickFiles(); root.closeRequested(); return }
    if (act === "emoji") { emojiPicker.load(); root.page = "emoji"; return }
    if (act === "stickers") { root.loadStickers(); root.page = "stickers"; return }
    if (act === "poll") { if (pollOptions.count === 0) root.resetPoll(); root.page = "poll"; return }
    if (act === "pin") { locPicker.reset(); root.page = "pin"; return }
    if (act === "current") { locPicker.reset(); root.page = "current"; return }
    if (act === "live") { locPicker.reset(); root.page = "live"; return }
    root.note = "That is not something this sheet can do yet."
    root.page = "note"
  }

  property var stickerPacks: []
  function loadStickers() {
    if (!root.svc) return
    root.svc.listStickers(function(r, e) { root.stickerPacks = (r && r.packs) ? r.packs : [] })
  }

  // Chrome
  Rectangle {
    anchors.fill: parent
    anchors.margins: Style.space(10)
    radius: Style.space(18)
    antialiasing: true
    color: root.surface
  }

  Item {
    id: header
    visible: root.page !== "grid" && root.page !== "pin"
             && root.page !== "current" && root.page !== "live"
    anchors.top: parent.top; anchors.left: parent.left; anchors.right: parent.right
    anchors.topMargin: Style.space(16); anchors.leftMargin: Style.space(18); anchors.rightMargin: Style.space(18)
    height: visible ? Style.space(30) : 0
    z: 5
    Rectangle {
      anchors.left: parent.left; anchors.verticalCenter: parent.verticalCenter
      width: Style.space(30); height: width; radius: width / 2
      color: bh.hovered ? Qt.lighter(root.chip, 1.2) : root.chip
      IconLabel { anchors.centerIn: parent; icon: Icons.back; color: root.fg; size: Style.font.body }
      HoverHandler { id: bh }
      MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.reset() }
    }
    Text {
      anchors.centerIn: parent
      text: root.page === "poll" ? "New poll"
          : root.page === "pin" ? "Drop a pin"
          : root.page === "current" ? "Current location"
          : root.page === "live" ? "Live location"
          : root.page === "stickers" ? "Stickers"
          : root.page === "emoji" ? "Emoji" : ""
      color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: true
    }
  }

  // Grid — the block is centred while each row still fills left to right.
  Grid {
    visible: root.page === "grid"
    anchors.centerIn: parent
    columns: 5
    spacing: Style.space(10)

    Repeater {
      model: root.tiles
      delegate: Item {
        required property var modelData
        width: Style.space(62); height: Style.space(72)
        Rectangle {
          id: disc
          anchors.horizontalCenter: parent.horizontalCenter
          width: Style.space(58); height: Style.space(40)
          radius: height / 2
          antialiasing: true
          color: hov.hovered ? Qt.lighter(root.chip, 1.2) : root.chip
          scale: hov.hovered ? 1.05 : 1
          Behavior on scale { NumberAnimation { duration: 110; easing.type: Easing.OutCubic } }
          // Per-tile scale: Material Symbols glyphs differ in width at a given size.
          IconLabel { anchors.centerIn: parent; icon: modelData.icon; color: root.fg
                      size: Style.font.icon * (modelData.iconScale || 1) }
        }
        Text {
          // Anchored to the tile, not to `disc.bottom`: the disc's hover `scale`
          // is a render transform, so the label would appear to drift.
          anchors.top: parent.top; anchors.topMargin: Style.space(45)
          anchors.horizontalCenter: parent.horizontalCenter
          width: parent.width
          horizontalAlignment: Text.AlignHCenter
          text: modelData.label
          color: Util.alpha(root.fg, 0.75)
          font.family: Fonts.ui; font.pixelSize: Style.space(10)
          wrapMode: Text.Wrap
        }
        HoverHandler { id: hov }
        MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.activate(modelData.act) }
      }
    }
  }

  // Note
  Text {
    visible: root.page === "note"
    anchors.centerIn: parent
    width: parent.width - Style.space(80)
    horizontalAlignment: Text.AlignHCenter
    text: root.note
    color: Util.alpha(root.fg, 0.75)
    wrapMode: Text.Wrap
    font.family: Fonts.ui; font.pixelSize: Style.font.body
  }

  // Poll
  ListModel { id: pollOptions }
  function resetPoll() {
    pollOptions.clear()
    pollOptions.append({ text: "" })
    pollOptions.append({ text: "" })
    pollQuestion.text = ""
    root.pollClosed = false
  }
  property bool pollClosed: false
  readonly property color fieldBg: root.chip
  readonly property color fieldBgFocus: Qt.lighter(root.chip, 1.15)
  readonly property bool pollValid: {
    if (pollQuestion.text.trim() === "") return false
    var n = 0
    for (var i = 0; i < pollOptions.count; i++) if (String(pollOptions.get(i).text).trim() !== "") n++
    return n >= 2
  }

  Item {
    visible: root.page === "poll"
    anchors.fill: parent
    anchors.margins: Style.space(20)
    anchors.topMargin: Style.space(52)

    QQC.ScrollView {
      id: pollScroll
      anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
      anchors.bottom: pollFooter.top; anchors.bottomMargin: Style.space(8)
      clip: true
      QQC.ScrollBar.horizontal.policy: QQC.ScrollBar.AlwaysOff

      Column {
        width: pollScroll.width
        spacing: Style.space(6)

        Text {
          text: "Poll type"
          color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall; font.bold: true
        }
        Row {
          spacing: Style.space(6)
          Repeater {
            model: [ { t: "Open", closed: false }, { t: "Closed", closed: true } ]
            delegate: Rectangle {
              required property var modelData
              readonly property bool sel: root.pollClosed === modelData.closed
              width: Style.space(84); height: Style.space(28); radius: height / 2
              color: sel ? root.accent : root.chip
              Text {
                anchors.centerIn: parent; text: modelData.t
                color: parent.sel ? Color.background : root.fg
                font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall; font.bold: parent.sel
              }
              MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.pollClosed = modelData.closed }
            }
          }
        }
        Text {
          width: parent.width
          text: root.pollClosed ? "Results stay hidden until you close the poll"
                                : "Voters see results as soon as they have voted"
          color: Util.alpha(root.fg, 0.6); wrapMode: Text.Wrap
          font.family: Fonts.ui; font.pixelSize: Style.space(10)
          bottomPadding: Style.space(4)
        }

        Text {
          text: "Question"
          color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall; font.bold: true
        }
        Rectangle {
          width: parent.width; height: Style.space(34); radius: Style.space(10)
          color: pollQuestion.activeFocus ? root.fieldBgFocus : root.fieldBg
          QQC.TextField {
            id: pollQuestion
            anchors.fill: parent; anchors.leftMargin: Style.space(12); anchors.rightMargin: Style.space(12)
            verticalAlignment: TextInput.AlignVCenter
            placeholderText: "Ask something…"
            placeholderTextColor: Util.alpha(root.fg, 0.55)
            color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body
            background: Item {}
            QQC.ContextMenu.menu: null
            TextContextMenu { editor: parent }
          }
        }

        Item { width: 1; height: Style.space(4) }
        Text {
          text: "Options"
          color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall; font.bold: true
        }

        Repeater {
          model: pollOptions
          delegate: Row {
            required property int index
            required property string text
            width: pollScroll.width
            spacing: Style.space(6)
            Rectangle {
              width: parent.width - (pollOptions.count > 2 ? Style.space(34) + Style.space(6) : 0)
              height: Style.space(32); radius: Style.space(10)
              color: optField.activeFocus ? root.fieldBgFocus : root.fieldBg
              QQC.TextField {
                id: optField
                anchors.fill: parent; anchors.leftMargin: Style.space(12); anchors.rightMargin: Style.space(10)
                verticalAlignment: TextInput.AlignVCenter
                text: parent.parent.text
                placeholderText: "Option " + (index + 1)
                placeholderTextColor: Util.alpha(root.fg, 0.55)
                color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
                background: Item {}
                QQC.ContextMenu.menu: null
                TextContextMenu { editor: parent }
                // Keep the model authoritative: reading a Repeater's fields back
                // by index breaks once rows are removed.
                onTextChanged: pollOptions.setProperty(index, "text", text)
              }
            }
            Rectangle {
              visible: pollOptions.count > 2
              width: Style.space(28); height: width; radius: width / 2
              anchors.verticalCenter: parent.verticalCenter
              color: rmh.hovered ? Qt.lighter(root.chip, 1.2) : root.chip
              Text { anchors.centerIn: parent; text: "\u00d7"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body }
              HoverHandler { id: rmh }
              MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: pollOptions.remove(index) }
            }
          }
        }

        Item {
          width: parent.width; height: Style.space(30)
          visible: pollOptions.count < 8
          Row {
            anchors.left: parent.left; anchors.verticalCenter: parent.verticalCenter
            spacing: Style.space(6)
            IconLabel { icon: Icons.plus; color: root.accent; anchors.verticalCenter: parent.verticalCenter; size: Style.font.bodySmall }
            Text { text: "Add option"; color: root.accent; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall; font.underline: addHover.hovered; anchors.verticalCenter: parent.verticalCenter }
          }
          HoverHandler { id: addHover }
          MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: pollOptions.append({ text: "" }) }
        }
      }
    }

    Item {
      id: pollFooter
      anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom
      height: Style.space(34)
      Row {
        anchors.right: parent.right
        spacing: Style.space(8)
        Rectangle {
          width: Style.space(72); height: Style.space(30); radius: height / 2
          color: cancelHover.hovered ? Qt.lighter(root.chip, 1.2) : root.chip
          Text { anchors.centerIn: parent; text: "Cancel"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall }
          HoverHandler { id: cancelHover }
          MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.reset() }
        }
        Rectangle {
          width: Style.space(96); height: Style.space(30); radius: height / 2
          color: root.pollValid ? root.accent : root.chip
          opacity: root.pollValid ? 1 : 0.7
          Text {
            anchors.centerIn: parent; text: "Create poll"
            color: root.pollValid ? Color.background : Util.alpha(root.fg, 0.5)
            font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall; font.bold: root.pollValid
          }
          MouseArea {
            anchors.fill: parent
            cursorShape: root.pollValid ? Qt.PointingHandCursor : Qt.ArrowCursor
            onClicked: {
              if (!root.pollValid) return
              var opts = []
              for (var i = 0; i < pollOptions.count; i++) {
                var t = String(pollOptions.get(i).text).trim()
                if (t !== "") opts.push(t)
              }
              root.svc.createPoll(root.roomId, pollQuestion.text.trim(), opts, root.pollClosed, function(r, e) {
                if (e) { root.note = "Could not create the poll: " + e; root.page = "note"; return }
                root.resetPoll()
                root.closeRequested()
              })
            }
          }
        }
      }
    }
  }

  // Location pages — one picker, three modes.
    LocationPicker {
      id: locPicker
      visible: root.page === "pin" || root.page === "current" || root.page === "live"
      anchors.fill: parent
      anchors.topMargin: 0
      mode: root.page === "current" ? "current" : (root.page === "live" ? "live" : "pin")
      svc: root.svc
      fg: root.fg
      accent: root.accent
      surface: Util.alpha(root.fg, 0.10)
      chip: root.chip
      menuSurface: root.deepChip
      onBackRequested: root.page = "grid"
      onCloseRequested: root.closeRequested()
      onShareRequested: function (lat, lon, durationMs) {
        if (!root.svc) return
        var done = function (r, e) {
          if (e) { root.note = e; root.page = "note"; return }
          root.closeRequested()
        }
        // A duration means MSC3489: a beacon the engine republishes until it
        // expires. Everything else is a single point; "current location" is m.self.
        if (durationMs > 0) root.svc.startLiveLocation(root.roomId, durationMs, done)
        else root.svc.sendLocation(root.roomId, lat, lon, "", root.page === "current", done)
      }
    }

  // Stickers
  Item {
    visible: root.page === "stickers"
    anchors.fill: parent
    anchors.margins: Style.space(22)
    anchors.topMargin: Style.space(54)

    Text {
      visible: root.stickerPacks.length === 0
      anchors.centerIn: parent
      width: parent.width
      horizontalAlignment: Text.AlignHCenter
      text: "No sticker packs yet.\nAdd one in Element (im.ponies.user_emotes) and it shows up here."
      color: Util.alpha(root.fg, 0.6); wrapMode: Text.Wrap
      font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
    }

    QQC.ScrollView {
      anchors.fill: parent
      visible: root.stickerPacks.length > 0
      clip: true
      Flow {
        width: parent.width
        spacing: Style.space(8)
        Repeater {
          model: root.stickerPacks.length > 0 ? root.stickerPacks[0].stickers : []
          delegate: Item {
            required property var modelData
            width: Style.space(56); height: Style.space(56)
            Image {
              anchors.fill: parent
              fillMode: Image.PreserveAspectFit
              asynchronous: true
              source: modelData.path ? "file://" + modelData.path : ""
            }
            Rectangle {
              anchors.fill: parent; radius: Style.space(8); color: "transparent"
              border.width: sh.hovered ? 2 : 0; border.color: Util.alpha(root.accent, 0.7)
            }
            HoverHandler { id: sh }
            MouseArea {
              anchors.fill: parent; cursorShape: Qt.PointingHandCursor
              onClicked: root.svc.sendSticker(root.roomId, modelData, function(r, e) {
                if (e) { root.note = "Could not send the sticker: " + e; root.page = "note"; return }
                root.closeRequested()
              })
            }
          }
        }
      }
    }
  }

  // Emoji
  Item {
    visible: root.page === "emoji"
    anchors.fill: parent
    anchors.margins: Style.space(20)
    anchors.topMargin: Style.space(52)
    EmojiPicker {
      id: emojiPicker
      anchors.fill: parent
      fg: root.fg; accent: root.accent; chip: root.chip
      onPicked: function(e) { root.insertEmoji(e) }
    }
  }
}
