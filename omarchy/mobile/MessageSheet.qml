import QtQuick
import QtQuick.Effects
import QtQuick.Controls as QQC
import qs.Commons
import qs.Ui
import "../components"
import ".."
import Quickshell.Io

// Message action sheet: the page frosts over and the pressed bubble lifts above it.
Item {
  id: root
  property var svc: null
  property color fg: Color.menu.text
  property color accent: Color.accent
  property color surface: Util.alpha(Color.popups.background, 0.98)
  // A tone dark enough for a light chat theme; see ChatPage.deepChipC.
  property color deepChip: Color.background
  property color chip: Util.alpha(Color.background, 0.85)
  property string roomId: ""
  property real cardRadius: Style.space(22)
  property var item: null
  property bool forwardMode: false
  property bool drawerOpen: false
  signal act(string action, var item)
  signal shiftRequested(real dy)
  visible: item !== null

  readonly property bool ownMsg: !!(root.item && root.item.isOwn)
  readonly property bool mediaKind: !!(root.item && (root.item.kind === "image" || root.item.kind === "video" || root.item.kind === "file" || root.item.kind === "audio"))
  readonly property bool hasCaption: {
    if (!root.item) return false
    var b = root.item.body || ""
    if (b === "") return false
    var fn = (root.item.media && root.item.media.filename) ? root.item.media.filename : ""
    return b !== fn
  }
  property real holeX: 0
  property real holeY: 0
  property real holeW: 0
  property real holeH: 0
  property real destY: 0    // where the bubble will land; pill/menu wait there

  property real copyY: 0
  property bool animateCopy: false
  property bool closing: false
  property bool frostOut: false
  // Close in two phases: fly home behind the frost, then melt the frost.
  property bool handback: false
  // Two-frame handback so the original bubble is never absent.
  SequentialAnimation {
    id: closeAnim
    NumberAnimation { id: closeAnimY; target: root; property: "copyY"; duration: 200; easing.type: Easing.OutBack; easing.overshoot: 1.25 }
    // Same-frame invisible swap; an overlap double-composites the translucent fill.
    ScriptAction { script: { root.item = null; root.srcItem = null; root.forwardMode = false; root.closing = false; root.frostOut = false } }
  }
  // The spotlight never escapes the list viewport.
  property real vpTop: 0
  property real vpBottom: 1e6
  property real shiftApplied: 0
  readonly property real pad: Style.space(4)
  readonly property real gap: Style.space(10)

  property var srcItem: null
  property Item pagesItem: null
  /// The action list. `openFor` sizes the sheet from `actionsFor(...).length`.
  function actionsFor(it) {
    if (!it) return []
    if (it.sendState === "failed" || it.sendState === "sending") {
      return [
        { t: it.sendState === "failed" ? "Try again" : "Retry now", a: "retry", icon: Icons.retry, danger: false, mirror: false },
        { t: "Copy", a: "copy", icon: Icons.copy, danger: false, mirror: false },
        { t: "Delete", a: "cancelsend", icon: Icons.trash, danger: true, mirror: false }
      ]
    }
    var out = [
      { t: "Reply", a: "reply", icon: Icons.replyArrow, danger: false, mirror: false },
      { t: "Forward", a: "forward", icon: Icons.forward, danger: false, mirror: false },
      { t: "Copy", a: "copy", icon: Icons.copy, danger: false, mirror: false }
    ]
    // Threads and pins need a real event id, so neither is offered for a local echo.
    if (it.eventId) {
      // Matrix has no nested threads, so replying from inside one opens the thread.
      out.push(it.threadRoot
        ? { t: "Open thread", a: "openthread", icon: Icons.thread, danger: false, mirror: false }
        : { t: it.threadSummary ? "Open thread" : "Reply in thread", a: it.threadSummary ? "openthread" : "thread", icon: Icons.thread, danger: false, mirror: false })
      var isPinned = !!(root.svc && root.svc.isPinned(root.svc.roomOfKey(root.roomId), it.eventId))
      out.push({ t: isPinned ? "Unpin" : "Pin", a: "pin", icon: Icons.keep, danger: false, mirror: false })
    }
    // Not polls: the edit flow is text-in-the-composer and would replace the poll.
    if (it.kind !== "poll" && it.can && it.can.edit) {
      out.push(root.mediaKind
        ? { t: root.hasCaption ? "Edit caption" : "Add caption", a: "caption", icon: Icons.edit, danger: false, mirror: false }
        : { t: "Edit", a: "edit", icon: Icons.edit, danger: false, mirror: false })
    }
    // Ending needs redact power, and a closed poll cannot be reopened.
    if (it.kind === "poll" && it.poll && !it.poll.ended && it.can && it.can.redact)
      out.push({ t: "End poll", a: "endpoll", icon: Icons.stop, danger: false, mirror: false })
    // Stopping a live share is not a redaction: beacon_info goes out again with
    // live:false. Only offered while the engine is still publishing.
    if (it.kind === "liveLocation" && it.isOwn && it.liveShare && it.liveShare.live
        && root.svc && root.svc.liveSharing)
      out.push({ t: "Stop", a: "stoplive", icon: Icons.stop, danger: false, mirror: false })
    if (it.can && it.can.redact)
      out.push({ t: "Delete", a: "redact", icon: Icons.trash, danger: true, mirror: false })
    return out
  }

  function openFor(it, bx, by, bw, bh, bubbleObj) {
    if (root.closing) { closeAnim.stop(); root.closing = false }
    bx = Math.round(bx); by = Math.round(by); bw = Math.round(bw); bh = Math.round(bh)
    root.srcItem = bubbleObj || null
    root.forwardMode = false
    root.drawerOpen = false
    drawerPicker.load()          // no-op once loaded
    root.item = it
    root.holeX = bx; root.holeY = by; root.holeW = bw; root.holeH = bh
    root.destY = by
    root.animateCopy = false
    root.copyY = by
    Qt.callLater(function() {
      if (!root.item) return
      var pillH = Style.space(48)
      // Analytic menu height: measuring menuBox on a cold first open races the
      // Repeater and reads ~0, collapsing the layout.
      var rows = root.actionsFor(it).length
      var menuH = rows * Style.space(38) + Style.space(16)
      var contentH = pillH + root.gap + bh + root.gap + menuH
      var top = Math.max(Style.space(12), Math.min(by - pillH - root.gap, root.height - contentH - Style.space(12)))
      root.destY = Math.round(top + pillH + root.gap)
      root.animateCopy = true
      root.copyY = root.destY
    })
  }
  function close() {
    if (root.item === null || root.closing) return
    root.closing = true
    root.frostOut = true
    root.drawerOpen = false
    root.animateCopy = false
    closeAnimY.to = root.holeY
    closeAnim.start()
  }

  function forwardTo(rid) {
    var it = root.item
    if (!it || !root.svc) return
    if (it.kind === "image" && it.media && (it.media.path || it.media.thumbnailPath)) root.svc.sendFiles(rid, [it.media.path || it.media.thumbnailPath])
    else root.svc.sendText(rid, it.body, {})
    root.close()
  }

  // Frost: a blurred, dimmed snapshot of the page with a hole masked to the
  // bubble's own corner radii.
  Item {
    anchors.fill: parent
    visible: root.item !== null
    opacity: (root.item !== null && !root.frostOut) ? 1 : 0
    Behavior on opacity { NumberAnimation { duration: 200; easing.type: Easing.OutCubic } }
    layer.enabled: true
    layer.smooth: true
    layer.effect: MultiEffect {
      blurEnabled: true; blur: 0.6; blurMax: 48; autoPaddingEnabled: false
      maskEnabled: true; maskSource: cardMask
    }
    Rectangle { anchors.fill: parent; color: Qt.rgba(Color.menu.background.r, Color.menu.background.g, Color.menu.background.b, 1) }
    ShaderEffectSource { anchors.fill: parent; sourceItem: root.pagesItem; live: true }
    Rectangle { anchors.fill: parent; color: Util.alpha("#000000", 0.55) }
  }
  Item {
    id: cardMask
    anchors.fill: parent
    layer.enabled: true
    layer.smooth: true
    visible: false
    Rectangle { anchors.fill: parent; radius: root.cardRadius; color: "black"; antialiasing: true }
  }
  MouseArea {
    anchors.fill: parent
    onClicked: { if (root.drawerOpen) root.drawerOpen = false; else root.close() }
    onWheel: function(w) { w.accepted = true }
  }

  Item {
    x: 0
    y: root.vpTop
    width: parent.width
    height: Math.max(0, Math.min(root.vpBottom, root.height) - root.vpTop)
    clip: true
    Item {
    x: root.holeX
    y: root.copyY - root.vpTop
    width: root.holeW
    height: root.holeH
    visible: root.item !== null
    Behavior on y { enabled: root.animateCopy; NumberAnimation { duration: 220; easing.type: Easing.OutBack; easing.overshoot: 1.1 } }
    // No opaque backing: the lift uses the bubble's own translucent pixels.
    Rectangle {
      anchors.fill: parent
      visible: root.srcItem === null
      antialiasing: true
      color: Util.alpha(root.surface, 0.97)
      radius: Style.space(16)
    }
    ShaderEffectSource { anchors.fill: parent; sourceItem: root.srcItem; live: true; hideSource: !root.handback; visible: root.srcItem !== null }
    Text {
      visible: root.srcItem === null
      anchors.fill: parent; anchors.margins: Style.space(10)
      // A poll's `body` is the plain-text fallback; the copy shows only the question.
      text: {
        if (!root.item) return ""
        if (root.item.kind === "poll" && root.item.poll) return root.item.poll.question || ""
        if (root.item.kind === "location" && root.item.location) return root.item.location.description || "Location"
        return root.item.body || ""
      }
      color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body
      wrapMode: Text.Wrap; elide: Text.ElideRight
    }
    }
  }


  // Reactions pill (above the spotlight)
  Rectangle {
    id: pill
    visible: !root.drawerOpen
    opacity: (root.item !== null && !root.closing) ? 1 : 0
    Behavior on opacity { NumberAnimation { duration: 130 } }
    transform: Scale {
      origin.x: pill.width / 2; origin.y: pill.height / 2
      xScale: (root.item !== null && !root.closing) ? 1 : 0.4
      Behavior on xScale { NumberAnimation { duration: 160; easing.type: Easing.OutCubic } }
    }
    x: Math.max(Style.space(12), Math.min(root.ownMsg ? root.holeX + root.holeW - width : root.holeX, parent.width - width - Style.space(12)))
    y: root.destY - root.gap - height
    width: reactRow.implicitWidth + Style.space(20); height: Style.space(48); radius: height / 2
    antialiasing: true
    color: root.surface
    Row {
      id: reactRow
      anchors.centerIn: parent
      spacing: Style.space(6)
      Repeater {
        model: ["👍", "❤️", "😂", "😮", "😢", "😡"]
        delegate: Rectangle {
          required property var modelData
          width: Style.space(36); height: Style.space(36); radius: height / 2
          color: qh.containsMouse ? Util.alpha(root.accent, 0.32) : "transparent"
          // Emoji need their ink centred: variation selectors and surrogate pairs
          // make the advance wider than the visible glyph, so x is corrected, not y.
          TextMetrics { id: em; font.pixelSize: Style.space(19); text: modelData }
          Text {
            text: modelData
            font.pixelSize: Style.space(19)
            anchors.verticalCenter: parent.verticalCenter
            x: (parent.width - em.tightBoundingRect.width) / 2 - em.tightBoundingRect.x
          }
          MouseArea { id: qh; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: { if (root.svc && root.item) root.svc.react(root.roomId, root.item.eventId, modelData); root.close() } }
        }
      }
      Rectangle {
        width: Style.space(36); height: Style.space(36); radius: height / 2
        color: ah.containsMouse ? Util.alpha(root.accent, 0.32) : "transparent"
        IconLabel { anchors.centerIn: parent; icon: Icons.emojiMore; color: root.fg; size: Style.space(19) }
        MouseArea { id: ah; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: root.drawerOpen = true }
      }
    }
  }

  // Action menu (below the spotlight)
  Rectangle {
    id: menuBox
    visible: !root.drawerOpen
    opacity: (root.item !== null && !root.closing) ? 1 : 0
    Behavior on opacity { NumberAnimation { duration: 130 } }
    transform: Scale {
      origin.x: menuBox.width / 2; origin.y: 0
      yScale: (root.item !== null && !root.closing) ? 1 : 0.4
      Behavior on yScale { NumberAnimation { duration: 160; easing.type: Easing.OutCubic } }
    }
    x: Math.max(Style.space(12), Math.min(root.ownMsg ? root.holeX + root.holeW - width : root.holeX, parent.width - width - Style.space(12)))
    y: Math.min(root.destY + root.holeH + root.gap, root.height - height - Style.space(12))
    width: Style.space(220)
    height: menuCol.implicitHeight + Style.space(16)
    radius: Style.space(16)
    antialiasing: true
    color: root.surface
    Column {
      id: menuCol
      anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
      anchors.margins: Style.space(8)
      Text { visible: root.forwardMode; text: "Forward to"; color: Util.alpha(root.fg, 0.6); font.family: Fonts.ui; font.pixelSize: Style.font.caption; font.bold: true; leftPadding: Style.space(8); bottomPadding: Style.space(4) }
      Repeater {
        model: root.forwardMode
          ? (root.svc ? root.svc.rooms.filter(function(r) { return !r.isSpace && !r.isInvite && r.id !== root.roomId }).slice(0, 8).map(function(r) { return { t: r.name || r.id, a: "fwd:" + r.id, icon: "", danger: false, mirror: false } }) : [])
          : root.actionsFor(root.item)
        delegate: Rectangle {
          required property var modelData
          width: parent.width; height: Style.space(38); radius: Style.space(10)
          color: mh.containsMouse ? Util.alpha(root.fg, 0.08) : "transparent"
          Row {
            anchors.verticalCenter: parent.verticalCenter; anchors.left: parent.left; anchors.leftMargin: Style.space(10); spacing: Style.space(12)
            Item {
              visible: modelData.icon !== ""
              width: Style.space(20); height: Style.space(20)
              anchors.verticalCenter: parent.verticalCenter
              Text {
                anchors.centerIn: parent
                text: modelData.icon
                color: modelData.danger ? Color.urgent : Util.alpha(root.fg, 0.75)
                font.family: Fonts.icon; renderType: Text.NativeRendering; font.pixelSize: Style.font.icon
                transform: Scale { xScale: modelData.mirror ? -1 : 1; origin.x: implicitWidth / 2; origin.y: implicitHeight / 2 }
              }
            }
            Text { text: modelData.t; color: modelData.danger ? Color.urgent : root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body; anchors.verticalCenter: parent.verticalCenter; elide: Text.ElideRight; width: Style.space(150) }
          }
          MouseArea {
            id: mh; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor
            onClicked: {
              var it = root.item
              if (modelData.a === "redact") { root.confirmItem = it; return }
              root.close(); root.act(modelData.a, it)
            }
          }
        }
      }
    }
  }

  // Delete confirmation: slides up from the bottom (redactions are permanent)
  property var confirmItem: null
  Item {
    anchors.fill: parent
    z: 60
    Rectangle {
      anchors.fill: parent; radius: root.cardRadius; antialiasing: true; color: "#000000"
      opacity: root.confirmItem ? 0.5 : 0
      visible: opacity > 0
      Behavior on opacity { NumberAnimation { duration: 180 } }
      MouseArea { anchors.fill: parent; enabled: root.confirmItem !== null; onClicked: root.confirmItem = null }
    }
    Rectangle {
      anchors.left: parent.left; anchors.right: parent.right
      height: confCol.implicitHeight + Style.space(26)
      y: root.confirmItem ? parent.height - height : parent.height + Style.space(6)
      topLeftRadius: Style.space(20); topRightRadius: Style.space(20)
      bottomLeftRadius: root.cardRadius; bottomRightRadius: root.cardRadius
      antialiasing: true
      color: root.surface
      Behavior on y { NumberAnimation { duration: 220; easing.type: Easing.OutCubic } }
      MouseArea { anchors.fill: parent }
      Column {
        id: confCol
        anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
        anchors.margins: Style.space(16); anchors.topMargin: Style.space(10)
        spacing: Style.space(10)
        Rectangle { width: Style.space(36); height: Style.space(4); radius: 2; color: Util.alpha(root.fg, 0.25); anchors.horizontalCenter: parent.horizontalCenter }
        Text { width: parent.width; horizontalAlignment: Text.AlignHCenter; text: "Delete message?"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.subtitle; font.bold: true }
        Text { width: parent.width; horizontalAlignment: Text.AlignHCenter; wrapMode: Text.Wrap; text: "This can't be undone — the message is removed for everyone."; color: Util.alpha(root.fg, 0.6); font.family: Fonts.ui; font.pixelSize: Style.font.caption }
        Item { width: 1; height: Style.space(2) }
        Row {
          anchors.horizontalCenter: parent.horizontalCenter
          spacing: Style.space(12)
          Rectangle {
            width: Style.space(110); height: Style.space(40); radius: height / 2
            antialiasing: true
            color: root.deepChip
            Text { anchors.centerIn: parent; text: "Cancel"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body }
            MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.confirmItem = null }
          }
          Rectangle {
            width: Style.space(110); height: Style.space(40); radius: height / 2
            antialiasing: true
            color: Color.urgent
            Text { anchors.centerIn: parent; text: "Delete"; color: Color.background; font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: true }
            MouseArea {
              anchors.fill: parent; cursorShape: Qt.PointingHandCursor
              onClicked: { var it = root.confirmItem; root.confirmItem = null; root.close(); root.act("redact", it) }
            }
          }
        }
      }
    }
  }

  // Emoji drawer — the same EmojiPicker the attachment sheet uses.
  function loadEmojis() { drawerPicker.load() }

  Rectangle {
    id: drawer
    anchors.left: parent.left; anchors.right: parent.right
    height: Style.space(360)
    y: root.drawerOpen ? parent.height - height : parent.height + Style.space(6)
    topLeftRadius: Style.space(20); topRightRadius: Style.space(20)
    bottomLeftRadius: root.cardRadius; bottomRightRadius: root.cardRadius
    antialiasing: true
    color: root.surface
    Behavior on y { NumberAnimation { duration: 220; easing.type: Easing.OutCubic } }
    MouseArea { anchors.fill: parent; onClicked: {} }

    Column {
      anchors.fill: parent; anchors.margins: Style.space(12); anchors.bottomMargin: Style.space(8)
      spacing: Style.space(8)
      Rectangle { width: Style.space(36); height: Style.space(4); radius: 2; color: Util.alpha(root.fg, 0.25); anchors.horizontalCenter: parent.horizontalCenter }
      EmojiPicker {
        id: drawerPicker
        width: parent.width
        height: parent.height - Style.space(16)
        fg: root.fg; accent: root.accent
        chip: root.chip
        onPicked: function(e) {
          if (root.svc && root.item) root.svc.react(root.roomId, root.item.eventId, e)
          root.close()
        }
      }
    }
  }
}
