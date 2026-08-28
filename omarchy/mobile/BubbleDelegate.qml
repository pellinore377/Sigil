import QtQuick
import QtQuick.Effects
import Quickshell
import qs.Commons
import qs.Ui
import "../components"

// One timeline row, Google-Messages style: own messages right in accent bubbles,
// others left with an avatar on group starts; day chips; tiny state captions.
Item {
  id: root
  property var svc: null
  property color fg: Color.menu.text
  property string roomId: ""
  property bool dm: false
  property bool encrypted: false
  property string themeAccent: ""
  property string playingVoice: ""       // eventId currently playing
  property real voicePos: 0
  readonly property var flatWave: [0.4,0.6,0.35,0.7,0.5,0.8,0.45,0.65,0.4,0.75,0.5,0.6,0.35,0.7,0.55,0.45,0.6,0.4,0.7,0.5]
  signal voiceToggled(var item)
  signal voiceSeeked(var item, real pos)
  // Fit a waveform to however many bars the row can show (crops looked short).
  function resampleWave(arr, n) {
    if (!arr || arr.length === 0 || n <= 0) return []
    var out = []
    for (var i = 0; i < n; i++) {
      var a = Math.floor(i * arr.length / n)
      var b = Math.max(a + 1, Math.floor((i + 1) * arr.length / n))
      var peak = 0
      for (var j = a; j < b && j < arr.length; j++) peak = Math.max(peak, arr[j])
      out.push(peak)
    }
    return out
  }

  function toggleVoice(it) { root.voiceToggled(it) }
  function seekVoice(it, pos) { root.voiceSeeked(it, pos) }
  property color themeSurface: "transparent"
  required property var model
  required property int index
  property var page: null
  // Entry animation: theirs slides in from their side, ours rises from the composer.
  property real entryDx: 0
  property real entryDy: 0
  property real entryScale: 1
  property real entryOpacity: 1
  transform: [
    Translate { x: root.entryDx; y: root.entryDy },
    Scale {
      origin.x: root.own ? root.width : 0
      origin.y: root.height
      xScale: root.entryScale
      yScale: root.entryScale
    }
  ]
  opacity: root.entryOpacity
  ParallelAnimation {
    id: entryAnim
    NumberAnimation { target: root; property: "entryDx"; to: 0; duration: 300; easing.type: Easing.OutCubic }
    NumberAnimation { target: root; property: "entryDy"; to: 0; duration: 320; easing.type: Easing.OutBack; easing.overshoot: 0.8 }
    NumberAnimation { target: root; property: "entryScale"; to: 1; duration: 320; easing.type: Easing.OutBack; easing.overshoot: 1.0 }
    NumberAnimation { target: root; property: "entryOpacity"; to: 1; duration: 190; easing.type: Easing.OutCubic }
  }
  function playEntry() {
    if (root.own) {
      // Up and out of the composer, which sits just below the list.
      root.entryDx = 0
      root.entryDy = Style.space(54)
      root.entryScale = 0.84
    } else {
      root.entryDx = -Style.space(28)
      root.entryDy = Style.space(4)
      root.entryScale = 0.9
    }
    root.entryOpacity = 0
    entryAnim.restart()
  }
  /// First-pass work for this message; any future reuse hook must call it too.
  function adopt() {
    // `model.id`, not `eventId`: a local echo has no event id yet.
    if (root.isMsg && root.index === 0 && root.page && root.page.claimEntry(model.id))
      root.playEntry()
    if (root.page && root.page.noteBuild) root.page.noteBuild(model.kind || "?")
    root.requestDocThumb()
    root.requestTrackInfo()
    root.requestVcard()
  }
  Component.onCompleted: root.adopt()

  property bool recycling: false   // see ChatPage: recycling is off
  /// Pinned state comes from the ROOM, not the view: a thread or pins list is keyed `<roomId>|…`.
  /// Set when this message started a thread: `{count, sender, senderName, body}`.
  readonly property var threadSummary: (root.page && root.page.threadFor) ? root.page.threadFor(model.eventId || "") : null
  readonly property color accentC: root.themeAccent !== "" ? Qt.color(root.themeAccent) : Color.accent
  signal openThreadRequested(string rootId)
  /// The pin protrudes above the bubble, so the row reserves the overhang.
  readonly property real pinLift: root.pinned ? Style.space(4) : 0
  readonly property bool jumpFlash: !!(root.page && model.eventId && root.page.jumpedTo === model.eventId)
  readonly property bool pinned: !!(root.svc && model.eventId
                                    && root.svc.isPinned(root.svc.roomOfKey(root.roomId), model.eventId))


  // The media's on-screen rect travels with the signal, so the viewer can grow out of it.
  signal openLocation(var item, var from)
  signal openDocument(var item)
  /// The full player, for a music file. A voice note has nowhere to expand to.
  signal openAudio(var item)
  /// Open (or create) a direct message with someone from a contact card.
  signal openDmWith(string userId)
  /// Write a `.vcf` for a Matrix contact and hand it to the desktop.
  signal shareVcf(string userId, string displayName)
  signal openImage(var item, var from)
  signal playVideo(var item, var from)
  /// Test hook: the media tap, without synthetic input.
  function debugOpenMedia() {
    if (root.photoKind) { root.openImage(root.model, root.mediaRect(imgBox)); return "image" }
    if (model.kind === "video") { root.playVideo(root.model, root.mediaRect(vidBox)); return "video" }
    return "none"
  }
  function mediaRect(box) {
    var p = box.mapToItem(null, 0, 0)
    return Qt.rect(p.x, p.y, box.width, box.height)
  }
  signal menuRequested(var item, real sceneX, real sceneY, real bw, real bh, var bubbleObj)
  signal replyRequested(string eventId, string senderName, string body)

  // Test hook: simulate a long-press on this bubble.
  function pressMenu() { var p = bubble.mapToItem(null, 0, 0); root.menuRequested(root.model, p.x, p.y, bubble.width, bubble.height, bubble) }

  readonly property bool isMsg: root.svc ? root.svc.isMessageKind(model.kind) : true
  readonly property bool isState: !isMsg && model.kind !== "dayDivider" && model.kind !== "readMarker" && model.kind !== "timelineStart"
  readonly property bool own: !!model.isOwn
  readonly property bool groupStart: model.showHeader === true
  readonly property bool groupEnd: model.groupEnd !== false
  property real lastReadTs: 0
  readonly property real bubbleMax: width * 0.78
  /// A poll draws its own surface; a bubble around it gives two nested cards.
  readonly property bool pollKind: model.kind === "poll"
  readonly property bool photoKind: model.kind === "image" || model.kind === "sticker"
  // imgKind = full-bleed media layout (no bubble padding); photoKind is the still-image box.
  readonly property bool locKind: model.kind === "location" || model.kind === "liveLocation"
  readonly property bool liveLocKind: model.kind === "liveLocation"
  /// MSC3488 `m.self` means "this is where I am", so the marker is the sender, not a pin.
  readonly property bool selfLocation:
       root.liveLocKind
    || !!(model.location && model.location.asset === "m.self")
  /// The live share this bubble belongs to; matrix-sdk aggregates `beacon` updates onto `beacon_info`.
  readonly property var liveShare: root.liveLocKind ? (model.liveShare || null) : null
  readonly property bool imgKind: photoKind || model.kind === "video" || locKind
  /// Edge-to-edge content inside the bubble. `imgKind` stays media-only: the preview
  /// loader and entry animations key off it.
  readonly property bool fullBleed: root.imgKind || root.linkCardKind || root.docThumbKind
                                   || root.trackKind || root.codeKind || root.contactKind
  // Raw protocol state changes are noise, as is the second `beacon_info` that ends a live share.
  readonly property bool hiddenItem: model.kind === "liveLocationEnd"
    || (!!(root.page && root.page.debugNoNotices) && (isState || model.kind === "rtcNotification"))
    || (isState && /(org|io|im|m)\.[a-z0-9_]+\.[a-z]/.test(model.stateText || model.body || ""))
    // Pinning writes room state; the badge and the Pins page already say it.
    || (isState && /pinned events/i.test(model.stateText || model.body || ""))
  // null = follow `autoDetails`; true/false = the reader's own choice.
  // The engine emits plain <a href> and TextEdit has no linkColor, so tint anchors here.
  function themedHtml(h) {
    if (!h) return ""
    var c = String(root.themeAccent !== "" ? Qt.color(root.themeAccent) : Color.accent)
    return String(h).replace(/<a href="([^"]*)">/g, '<a href="$1"><font color="' + c + '">')
                    .replace(/<\/a>/g, '</font></a>')
  }

  property var detailsChoice: null
  readonly property bool detailsOn: root.detailsChoice === null ? root.autoDetails : root.detailsChoice === true
  /// Test hook: the delegate's internal geometry.
  function debugGeom() {
    return JSON.stringify({
      kind: model.kind,
      delegateH: Math.round(root.height),
      colH: Math.round(col.implicitHeight),
      rowH: Math.round(bubbleRow.height),
      rowY: Math.round(bubbleRow.y),
      bubbleH: Math.round(bubble.height),
      bubbleY: Math.round(bubble.y),
      innerH: Math.round(inner.implicitHeight),
      track: root.trackKind ? JSON.parse(trackBox.item.debugTone()) : null,
      detailY: Math.round(detailRow.y),
      detailH: Math.round(detailRow.height)
    })
  }
  function toggleDetails() {
    // The row's height animates after the tap, so read `atYEnd` first and hold the bottom.
    var v = root.ListView.view
    var wasEnd = v ? v.atYEnd : false
    root.detailsChoice = !root.detailsOn
    if (wasEnd && v) { holdEnd.ticks = 0; holdEnd.restart() }
  }

  Timer {
    id: holdEnd
    interval: 16; repeat: true
    property int ticks: 0
    onTriggered: {
      var v = root.ListView.view
      if (v) v.positionViewAtBeginning()
      if (++ticks > 20) { running = false; ticks = 0 }
    }
  }
  // The page decides which message owns the receipt line and who has read it.
  property string receiptEventId: ""
  property var receiptReaders: []
  property bool animateMarks: false
  property bool unreadDividerAllowed: true
  readonly property bool ownsReceipt: root.own && model.eventId !== undefined && model.eventId === root.receiptEventId
  readonly property var readers: root.ownsReceipt ? root.receiptReaders : []
  readonly property real markDot: Style.space(13)
  /// Behind a stacked read receipt: opaque, in the conversation's own tone.
  property color receiptGround: Color.menu.background
  readonly property bool sendingNow: model.sendState === "sending"
  readonly property bool failedNow: model.sendState === "failed"
  readonly property bool showMark: root.own && (root.ownsReceipt || root.sendingNow || root.failedNow)
  readonly property var readersUnused: {
    if (!root.svc || !model.eventId) return []
    var all = root.svc.receiptsByRoom[root.roomId]
    if (!all) return []
    var me = root.svc.userId
    var out = []
    for (var i = 0; i < all.length; i++) {
      var r = all[i]
      if (r.userId === me) continue
      if (r.eventId === model.eventId) out.push(r)
    }
    return out.slice(0, 4)
  }
  property string latestOwnId: ""
  readonly property int reactionCount: {
    if (!root.isMsg || !model.reactions) return 0
    return model.reactions.count !== undefined ? model.reactions.count : model.reactions.length
  }
  // How far the reaction badge rises above the bubble; the row reserves it.
  readonly property real reactionLift: root.reactionCount > 0 ? Style.space(13) : 0
  // Tap-only in normal use; the panel can force them all open for testing.
  property bool autoDetails: false

  width: parent ? parent.width : 360
  implicitHeight: hiddenItem ? 0 : col.implicitHeight
  height: implicitHeight
  visible: !hiddenItem

  Column {
    id: col
    anchors.left: parent.left; anchors.right: parent.right
    anchors.leftMargin: Style.space(14); anchors.rightMargin: Style.space(14)
    spacing: 0

    // Session stamp (GM style): plain centered caption
    Item {
      width: parent.width; height: model.dayLabel ? Style.space(30) : 0
      visible: !!model.dayLabel
      Text { anchors.centerIn: parent; text: model.dayLabel; color: Util.alpha(root.fg, 0.55); font.family: Fonts.ui; font.pixelSize: Style.font.caption; font.bold: true }
    }

    Item {
      width: parent.width; height: (model.kind === "readMarker" && root.unreadDividerAllowed) ? Style.space(20) : 0
      visible: model.kind === "readMarker" && root.unreadDividerAllowed
      Rectangle { anchors.verticalCenter: parent.verticalCenter; width: parent.width; height: 1; color: Util.alpha(Color.urgent, 0.5) }
      Rectangle { anchors.centerIn: parent; width: un.implicitWidth + Style.space(14); height: Style.space(16); radius: height / 2; color: Color.urgent
        Text { id: un; anchors.centerIn: parent; text: "Unread"; color: Color.background; font.family: Fonts.ui; font.pixelSize: Style.space(9); font.bold: true } }
    }

    // State caption
    Text {
      // An unrendered item must claim no padding, or hidden `beacon_info` events open a gap.
      visible: root.isState && (model.stateText || model.body || "") !== ""
      width: parent.width
      // Same rhythm as a bubble row, which carries its gap above itself.
      topPadding: Style.space(10); bottomPadding: 0
      horizontalAlignment: Text.AlignHCenter
      text: (model.stateText || model.body || "")
      color: Util.alpha(root.fg, 0.42); font.family: Fonts.ui; font.pixelSize: Style.font.caption
      wrapMode: Text.Wrap
    }

    // Sender header (group start, others only): avatar beside the name above.
    Item {
      id: senderHeader
      readonly property bool on: root.isMsg && !root.own && root.groupStart && !root.dm
      visible: on
      width: parent.width
      height: on ? Style.space(28) : 0
      Row {
        anchors.left: parent.left
        anchors.bottom: parent.bottom
        anchors.bottomMargin: Style.space(2)
        spacing: Style.space(7)
        Avatar {
          size: Style.space(20)
          anchors.verticalCenter: parent.verticalCenter
          source: model.senderAvatarPath || ""; name: model.senderName; userId: model.sender
          status: root.svc ? root.svc.presenceOf(model.sender) : ""
          statusBackdrop: root.themeSurface !== "transparent" ? root.themeSurface : Color.menu.background
        }
        Text {
          anchors.verticalCenter: parent.verticalCenter
          text: model.senderName || model.sender
          color: Util.alpha(root.fg, 0.6); font.family: Fonts.ui; font.pixelSize: Style.font.caption; font.bold: true
        }
      }
    }

    // Bubble row (top gap separates from the older message above)
    Item {
      id: bubbleRow
      visible: root.isMsg
      width: parent.width

      // Tapping the empty space beside the bubble shows the same details.
      TapHandler {
        acceptedButtons: Qt.LeftButton
        onSingleTapped: root.toggleDetails()
      }
      // Same gap for every kind, link-preview cards included.
      height: root.isMsg ? bubble.height + Style.space(root.groupStart ? 10 : 3) + Math.max(root.reactionLift, root.pinLift) : 0

      // Reactions hang off the bubble's inner top corner; the overlap eats the empty rounded corner.
      Row {
        id: reactionRow
        z: 3
        visible: root.reactionCount > 0
        spacing: Style.space(2)
        // Straddles the corner: centred on the top edge, half an emoji past the side.
        anchors.left: root.own ? bubble.left : undefined
        anchors.right: root.own ? undefined : bubble.right
        anchors.leftMargin: root.own ? -Style.space(9) : 0
        anchors.rightMargin: root.own ? 0 : -Style.space(9)
        anchors.verticalCenter: bubble.top
        Repeater {
          model: root.model.reactions
          // An Item, not a Row: children of a Row may not use anchors.
          delegate: Item {
            required property var model
            width: rKey.implicitWidth + (rCount.visible ? rCount.implicitWidth + Style.space(1) : 0)
            height: rKey.implicitHeight
            Text {
              id: rKey
              anchors.left: parent.left
              anchors.verticalCenter: parent.verticalCenter
              text: parent.model.key
              color: root.fg
              font.family: Fonts.ui
              font.pixelSize: Style.space(20)
              // Ours reads a touch heavier without needing a chip behind it.
              scale: parent.model.mine ? 1.0 : 0.92
            }
            Text {
              id: rCount
              anchors.left: rKey.right
              anchors.leftMargin: Style.space(1)
              anchors.verticalCenter: parent.verticalCenter
              visible: parent.model.count > 1
              text: parent.model.count
              color: parent.model.mine ? Color.accent : Util.alpha(root.fg, 0.65)
              font.family: Fonts.ui
              font.pixelSize: Style.space(11)
              font.bold: parent.model.mine
            }
            MouseArea {
              anchors.fill: parent
              anchors.margins: -Style.space(3)
              cursorShape: Qt.PointingHandCursor
              onClicked: if (root.svc) root.svc.react(root.roomId, root.model.eventId, parent.model.key)
            }
          }
        }
      }

      Rectangle {
        id: bubble
        anchors.bottom: parent.bottom
        // A link-only message renders as the preview card itself — no bubble.
        readonly property bool cardOnly: root.linkOnlyBody && lp.active
        readonly property real contentW: root.locKind ? locBox.width
             : root.imgKind ? (model.kind === "video" ? vidBox.width : imgBox.width)
             : root.linkCardKind ? lp.width
             : root.docThumbKind || root.trackKind || root.codeKind || root.contactKind ? root.bubbleMax
             : (cardOnly || root.pollKind ? inner.implicitWidth : inner.implicitWidth + Style.space(22))
        width: Math.min(root.bubbleMax, Math.max(Style.space(40), contentW))
        height: inner.implicitHeight + (root.fullBleed || cardOnly || root.pollKind ? 0 : Style.space(20))
        anchors.right: root.own ? parent.right : undefined
        anchors.left: root.own ? undefined : parent.left
        anchors.leftMargin: 0
        radius: Style.space(16)
        antialiasing: true
        // Consecutive bubbles from one sender share small corners on that side; a lone bubble is round.
        readonly property real rSmall: Style.space(5)
        topLeftRadius: (!root.own && !root.groupStart) ? rSmall : radius
        bottomLeftRadius: (!root.own && !root.groupEnd) ? rSmall : radius
        topRightRadius: (root.own && !root.groupStart) ? rSmall : radius
        bottomRightRadius: (root.own && !root.groupEnd) ? rSmall : radius
        // Opaque fills (pre-blended over the card tone), identical everywhere.
        function mix(src, a) {
          // Blend over the popup surface tone, matching the message sheet.
          var bg = Color.popups.background
          return Qt.rgba(src.r * a + bg.r * (1 - a), src.g * a + bg.g * (1 - a), src.b * a + bg.b * (1 - a), 1)
        }
        readonly property color accentC: root.themeAccent !== "" ? Qt.color(root.themeAccent) : Color.accent
        function blend2(a, b, t) { return Qt.rgba(a.r * (1 - t) + b.r * t, a.g * (1 - t) + b.g * t, a.b * (1 - t) + b.b * t, 1) }
        // The tone a bubble would be; a poll wears it itself, so it is named.
        readonly property color bubbleFill:
               root.own ? mix(accentC, 0.42)
             : model.isHighlighted ? (root.themeAccent !== "" ? blend2(root.themeSurface, accentC, 0.22) : mix(accentC, 0.24))
             : (root.themeAccent !== "" ? root.themeSurface : mix(root.fg, 0.22))
        color: (cardOnly || root.pollKind || (model.kind === "video" && !root.textVisible))
             ? "transparent" : bubbleFill
        // Jump landing: a brief accent ring on the message you asked for.
        border.width: root.jumpFlash ? Math.max(1, Style.space(2)) : 0
        border.color: root.jumpFlash ? Util.alpha(root.accentC, 0.9) : "transparent"
        Behavior on border.width { NumberAnimation { duration: 220; easing.type: Easing.OutCubic } }

        Column {
          id: inner
          anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
          // A poll carries its own padding; inset here it overhangs the bubble.
          anchors.margins: (root.fullBleed || bubble.cardOnly || root.pollKind) ? 0 : Style.space(10)
          spacing: root.fullBleed ? 0 : Style.space(4)

          // Reply quote
          Rectangle {
            visible: model.replyTo !== null && model.replyTo !== undefined
            width: parent.width; height: visible ? q.implicitHeight + Style.space(10) : 0
            radius: Style.space(10)
            color: Util.alpha(Color.background, 0.35)
            Column {
              id: q
              anchors.left: parent.left; anchors.leftMargin: Style.space(10); anchors.right: parent.right; anchors.rightMargin: Style.space(8); anchors.verticalCenter: parent.verticalCenter
              Text { text: model.replyTo ? (model.replyTo.senderName || "") : ""; color: Util.alpha(root.fg, 0.75); font.family: Fonts.ui; font.pixelSize: Style.space(10); font.bold: true }
              Text { width: parent.width; text: model.replyTo ? (model.replyTo.kind === "image" ? "📷 Photo" : (model.replyTo.body || "")) : ""; color: Util.alpha(root.fg, 0.55); font.family: Fonts.ui; font.pixelSize: Style.space(10); elide: Text.ElideRight; maximumLineCount: 1 }
            }
          }

          // Image (GM style: full-bleed, rounded top; caption squares the bottom)
          Loader {
            id: imgBox
            // Lazily built: `visible: false` does not prevent construction.
            active: !root.recycling && (root.photoKind)
            visible: root.photoKind
            sourceComponent: Item {
              id: imgBoxInner
              readonly property real iw: (model.media && model.media.width) ? model.media.width : 400
              readonly property real ih: (model.media && model.media.height) ? model.media.height : 300
              readonly property real s: Math.min(1, root.bubbleMax / iw, Style.space(300) / ih)
              implicitWidth: Math.max(Style.space(120), Math.round(iw * s))
              implicitHeight: Math.max(Style.space(80), Math.round(ih * s))
              Rectangle {
                anchors.fill: parent
                topLeftRadius: bubble.topLeftRadius; topRightRadius: bubble.topRightRadius
                bottomLeftRadius: root.textVisible ? 0 : bubble.bottomLeftRadius
                bottomRightRadius: root.textVisible ? 0 : bubble.bottomRightRadius
                color: Util.alpha(root.fg, 0.08); visible: img.status !== Image.Ready
              }
              // Animated (GIF/APNG) plays inline; stills use Image.
              readonly property bool animated: {
                var m = model.media || {}
                var mime = (m.mime || "").toLowerCase()
                var fn = (m.filename || "").toLowerCase()
                return mime.indexOf("gif") >= 0 || fn.indexOf(".gif") >= 0
              }
              readonly property string mediaSrc: {
                if (!root.photoKind) return ""     // never hand audio/video files to the image decoder
                var m = model.media || {}
                // animations need the real file, not a static thumbnail
                var p = animated ? (m.path || m.thumbnailPath || "") : (m.thumbnailPath || m.path || "")
                return p !== "" ? "file://" + p : ""
              }
              onAnimatedChanged: if (animated && root.svc && !(model.media && model.media.path)) root.svc.fetchMedia(root.roomId, model.eventId, null, function(r, e) {})
              Image {
                id: img
                anchors.fill: parent
                source: parent.animated ? "" : parent.mediaSrc
                fillMode: Image.PreserveAspectCrop
                asynchronous: true; cache: true
                sourceSize.width: 600
                visible: false
              }
              AnimatedImage {
                id: gif
                anchors.fill: parent
                source: parent.animated ? parent.mediaSrc : ""
                fillMode: Image.PreserveAspectCrop
                cache: true
                playing: true
                visible: false
              }
              MultiEffect {
                anchors.fill: parent
                source: parent.animated ? gif : img
                maskEnabled: true; maskThresholdMin: 0.5; maskSpreadAtMin: 1.0
                maskSource: imgMask
                visible: parent.animated ? gif.status === AnimatedImage.Ready : img.status === Image.Ready
                autoPaddingEnabled: false
              }
              Rectangle {
                id: imgMask
                anchors.fill: parent
                topLeftRadius: bubble.topLeftRadius; topRightRadius: bubble.topRightRadius
                bottomLeftRadius: root.textVisible ? 0 : bubble.bottomLeftRadius
                bottomRightRadius: root.textVisible ? 0 : bubble.bottomRightRadius
                color: "black"; visible: false; layer.enabled: true; layer.smooth: true
              }
              Spinner { anchors.centerIn: parent; visible: !!(model.media && !model.media.thumbnailPath && !model.media.path); color: Util.alpha(root.fg, 0.5) }
              Rectangle {
                visible: parent.animated
                anchors.left: parent.left; anchors.bottom: parent.bottom; anchors.margins: Style.space(8)
                width: gt.implicitWidth + Style.space(10); height: Style.space(18); radius: Style.space(5)
                color: Util.alpha("#000000", 0.55)
                Text { id: gt; anchors.centerIn: parent; text: "GIF"; color: "#ffffff"; font.family: Fonts.ui; font.pixelSize: Style.space(9); font.bold: true }
              }
              MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.openImage(root.model, root.mediaRect(imgBox)) }
            }
          }

          // Video: poster frame with a play badge
          Loader {
            id: vidBox
            active: !root.recycling && (model.kind === "video")
            visible: model.kind === "video"
            sourceComponent: Item {
              id: vidBoxInner
              readonly property real vw: (model.media && model.media.width) ? model.media.width : 480
              readonly property real vh: (model.media && model.media.height) ? model.media.height : 320
              readonly property real vs: Math.min(1, root.bubbleMax / vw, Style.space(300) / vh)
              implicitWidth: Math.max(Style.space(160), Math.round(vw * vs))
              implicitHeight: Math.max(Style.space(110), Math.round(vh * vs))
              Rectangle {
                anchors.fill: parent
                topLeftRadius: bubble.topLeftRadius; topRightRadius: bubble.topRightRadius
                bottomLeftRadius: root.textVisible ? 0 : bubble.bottomLeftRadius
                bottomRightRadius: root.textVisible ? 0 : bubble.bottomRightRadius
                antialiasing: true
                color: Util.alpha(root.fg, 0.12)
              }
              // Masked via its own layer: an invisible MultiEffect source can come back blank after recycling.
              Image {
                id: poster
                anchors.fill: parent
                source: (model.media && model.media.thumbnailPath) ? "file://" + model.media.thumbnailPath : ""
                fillMode: Image.PreserveAspectCrop
                asynchronous: true
                cache: true
                layer.enabled: true
                layer.smooth: true
                layer.effect: MultiEffect { maskEnabled: true; maskThresholdMin: 0.5; maskSpreadAtMin: 1.0; maskSource: vidMask }
              }
              Rectangle {
                id: vidMask
                anchors.fill: parent
                topLeftRadius: bubble.topLeftRadius; topRightRadius: bubble.topRightRadius
                bottomLeftRadius: root.textVisible ? 0 : bubble.bottomLeftRadius
                bottomRightRadius: root.textVisible ? 0 : bubble.bottomRightRadius
                antialiasing: true
                color: "black"; visible: false; layer.enabled: true; layer.smooth: true
              }
              Rectangle {
                anchors.centerIn: parent
                width: Style.space(48); height: Style.space(48); radius: width / 2
                color: Util.alpha("#000000", 0.55)
                IconLabel { anchors.centerIn: parent; icon: Icons.play; color: "#ffffff"; size: Style.font.iconLarge }
              }
              MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.playVideo(root.model, root.mediaRect(vidBox)) }
            }
          }

          // Location (m.location) as a map card
          Loader {
            id: locBox
            active: !root.recycling && (root.locKind)
            visible: root.locKind
            // Async is safe ONLY here: this Loader's height is a fixed aspect of the
            // bubble width (8:5). Bodies sized from `implicitHeight` must stay sync.
            asynchronous: true
            width: active ? root.bubbleMax : 0
            height: active ? Math.round(root.bubbleMax * 0.62) : 0
            sourceComponent: LocationBody {
              // Only build the GL map when the bubble is near the viewport and the list is at
              // rest, or a run of location messages builds one MapLibre renderer per row.
              mapAllowed: root.locKind && root.ListView.view && !(root.page && root.page.scrolling)
                          ? (root.y + root.height > root.ListView.view.contentY - root.ListView.view.height * 0.5
                             && root.y < root.ListView.view.contentY + root.ListView.view.height * 1.5)
                          : false
              id: locBoxInner
              topLeftRadius: bubble.topLeftRadius
              topRightRadius: bubble.topRightRadius
              bottomLeftRadius: root.textVisible ? 0 : bubble.bottomLeftRadius
              bottomRightRadius: root.textVisible ? 0 : bubble.bottomRightRadius
              location: model.location || null
              markerAvatar: root.selfLocation ? (model.senderAvatarPath || "") : ""
              live: !!(root.liveShare && root.liveShare.live)
              ended: root.liveLocKind && !(root.liveShare && root.liveShare.live)
              expiresAt: root.liveShare ? (root.liveShare.expiresAt || 0) : 0
              svc: root.svc
              fg: root.fg
              accent: root.themeAccent !== "" ? Qt.color(root.themeAccent) : Color.accent
              surface: Util.alpha(root.fg, 0.10)
              frameC: bubble.color
              onOpenRequested: root.openLocation(root.model, root.mediaRect(locBox))
            }
          }

          // Poll (MSC3381)
          Loader {
            id: pollBox
            active: !root.recycling && (model.kind === "poll")
            visible: model.kind === "poll"
            width: active ? Math.min(root.bubbleMax, Style.space(258)) : 0
            height: (active && item) ? item.implicitHeight : 0
            sourceComponent: PollBody {
              poll: model.poll || null
              surface: bubble.bubbleFill
              fg: root.fg
              accent: root.themeAccent !== "" ? Qt.color(root.themeAccent) : Color.accent
              own: root.own
              onVoteRequested: function(ids) {
                if (root.svc && model.eventId) root.svc.pollVote(root.roomId, model.eventId, ids)
              }
            }
          }

          // Voice message: play/pause + waveform progress + duration
          Loader {
            id: voiceBox
            active: !root.recycling && (model.kind === "voice" || (model.kind === "audio" && model.media && model.media.waveform))
            visible: model.kind === "voice" || (model.kind === "audio" && model.media && model.media.waveform)
            sourceComponent: Item {
              id: voiceInner
              implicitWidth: Math.min(root.bubbleMax, Style.space(260))
              implicitHeight: Style.space(40)
              readonly property real dur: (model.media && model.media.duration) ? model.media.duration / 1000 : 0
              readonly property bool playing: root.playingVoice === model.eventId
              readonly property real pos: playing ? root.voicePos : 0
              readonly property real frac: dur > 0 ? Math.max(0, Math.min(1, pos / dur)) : 0
              Rectangle {
                id: vplay
                anchors.left: parent.left; anchors.verticalCenter: parent.verticalCenter
                width: Style.space(34); height: Style.space(34); radius: width / 2
                color: root.themeAccent !== "" ? Qt.lighter(Qt.color(root.themeAccent), 1.45) : "#f2f2f2"
                IconLabel { anchors.centerIn: parent; icon: voiceInner.playing ? Icons.pause : Icons.play; color: "#1a1a1a"; size: Style.font.icon }
                MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.toggleVoice(root.model) }
              }
              Item {
                id: vwave
                anchors.left: vplay.right; anchors.leftMargin: Style.space(8)
                anchors.right: vdur.left; anchors.rightMargin: Style.space(8)
                anchors.verticalCenter: parent.verticalCenter
                height: Style.space(26)
                readonly property int slots: Math.max(8, Math.floor(width / Style.space(5)))
                readonly property var bars: root.resampleWave(
                  (model.media && model.media.waveform && model.media.waveform.length > 0) ? model.media.waveform : root.flatWave,
                  slots)
                Row {
                  id: vrow
                  height: parent.height
                  spacing: Style.space(2)
                  Repeater {
                    model: vwave.bars
                    delegate: Item {
                      required property var modelData
                      required property int index
                      width: Style.space(3); height: vrow.height
                      Rectangle {
                        width: parent.width
                        height: Math.max(Style.space(3), Style.space(22) * Math.min(1, modelData))
                        y: Math.round((parent.height - height) / 2)
                        radius: width / 2
                        color: (index / Math.max(1, vwave.bars.length)) <= voiceInner.frac ? Util.alpha(root.fg, 0.95) : Util.alpha(root.fg, 0.35)
                      }
                    }
                  }
                }
                MouseArea {
                  anchors.fill: parent
                  cursorShape: Qt.PointingHandCursor
                  onClicked: function(m) { if (voiceInner.dur > 0) root.seekVoice(root.model, (m.x / width) * voiceInner.dur) }
                }
              }
              Text {
                id: vdur
                anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter
                text: {
                  var t = voiceInner.playing ? Math.max(0, voiceInner.dur - voiceInner.pos) : voiceInner.dur
                  var sec = Math.max(0, Math.round(t)), mn = Math.floor(sec / 60)
                  return (mn < 10 ? "0" : "") + mn + ":" + ((sec % 60) < 10 ? "0" : "") + (sec % 60)
                }
                color: Util.alpha(root.fg, 0.8); font.family: Fonts.ui; font.pixelSize: Style.font.caption
              }
            }
          }

          // Document preview: the first lines of the file, drawn as a page.
          Loader {
            id: docBox
            active: !root.recycling && (root.docThumbKind)
            visible: root.docThumbKind
            width: active ? root.bubbleMax : 0
            // As tall as it needs to be, up to a page.
            height: (active && item)
                    ? Math.min(Math.round(root.bubbleMax * 0.66),
                               Math.max(Style.space(70), item.contentHeight)) : 0
            sourceComponent: DocThumb {
              id: docBoxInner
              doc: root.docThumb
              fg: root.fg
              accent: root.themeAccent !== "" ? Qt.color(root.themeAccent) : Color.accent
              topLeftRadius: bubble.topLeftRadius
              topRightRadius: bubble.topRightRadius
              bottomLeftRadius: root.textVisible ? 0 : bubble.bottomLeftRadius
              bottomRightRadius: root.textVisible ? 0 : bubble.bottomRightRadius
              MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                onClicked: root.openDocument(root.model)
              }
            }
          }

          // A shared contact — from `com.sigil.contact` or from a `.vcf`.
          Loader {
            id: contactBox
            active: !root.recycling && (root.contactKind)
            visible: root.contactKind
            width: active ? root.bubbleMax : 0
            height: (active && item) ? item.implicitHeight : 0
            sourceComponent: ContactBody {
              cards: root.contactCards
              fg: root.fg
              accent: root.themeAccent !== "" ? Qt.color(root.themeAccent) : Color.accent
              topLeftRadius: bubble.topLeftRadius
              topRightRadius: bubble.topRightRadius
              bottomLeftRadius: root.textVisible ? 0 : bubble.bottomLeftRadius
              bottomRightRadius: root.textVisible ? 0 : bubble.bottomRightRadius
              onMessageRequested: function (uid) { root.openDmWith(uid) }
              onCopyRequested: function (v) {
                Quickshell.execDetached(["sh", "-c", 'printf "%s" "$1" | wl-copy', "copy", v])
              }
              onOpenRequested: function (u) { Qt.openUrlExternally(u) }
              onShareVcfRequested: function (uid, name) { root.shareVcf(uid, name) }
              svc: root.svc
              // Save writes the vCard to downloads and records the person; tapping again re-downloads.
              onSaveRequested: function (uid, name, saved) {
                if (!root.svc) return
                if (!saved) root.svc.saveContact(uid, name, null)
                root.svc.downloadContactVcf(uid, name, function (r, e) {
                  if (!root.page) return
                  if (r && r.path) root.page.note("Saved " + (r.filename || "contact") + " to Downloads")
                  else root.page.note("Could not save the vCard" + (e && e.message ? ": " + e.message : ""))
                })
              }
            }
          }

          // Music file: cover, play button, and a strip of the album's colour.
          Loader {
            id: trackBox
            active: !root.recycling && (root.trackKind)
            visible: root.trackKind
            width: active ? root.bubbleMax : 0
            height: (active && item) ? item.implicitHeight : 0
            sourceComponent: AudioBody {
              id: trackBoxInner
              info: root.trackInfo
              title: root.trackTitle
              durationLabel: {
                var bits = []
                if (model.media && model.media.durationLabel) bits.push(model.media.durationLabel)
                if (model.media && model.media.sizeLabel) bits.push(model.media.sizeLabel)
                return bits.join(" · ")
              }
              fg: root.fg
              accent: root.themeAccent !== "" ? Qt.color(root.themeAccent) : Color.accent
              topLeftRadius: bubble.topLeftRadius
              topRightRadius: bubble.topRightRadius
              bottomLeftRadius: root.textVisible ? 0 : bubble.bottomLeftRadius
              bottomRightRadius: root.textVisible ? 0 : bubble.bottomRightRadius
              onOpenRequested: root.openAudio(root.model)
            }
          }

          // Under a preview it is the caption strip, so it carries its own padding.
          Loader {
            id: fileRowBox
            // The condition lives out here: a Loader cannot read an id declared in its own sourceComponent.
            readonly property bool wanted: (model.kind === "file" && !root.docThumbKind && !root.contactKind)
                                           || (model.kind === "audio" && !voiceBox.visible && !root.trackKind)
            active: !root.recycling && (fileRowBox.wanted)
            visible: fileRowBox.wanted
            sourceComponent: Item {
              id: fileRowBoxInner
              // Full width only once a preview has settled the bubble width, or `inner` and this child loop.
              implicitWidth: root.fullBleed ? inner.width : fileRow.width
              implicitHeight: fileRow.height + (root.fullBleed ? Style.space(18) : 0)
              Row {
                id: fileRow
                // Top-anchored: the box takes its height from this row, so centring is a loop.
                anchors.left: parent.left
                anchors.top: parent.top
                anchors.leftMargin: root.fullBleed ? Style.space(10) : 0
                anchors.topMargin: root.fullBleed ? Style.space(9) : 0
                // A file we could preview *is* its preview.
                visible: fileRowBox.wanted
                spacing: Style.space(8)
              // A TapHandler, not a MouseArea: children of a Row may not use anchors.
              readonly property bool readable: model.kind === "file" && !!(model.media && model.media.previewable)
              TapHandler {
                enabled: parent.readable
                acceptedButtons: Qt.LeftButton
                onSingleTapped: root.openDocument(root.model)
              }
              Rectangle { width: Style.space(34); height: Style.space(34); radius: height / 2; color: Util.alpha(Color.accent, 0.25)
                IconLabel { anchors.centerIn: parent; icon: model.kind === "video" ? Icons.videoOn : (model.kind === "file" ? Icons.file : Icons.music); color: root.fg; size: Style.font.icon } }
              Column {
                anchors.verticalCenter: parent.verticalCenter
                Text { text: model.media ? (model.media.filename || model.body) : model.body; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall; elide: Text.ElideMiddle; width: Math.min(root.bubbleMax - Style.space(80), implicitWidth) }
                // A track's length is what you want before opening it, not bytes.
                Text {
                  text: {
                    var bits = []
                    if (model.media && model.media.durationLabel) bits.push(model.media.durationLabel)
                    if (model.media && model.media.sizeLabel) bits.push(model.media.sizeLabel)
                    return bits.join(" · ")
                  }
                  color: Util.alpha(root.fg, 0.5); font.family: Fonts.ui; font.pixelSize: Style.font.caption
                }
              }
              }
            }
          }

          // Link preview card (homeserver OG data): image, then a tinted info panel.
          Loader {
            id: lp
            readonly property string url: {
              if (!root.isMsg || root.imgKind) return ""
              var b = model.body || ""
              var m = b.match(/https?:\/\/[^\s<>"]+/)
              return m ? m[0] : ""
            }
            readonly property string domain: {
              var m = url.match(/^https?:\/\/([^\/]+)/)
              return m ? m[1] : ""
            }
            readonly property var data: (url !== "" && root.svc) ? root.svc.linkPreviews[url] : undefined
            active: !root.recycling && (url !== "" && data !== undefined && data !== null && data !== false)
            width: root.bubbleMax
            onUrlChanged: if (url !== "" && root.svc) root.svc.linkPreview(url)
            Component.onCompleted: if (url !== "" && root.svc) root.svc.linkPreview(url)
            sourceComponent: Item {
              id: cardRoot
              width: lp.width
              height: card.height
              readonly property color footerC: (lp.data && lp.data.accent) ? Qt.color(lp.data.accent) : Util.alpha("#000000", 0.45)
              Rectangle {
                id: card
                width: parent.width
                height: (lpImgBox.visible ? lpImgBox.height : 0) + info.height
                // A card filling a bubble takes the bubble's shape, squaring under a caption.
                radius: Style.space(14)
                topLeftRadius: root.linkCardKind ? bubble.topLeftRadius : radius
                topRightRadius: root.linkCardKind ? bubble.topRightRadius : radius
                bottomLeftRadius: root.linkCardKind ? (root.textVisible ? 0 : bubble.bottomLeftRadius) : radius
                bottomRightRadius: root.linkCardKind ? (root.textVisible ? 0 : bubble.bottomRightRadius) : radius
                antialiasing: true
                clip: true
                color: parent.footerC
                // Masked to the card's rounded top: Rectangle.clip is rectangular.
                Item {
                  id: lpImgBox
                  anchors.top: parent.top; anchors.left: parent.left; anchors.right: parent.right
                  readonly property real ar: (lp.data && lp.data.imageWidth > 0 && lp.data.imageHeight > 0) ? (lp.data.imageHeight / lp.data.imageWidth) : 0.52
                  height: visible ? Math.min(Style.space(340), Math.round(lp.width * ar)) : 0
                  visible: !!(lp.data && lp.data.imagePath)
                  Image {
                    id: lpImg
                    anchors.fill: parent
                    source: (lp.data && lp.data.imagePath) ? "file://" + lp.data.imagePath : ""
                    fillMode: Image.PreserveAspectCrop
                    asynchronous: true
                    visible: false
                  }
                  MultiEffect { anchors.fill: parent; source: lpImg; maskEnabled: true; maskThresholdMin: 0.5; maskSpreadAtMin: 1.0; maskSource: lpMask; visible: lpImg.status === Image.Ready }
                  Rectangle {
                    id: lpMask
                    anchors.fill: parent
                    topLeftRadius: Style.space(14); topRightRadius: Style.space(14)
                    antialiasing: true
                    color: "black"; visible: false; layer.enabled: true; layer.smooth: true
                  }
                }
                Rectangle {
                  visible: lpImgBox.visible && !!(lp.data && lp.data.isVideo)
                  anchors.horizontalCenter: lpImgBox.horizontalCenter
                  anchors.verticalCenter: lpImgBox.verticalCenter
                  width: Style.space(52); height: Style.space(52); radius: width / 2
                  color: Util.alpha("#000000", 0.5)
                  IconLabel { anchors.centerIn: parent; icon: Icons.play; color: "#ffffff"; size: Style.space(24) }
                }
                Rectangle {
                  id: info
                  anchors.top: lpImgBox.visible ? lpImgBox.bottom : parent.top
                  anchors.left: parent.left; anchors.right: parent.right
                  height: infoCol.implicitHeight + Style.space(20)
                  color: "transparent"
                  Column {
                    id: infoCol
                    anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
                    anchors.margins: Style.space(10)
                    spacing: Style.space(4)
                    Text {
                      visible: text !== ""
                      width: parent.width; elide: Text.ElideRight; maximumLineCount: 2; wrapMode: Text.Wrap
                      text: (lp.data && lp.data.title) ? lp.data.title : ""
                      color: "#ffffff"; font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: true
                    }
                    Text {
                      visible: text !== ""
                      width: parent.width; elide: Text.ElideRight; maximumLineCount: 2; wrapMode: Text.Wrap
                      text: (lp.data && lp.data.description) ? lp.data.description : ""
                      color: Util.alpha("#ffffff", 0.8); font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
                    }
                    Row {
                      spacing: Style.space(6)
                      topPadding: Style.space(2)
                      Rectangle {
                        width: Style.space(16); height: Style.space(16); radius: Style.space(4)
                        anchors.verticalCenter: parent.verticalCenter
                        color: Util.alpha("#ffffff", 0.9)
                        Text { anchors.centerIn: parent; text: lp.domain.replace(/^www\./, "").substring(0, 1).toUpperCase(); color: cardRoot.footerC; font.family: Fonts.ui; font.pixelSize: Style.space(9); font.bold: true }
                      }
                      Text {
                        anchors.verticalCenter: parent.verticalCenter
                        text: lp.domain
                        color: Util.alpha("#ffffff", 0.85); font.family: Fonts.ui; font.pixelSize: Style.font.caption
                      }
                    }
                  }
                }
                MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: Qt.openUrlExternally(lp.url) }
              }
            }
          }

          // Text/code/text: text runs are captions, code runs fill the bubble.
          Repeater {
            model: root.codeKind ? root.parts : []
            delegate: Loader {
              required property var modelData
              required property int index
              width: root.bubbleMax
              sourceComponent: modelData.t === "code" ? codePart : textPart
              onLoaded: { item.part = modelData; item.idx = index }
            }
          }

          Component {
            id: textPart
            TextEdit {
              property var part: null
              property int idx: 0
              width: root.bubbleMax
              leftPadding: Style.space(10); rightPadding: Style.space(10)
              topPadding: Style.space(8); bottomPadding: Style.space(9)
              readOnly: true; selectByMouse: false
              textFormat: TextEdit.RichText
              text: part ? root.themedHtml(part.html) : ""
              wrapMode: TextEdit.Wrap
              color: root.fg
              font.family: Fonts.ui; font.pixelSize: Style.font.body
              onLinkActivated: function (link) { Qt.openUrlExternally(link) }
            }
          }

          Component {
            id: codePart
            CodeBlock {
              property var part: null
              property int idx: 0
              width: root.bubbleMax
              html: part ? (part.html || "") : ""
              lang: part ? (part.lang || "") : ""
              fg: root.fg
              // Round only where the block actually meets the bubble's edge.
              topLeftRadius: idx === 0 ? bubble.topLeftRadius : 0
              topRightRadius: idx === 0 ? bubble.topRightRadius : 0
              bottomLeftRadius: idx === root.parts.length - 1 ? bubble.bottomLeftRadius : 0
              bottomRightRadius: idx === root.parts.length - 1 ? bubble.bottomRightRadius : 0
            }
          }

          // Its own item: an animation needs each glyph movable; a Text cannot.
          RichText {
            id: richBox
            visible: root.richKind
            maxWidth: root.bubbleMax - Style.space(22)
            width: visible ? implicitWidth : 0
            height: visible ? implicitHeight : 0
            // Empty unless the message has effects: `visible: false` does not stop a Repeater instantiating.
            text: root.richKind ? (model.body || "") : ""
            effects: root.effects
            eventId: model.eventId || ""
            fg: root.fg
            svc: root.svc
            pixelSize: Style.font.body
            // Paused off-screen: per-character animations burn a core. `richKind` first — this
            // is the delegate's only contentY-dependent binding, re-evaluated every frame.
            active: !root.recycling && (root.richKind && root.ListView.view)
                    ? (root.y + root.height > root.ListView.view.contentY
                       && root.y < root.ListView.view.contentY + root.ListView.view.height)
                    : true
          }

          // Text (or image caption)
          TextEdit {
            visible: root.textVisible
            width: root.fullBleed ? bubble.width : Math.min(root.bubbleMax - Style.space(22), implicitWidth)
            leftPadding: root.fullBleed ? Style.space(10) : 0
            rightPadding: root.fullBleed ? Style.space(10) : 0
            topPadding: root.fullBleed ? Style.space(8) : 0
            bottomPadding: root.fullBleed ? Style.space(10) : 0
            id: bodyText
            readOnly: true; selectByMouse: false
            // linkify() emits HTML, so this must be rich text, not PlainText.
            textFormat: TextEdit.RichText
            // The engine sanitises remote markup and pre-linkifies plain bodies.
            text: model.kind === "utd" ? "🔒 Waiting for keys…"
                : model.kind === "redacted" ? "Message deleted"
                : root.linkCardKind ? root.themedHtml(root.captionHtml)
                : root.themedHtml(model.html || model.body)
            wrapMode: TextEdit.Wrap
            color: model.kind === "utd" || model.kind === "redacted" ? Util.alpha(root.fg, 0.55) : root.fg
            font.family: Fonts.ui; font.pixelSize: Style.font.body
            font.italic: model.kind === "emote" || model.kind === "redacted"
            selectionColor: Util.alpha(Color.accent, 0.4); selectedTextColor: root.fg
            onLinkActivated: function(link) { Qt.openUrlExternally(link) }
            HoverHandler { cursorShape: parent.hoveredLink ? Qt.PointingHandCursor : Qt.ArrowCursor }
            TapHandler {
              acceptedButtons: Qt.LeftButton
              onSingleTapped: {
                if (bodyText.hoveredLink !== "") Qt.openUrlExternally(bodyText.hoveredLink)
                else root.toggleDetails()
              }
            }
          }

          // Thread summary, styled as a reply quote: an inset strip in the bubble.
          Rectangle {
            id: threadChip
            visible: !!root.threadSummary
            // Full width: a 150px content-hugging floor bloated one-word threads.
            width: parent.width
            height: visible ? Style.space(34) : 0
            radius: Style.space(10)
            antialiasing: true
            // fg alphas only, never accent: `bubbleFill` is accent blended over the ground.
            color: Util.alpha(Color.background, 0.35)
            Row {
              id: tcRow
              anchors.left: parent.left; anchors.leftMargin: Style.space(9)
              anchors.right: tcChevron.left; anchors.rightMargin: Style.space(6)
              anchors.verticalCenter: parent.verticalCenter
              spacing: Style.space(6)
              IconLabel { anchors.verticalCenter: parent.verticalCenter
                icon: Icons.thread
                color: Util.alpha(root.fg, 0.75); size: Style.font.caption }
              Text {
                id: tcCount
                anchors.verticalCenter: parent.verticalCenter
                text: {
                  var n = root.threadSummary ? (root.threadSummary.count || 0) : 0
                  return n === 1 ? "1 reply" : n + " replies"
                }
                color: Util.alpha(root.fg, 0.75)
                font.family: Fonts.ui; font.pixelSize: Style.font.caption; font.bold: true
              }
              Text {
                anchors.verticalCenter: parent.verticalCenter
                visible: text !== ""
                // One line: a reply with a newline overflows the fixed-height strip.
                width: Math.max(0, Math.min(implicitWidth, tcRow.width - tcCount.width - Style.space(30)))
                elide: Text.ElideRight
                maximumLineCount: 1
                wrapMode: Text.NoWrap
                text: root.threadSummary ? (root.threadSummary.latestBody || "") : ""
                color: Util.alpha(root.fg, 0.55)
                font.family: Fonts.ui; font.pixelSize: Style.font.caption
              }
            }
            IconLabel { id: tcChevron
              anchors.right: parent.right; anchors.rightMargin: Style.space(9)
              anchors.verticalCenter: parent.verticalCenter
              icon: Icons.chevronRight
              color: Util.alpha(root.fg, 0.5); size: Style.font.caption }
            MouseArea {
              anchors.fill: parent
              cursorShape: Qt.PointingHandCursor
              onClicked: root.openThreadRequested(model.eventId || "")
            }
          }

        }

        TapHandler { acceptedButtons: Qt.RightButton; onTapped: function(ev) { var p = bubble.mapToItem(null, 0, 0); root.menuRequested(root.model, p.x, p.y, bubble.width, bubble.height, bubble) } }
        TapHandler {
          acceptedButtons: Qt.LeftButton; longPressThreshold: 0.5
          onSingleTapped: root.toggleDetails()
          onLongPressed: { var p = bubble.mapToItem(null, 0, 0); root.menuRequested(root.model, p.x, p.y, bubble.width, bubble.height, bubble) }
        }
      }

      // Pinned marker, tilted away from the sender's side to clear the group corner.
      Item {
        id: pinMark
        // Visibility follows the fade, not the trigger, or neither animation plays.
        visible: root.isMsg && opacity > 0.01
        z: 6
        width: Style.space(20); height: Style.space(20)
        anchors.top: bubble.top
        // Barely proud of the corner, not outside it: the small group radius is on this side.
        anchors.topMargin: -Style.space(4)
        anchors.left: root.own ? undefined : bubble.left
        anchors.right: root.own ? bubble.right : undefined
        anchors.leftMargin: Style.space(2)
        anchors.rightMargin: Style.space(2)
        opacity: root.pinned ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: 160 } }
        scale: root.pinned ? 1 : 0.6
        Behavior on scale { NumberAnimation { duration: 220; easing.type: Easing.OutBack; easing.overshoot: 2.2 } }
        // A disc behind it so the glyph reads over a photo or a map.
        Rectangle {
          anchors.fill: parent
          radius: width / 2
          antialiasing: true
          color: root.themeAccent !== "" ? Qt.color(root.themeAccent) : Color.accent
          // A full-accent disc on a 42%-accent bubble has no edge against it.
          border.width: Math.max(1, Style.space(1.5))
          border.color: Color.popups.background
        }
        IconLabel {
          anchors.centerIn: parent
          icon: Icons.pin
          // `Color.background` tracks a light theme; a hardcoded near-black does not.
          color: Color.background
          size: Style.font.caption
          rotation: root.own ? 35 : -35
          // Rotated, so it keeps the distance-field renderer (native bakes one orientation).
          renderMode: Text.QtRendering
        }
      }

    }

  // Mark stack as a component so the row can place it: ring, bang, tick, faces.
  component MarkStack: Item {
    Rectangle {
      visible: opacity > 0.01
      opacity: root.sendingNow ? 1 : 0
      anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter
      width: root.markDot; height: width
      color: "transparent"
      Behavior on opacity { NumberAnimation { duration: 160 } }
      IconLabel { anchors.centerIn: parent; size: root.markDot
                  icon: Icons.statusDot; color: Util.alpha(root.fg, 0.55) }
    }
    Rectangle {
      visible: opacity > 0.01
      opacity: root.failedNow ? 1 : 0
      anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter
      width: root.markDot; height: width
      color: "transparent"
      Behavior on opacity { NumberAnimation { duration: 160 } }
      IconLabel { anchors.centerIn: parent; size: root.markDot; filled: true
                  icon: Icons.errorMark; color: Color.urgent }
    }
    Rectangle {
      visible: opacity > 0.01
      opacity: (!root.sendingNow && !root.failedNow && root.readers.length === 0) ? 1 : 0
      anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter
      width: root.markDot; height: width
      color: "transparent"
      scale: opacity < 0.5 ? 0.7 : 1
      Behavior on opacity { NumberAnimation { duration: 200 } }
      Behavior on scale { NumberAnimation { duration: 220; easing.type: Easing.OutCubic } }
      // Scales on arrival, so it keeps the distance-field renderer.
      IconLabel { anchors.centerIn: parent; size: root.markDot; filled: true
                  renderMode: Text.QtRendering
                  icon: Icons.checkCircle
                  color: root.themeAccent !== "" ? Qt.color(root.themeAccent) : Color.accent }
    }
    Repeater {
      model: root.readers
      delegate: Item {
        required property var modelData
        required property int index
        width: root.markDot; height: root.markDot
        anchors.verticalCenter: parent.verticalCenter
        x: parent.width - root.markDot - index * (root.markDot * 0.68)
        z: index
        // Stacked faces overlap, so each needs something solid behind it.
        Rectangle {
          anchors.centerIn: parent
          width: root.markDot + Style.space(2); height: width; radius: width / 2
          antialiasing: true
          color: root.receiptGround
          y: face.y
          opacity: face.opacity
        }
        Avatar {
          id: face
          anchors.fill: parent
          size: root.markDot
          source: modelData.avatarPath || ""
          name: modelData.displayName || ""
          userId: modelData.userId || ""
          // Static unless this reader is new: `animateMarks` alone replayed every rebroadcast.
          readonly property bool fresh: root.animateMarks && !!modelData.fresh
          property real drop: fresh ? -Style.space(14) : 0
          property real vel: 0
          y: drop
          opacity: fresh ? 0 : 1
          Component.onCompleted: if (fresh) { dropPhysics.running = true; fadeIn.start() }
          FrameAnimation {
            id: dropPhysics
            running: false
            onTriggered: {
              var dt = Math.min(0.033, Math.max(0.001, frameTime))
              face.vel += (-face.drop * 260 - face.vel * 16) * dt
              face.drop += face.vel * dt
              if (Math.abs(face.drop) < 0.3 && Math.abs(face.vel) < 4) { face.drop = 0; face.vel = 0; running = false }
            }
          }
          NumberAnimation { id: fadeIn; target: face; property: "opacity"; from: 0; to: 1; duration: 180 }
        }
      }
    }
  }

    // Receipt · timestamp · lock on one line; the mark leads, tapping grows the rest.
    Item {
      id: detailRow
      // Only messages carry details; state rows would sprout orphan timestamps.
      readonly property bool wanted: root.isMsg && (root.showMark || root.detailsOn)
      // Visibility follows the fade, not the trigger, or the row vanishes on close.
      visible: opacity > 0.01
      opacity: wanted ? 1 : 0
      Behavior on opacity { NumberAnimation { duration: 220; easing.type: Easing.OutCubic } }
      width: parent.width
      height: wanted ? root.markDot + Style.space(4) : 0
      Behavior on height { NumberAnimation { duration: 180; easing.type: Easing.OutCubic } }

      Row {
        anchors.right: root.own ? parent.right : undefined
        anchors.rightMargin: Style.space(2)
        anchors.left: root.own ? undefined : parent.left
        // Bubbles sit flush left in group rooms, so the detail row must not inset.
        anchors.leftMargin: root.own ? 0 : Style.space(2)
        anchors.verticalCenter: parent.verticalCenter
        spacing: Style.space(5)
        // Left to right: the row is right-anchored, so details push the mark left.
        layoutDirection: Qt.LeftToRight

        Item {
          visible: root.own
          anchors.verticalCenter: parent.verticalCenter
          width: root.own ? root.markDot + Math.max(0, root.readers.length - 1) * (root.markDot * 0.68) : 0
          height: root.markDot
          Behavior on width { NumberAnimation { duration: 200; easing.type: Easing.OutCubic } }
          MarkStack { anchors.fill: parent }
        }

        Text {
          visible: root.own && opacity > 0.01
          opacity: root.detailsOn ? 1 : 0
          anchors.verticalCenter: parent.verticalCenter
          text: "·"
          color: Util.alpha(root.fg, 0.5)
          font.family: Fonts.ui; font.pixelSize: Style.space(10)
          Behavior on opacity { NumberAnimation { duration: 160 } }
        }

        Item {
          anchors.verticalCenter: parent.verticalCenter
          visible: width > 0.5
          width: root.detailsOn ? detailText.implicitWidth : 0
          height: detailText.implicitHeight
          clip: true
          opacity: root.detailsOn ? 1 : 0
          Behavior on width { NumberAnimation { duration: 220; easing.type: Easing.OutCubic } }
          Behavior on opacity { NumberAnimation { duration: 160 } }
          Row {
            id: detailText
            anchors.right: root.own ? parent.right : undefined
            anchors.left: root.own ? undefined : parent.left
            anchors.verticalCenter: parent.verticalCenter
            spacing: Style.space(4)
            Text {
              anchors.verticalCenter: parent.verticalCenter
              text: root.detailsText
              color: model.sendState === "failed" ? Color.urgent : Util.alpha(root.fg, 0.5)
              font.family: Fonts.ui; font.pixelSize: Style.space(10)
            }
            IconLabel { anchors.verticalCenter: parent.verticalCenter
              icon: root.detailsLock
              color: model.sendState === "failed" ? Color.urgent : Util.alpha(root.fg, 0.5); size: Style.space(10) }
          }
        }
      }
    }

  }

  readonly property string detailsText: {
    var parts = []
    parts.push(Qt.formatTime(new Date(model.ts), "h:mm AP"))
    if (model.isEdited) parts.push("Edited")
    return parts.join(" · ")
  }
  /// Always shown, struck through when unencrypted; separate because it needs the icon font.
  readonly property string detailsLock: root.encrypted ? Icons.lock : Icons.lockOff

  // With a link preview showing, a body that is nothing but the URL is redundant.
  readonly property bool linkOnlyBody: {
    if (!root.isMsg || root.imgKind) return false
    var b = (model.body || "").trim()
    return /^https?:\/\/\S+$/.test(b)
  }

  /// A link *plus* words: the card is laid out as a photo, the rest as a caption.
  readonly property bool linkCardKind: lp.active && !root.imgKind && !root.linkOnlyBody

  /// Fenced block: the engine splits text/code/text so code can fill the bubble.
  readonly property var parts: {
    var j = model.partsJson || ""
    if (j === "") return []
    try { return JSON.parse(j) } catch (e) { return [] }
  }
  readonly property bool codeKind: root.parts.length > 0

  /// Sigil's own styling, drawn character by character; only for messages that have it.
  readonly property var effects: {
    var j = model.effectsJson || ""
    if (j === "") return []
    try { return JSON.parse(j) } catch (e) { return [] }
  }
  readonly property bool richKind: root.effects.length > 0 && !root.codeKind && root.isMsg

  /// A shared Matrix contact or a parsed `.vcf`; same card, different actions.
  readonly property var contact: {
    var j = model.contactJson || ""
    if (j === "") return null
    try { return JSON.parse(j) } catch (e) { return null }
  }
  readonly property bool vcardFile: model.kind === "file" && !!(model.media && model.media.vcard)
  readonly property var vcardData: {
    if (!root.vcardFile || !root.svc || !model.eventId) return null
    var v = root.svc.vcards[root.svc.docThumbKey(root.roomId, model.eventId)]
    return (v && v !== true) ? v : null
  }
  readonly property var contactCards: {
    if (root.contact) {
      return [{
        name: root.contact.display_name || "",
        userId: root.contact.user_id || "",
        // The contact's own picture, resolved by the engine, not the sender's.
        avatarPath: root.contact.avatarPath || "",
        phones: [], emails: []
      }]
    }
    if (root.vcardData) {
      var out = []
      for (var i = 0; i < root.vcardData.cards.length; i++) {
        var c = Object.assign({}, root.vcardData.cards[i])
        c.photoPath = (root.vcardData.photos && root.vcardData.photos[i]) || ""
        out.push(c)
      }
      return out
    }
    return []
  }
  readonly property bool contactKind: root.contactCards.length > 0
  function requestVcard() {
    if (!root.vcardFile || !root.svc || !model.eventId) return
    root.svc.readVcard(root.roomId, model.eventId)
  }
  readonly property string vcardEventId: root.vcardFile && model.eventId ? model.eventId : ""
  onVcardEventIdChanged: root.requestVcard()

  /// A readable document: the preview fills the bubble, the file row is its caption.
  readonly property var docThumb: {
    if (model.kind !== "file" || !root.svc || !model.eventId) return null
    if (!(model.media && model.media.previewable)) return null
    var d = root.svc.docThumbs[root.svc.docThumbKey(root.roomId, model.eventId)]
    return (d && d !== true) ? d : null
  }
  readonly property bool docThumbKind: !!root.docThumb

  /// A music file, not a voice note (which arrives with its own MSC3245 waveform).
  readonly property bool trackKind: model.kind === "audio" && !(model.media && model.media.waveform)
  readonly property var trackInfo: {
    if (!root.trackKind || !root.svc || !model.eventId) return null
    var t = root.svc.audioInfos[root.svc.docThumbKey(root.roomId, model.eventId)]
    return (t && t !== true) ? t : null
  }
  /// The filename without its extension: nobody wants ".mp3" in a title.
  readonly property string trackTitle: {
    var n = (model.media && model.media.filename) ? model.media.filename : (model.body || "Audio")
    var dot = n.lastIndexOf(".")
    return dot > 0 ? n.substring(0, dot) : n
  }
  function requestTrackInfo() {
    if (!root.trackKind || !root.svc || !model.eventId) return
    root.svc.audioInfo(root.roomId, model.eventId, (model.media && model.media.size) || 0)
  }
  readonly property string trackEventId: root.trackKind && model.eventId ? model.eventId : ""
  onTrackEventIdChanged: root.requestTrackInfo()

  /// Ask once, on the event changing: a recycled delegate never completes again.
  function requestDocThumb() {
    if (model.kind !== "file" || !root.svc || !model.eventId) return
    if (!(model.media && model.media.previewable)) return
    root.svc.docThumb(root.roomId, model.eventId, (model.media && model.media.size) || 0)
  }
  readonly property string docEventId: (model.kind === "file" && model.eventId) ? model.eventId : ""
  onDocEventIdChanged: root.requestDocThumb()

  /// The message with the previewed link removed; the anchor goes, its words stay.
  function stripLink(html, url) {
    if (!html || !url) return html || ""
    var out = html
    // The href is HTML and the url came from the plain body, so `&` is spelled differently — try both.
    var forms = [url, url.replace(/&/g, "&amp;")]
    for (var i = 0; i < forms.length; i++) {
      var esc = forms[i].replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
      var next = out.replace(new RegExp('<a\\b[^>]*href="' + esc + '"[^>]*>[\\s\\S]*?<\\/a>', "i"), "")
      if (next !== out) { out = next; break }
      // No anchor to remove (an unlinkified body): take the bare text out.
      if (out.indexOf(forms[i]) >= 0) { out = out.split(forms[i]).join(" "); break }
    }
    // Whatever separated the link from the words is now a dangling break.
    return out.replace(/^(?:\s|<br\s*\/?>)+/i, "").replace(/(?:\s|<br\s*\/?>)+$/i, "")
  }
  readonly property string captionHtml: {
    if (!root.linkCardKind) return ""
    var h = model.html || ""
    if (h !== "") return root.stripLink(h, lp.url)
    return (model.body || "").split(lp.url).join(" ").replace(/\s+/g, " ").trim()
  }
  /// Markup removed, for the "is there anything left to show?" test.
  readonly property string captionPlain: root.captionHtml.replace(/<[^>]*>/g, "").replace(/&nbsp;/g, " ").trim()

  readonly property bool textVisible: {
    if (!root.isMsg) return false
    if (model.kind === "image" || model.kind === "sticker") return !!(model.media && model.media.filename && model.body && model.body !== model.media.filename)
    if (model.kind === "video") return !!(model.media && model.media.filename && model.body && model.body !== model.media.filename)
    if (model.kind === "voice") return !!(model.body && model.media && model.body !== model.media.filename)
    // Files and audio caption like pictures; an uncaptioned body is just the filename.
    if (model.kind === "file" || model.kind === "audio")
      return !!(model.media && model.media.filename && model.body && model.body !== model.media.filename)
    // A poll ships a plain-text fallback for clients that cannot render it.
    if (model.kind === "poll") return false
    // No caption under a map: the body restates the geo URI.
    if (root.locKind) return false
    // The parts Repeater already drew it.
    if (root.codeKind) return false
    // So did the styled-text renderer.
    if (root.richKind) return false
    // The card says everything `body` says, and better.
    if (root.contactKind) return false
    if (root.linkOnlyBody && lp.active) return false
    // Nothing but the link and some whitespace: the card already said it.
    if (root.linkCardKind) return root.captionPlain !== ""
    return true
  }
}
