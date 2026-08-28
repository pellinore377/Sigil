import QtQuick
import QtQuick.Effects
import Quickshell
import qs.Commons
import qs.Ui
import ".."
import "../calls"
import "."

// Media viewer: top bar, a swipeable pager over every image in the room, and quick reactions.
Rectangle {
  id: root
  property var svc: null
  property var item: null
  property string roomId: ""
  property color fg: Color.menu.text
  property real scrimRadius: 0
  property color accent: Color.accent      // follows the chat theme
  // Tips go through the panel's in-card layer: a QQC ToolTip gets its own window in Qt 6.9.
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

  // Opening grows out of the tapped thumbnail; closing fades, so `item` outlives the fade.
  property bool shown: false
  property bool instant: false
  property bool morphing: false
  property real mOx: 0
  property real mOy: 0
  property real mScale: 1
  visible: item !== null && opacity > 0.01
  opacity: root.shown ? 1 : 0
  Behavior on opacity { NumberAnimation { duration: 190; easing.type: Easing.OutCubic } }
  transform: Scale {
    origin.x: root.mOx
    origin.y: root.mOy
    xScale: root.morphing ? root.mScale : 1
    yScale: root.morphing ? root.mScale : 1
    Behavior on xScale { enabled: !root.instant; NumberAnimation { duration: 270; easing.type: Easing.OutCubic } }
    Behavior on yScale { enabled: !root.instant; NumberAnimation { duration: 270; easing.type: Easing.OutCubic } }
  }
  Timer { id: viewerGrow; interval: 16; onTriggered: { root.instant = false; root.morphing = false } }
  Timer {
    id: viewerFade
    interval: 200
    onTriggered: { root.stopPlayback(); root.item = null; root.images = []; root.cur = -1 }
  }
  // Own the keyboard while open, or Escape travels past and closes the chat behind.
  focus: root.item !== null
  onItemChanged: if (root.item) Qt.callLater(root.forceActiveFocus)
  Keys.onEscapePressed: function(e) {
    if (moreMenu.open) { moreMenu.open = false }
    else if (fwdMenu.open) { fwdMenu.open = false }
    else { root.close() }
    e.accepted = true
  }
  // Solid base under the frost: the mask's antialiased edge shrinks the shape by ~1px.
  color: Qt.rgba(0.05, 0.05, 0.06, 1.0)
  // Match the card's radius exactly so the corners sit flush with the panel.
  radius: scrimRadius
  antialiasing: true
  clip: true
  // Backdrop: an opaque page snapshot, blurred and masked to the card shape. Rounding
  // the mask, not the viewer rect, is what keeps the corners flush with the card.
  property Item pagesItem: null
  Item {
    anchors.fill: parent
    visible: root.item !== null
    layer.enabled: true
    layer.smooth: true
    layer.effect: MultiEffect {
      blurEnabled: true; blur: 0.6; blurMax: 48; autoPaddingEnabled: false
      maskEnabled: true; maskThresholdMin: 0.5; maskSpreadAtMin: 1.0
      maskSource: viewerMask
    }
    Rectangle { anchors.fill: parent; color: Qt.rgba(Color.menu.background.r, Color.menu.background.g, Color.menu.background.b, 1) }
    ShaderEffectSource { anchors.fill: parent; sourceItem: root.pagesItem; live: true; visible: root.pagesItem !== null }
    Rectangle { anchors.fill: parent; color: Util.alpha("#000000", 0.82) }
  }
  Item {
    id: viewerMask
    anchors.fill: parent
    layer.enabled: true
    layer.smooth: true
    visible: false
    Rectangle { anchors.fill: parent; radius: root.scrimRadius; antialiasing: true; color: "black" }
  }

  property var images: []
  property int cur: -1
  readonly property var curItem: (cur >= 0 && cur < images.length) ? images[cur] : null
  property var fullPaths: ({})
  property string toast: ""
  Timer { id: toastTimer; interval: 2500; onTriggered: root.toast = "" }
  function note(t) { root.toast = t; toastTimer.restart() }

  function show(it, from) {
    viewerFade.stop()
    var list = []
    var t = (root.svc && root.roomId) ? root.svc.timelineFor(root.roomId) : null
    if (t) {
      var m = t.model
      for (var i = m.count - 1; i >= 0; i--) {
        var x = m.get(i)
        if ((x.kind === "image" || x.kind === "video") && x.media)
          list.push({ eventId: x.eventId, kind: x.kind, senderName: x.senderName, isOwn: !!x.isOwn, ts: x.ts, media: x.media, canRedact: !!(x.can && x.can.redact) })
      }
    }
    if (list.length === 0 && it)
      list.push({ eventId: it.eventId, kind: it.kind, senderName: it.senderName, isOwn: !!it.isOwn, ts: it.ts, media: it.media, canRedact: !!(it.can && it.can.redact) })
    root.images = list
    var idx = 0
    for (var k = 0; k < list.length; k++) if (list[k].eventId === it.eventId) { idx = k; break }
    root.item = it
    root.toast = ""
    root.cur = idx
    // Jump, do not travel: StrictlyEnforceRange would animate contentX through every page.
    var mv = pager.highlightMoveDuration
    pager.highlightMoveDuration = 0
    pager.currentIndex = idx
    pager.positionViewAtIndex(idx, ListView.SnapPosition)
    pager.highlightMoveDuration = mv
    root.fetchFull(idx)
    // Snap with the animation off, then release. Resolve the origin here: mapFromItem is one-shot.
    if (from && from.width > 1 && root.width > 1) {
      var c = root.mapFromItem(null, from.x + from.width / 2, from.y + from.height / 2)
      root.mOx = c.x
      root.mOy = c.y
      root.mScale = Math.max(0.04, from.width / root.width)
      root.instant = true
      root.morphing = true
      viewerGrow.restart()
    } else {
      root.instant = false
      root.morphing = false
    }
    root.shown = true
  }
  function debugMoreMenu() { moreMenu.open = !moreMenu.open }
  function close() {
    if (!root.shown) return
    root.morphing = false      // fade at full size; no reverse flight
    root.shown = false
    viewerFade.restart()
  }

  // The engine decodes with ffmpeg into a shared-memory surface (as the call tiles do).
  property string playShm: ""
  property string playingEvent: ""
  property real playDuration: 0
  property real playOffset: 0        // where the current decode started
  property real playElapsed: 0       // seconds since that decode started
  property bool scrubbing: false
  property real scrubPos: 0
  readonly property real playPos: root.scrubbing ? root.scrubPos : Math.min(root.playDuration, root.playOffset + root.playElapsed)
  Timer {
    interval: 250; repeat: true
    running: root.playingEvent !== "" && root.playShm !== "" && !root.scrubbing
    onTriggered: root.playElapsed += 0.25
  }
  function fmtPos(t) {
    var s = Math.max(0, Math.floor(t)), m = Math.floor(s / 60)
    return m + ":" + ((s % 60) < 10 ? "0" : "") + (s % 60)
  }
  function seekTo(sec) {
    if (!root.svc || root.playingEvent === "") return
    root.svc.seekVideo(sec, function(r, e) {
      if (r && r.path) { root.playShm = r.path; root.playOffset = r.startAt || sec; root.playElapsed = 0 }
    })
  }
  function isVideo(it) { return !!(it && it.kind === "video") }
  function togglePlayback() {
    var it = root.curItem
    if (!it || !root.isVideo(it) || !root.svc) return
    if (root.playingEvent === it.eventId) { root.stopPlayback(); return }
    root.stopPlayback()
    root.playingEvent = it.eventId
    root.svc.playVideo(root.roomId, it.eventId, function(r, e) {
      if (r && r.path && root.playingEvent === it.eventId) {
        root.playShm = r.path
        root.playDuration = r.duration || 0
        root.playOffset = r.startAt || 0
        root.playElapsed = 0
      }
      else { root.playingEvent = ""; root.note(e && e.message ? e.message : "Could not play video") }
    })
  }
  function stopPlayback() {
    if (root.playingEvent !== "" && root.svc) root.svc.stopVideo()
    root.playingEvent = ""
    root.playShm = ""
    root.playDuration = 0
    root.playOffset = 0
    root.playElapsed = 0
    root.scrubbing = false
  }
  onCurChanged: root.stopPlayback()

  function fetchFull(idx) {
    if (idx < 0 || idx >= root.images.length) return
    var it = root.images[idx]
    if (root.fullPaths[it.eventId]) return
    if (it.kind === "video") return
    if (it.media.path) { var m = Object.assign({}, root.fullPaths); m[it.eventId] = it.media.path; root.fullPaths = m; return }
    if (root.svc) root.svc.fetchMedia(root.roomId, it.eventId, null, function(r, e) {
      if (r && r.path) { var m2 = Object.assign({}, root.fullPaths); m2[it.eventId] = r.path; root.fullPaths = m2 }
    })
  }
  function pathOf(it) {
    if (!it) return ""
    // Videos: the poster frame — handing the .mp4 to an Image fails to decode.
    if (it.kind === "video") return it.media.thumbnailPath || ""
    return root.fullPaths[it.eventId] || it.media.path || it.media.thumbnailPath || ""
  }

  function fmtTs(ts) {
    var d = new Date(ts), now = new Date()
    var start = new Date(now.getFullYear(), now.getMonth(), now.getDate())
    var diff = Math.floor((start - new Date(d.getFullYear(), d.getMonth(), d.getDate())) / 86400000)
    var t = Qt.formatTime(d, "h:mm AP")
    if (diff === 0) return t
    if (diff === 1) return "Yesterday " + t
    if (diff < 7) return Qt.formatDate(d, "ddd") + " " + t
    return Qt.formatDate(d, "d MMM") + " " + t
  }

  function download() {
    if (!root.svc || !root.curItem) return
    root.svc.saveMedia(root.roomId, root.curItem.eventId, Quickshell.env("HOME") + "/Downloads", function(r, e) {
      root.note(r && r.path ? "Saved to " + r.path : "Save failed" + (e && e.message ? ": " + e.message : ""))
    })
  }
  function del() {
    if (!root.svc || !root.curItem) return
    root.svc.redact(root.roomId, root.curItem.eventId)
    root.close()
  }
  function share() {
    var p = root.pathOf(root.curItem)
    if (p === "") return
    Quickshell.execDetached(["sh", "-c", 'wl-copy -t "$(file -b --mime-type "$1")" < "$1"', "share", p])
    root.note("Image copied to clipboard")
  }

  /// True while the visible picture is magnified. On the root because the pager needs it.
  property bool pageZoomed: false
  function currentPage() {
    var it = pager.itemAtIndex(pager.currentIndex)
    return (it && it.zoomAbout !== undefined) ? it : null
  }
  /// Test hook: drive the zoom without synthesising pointer input.
  function debugZoom(z, fx, fy) {
    var pg = root.currentPage()
    if (!pg) return "no page"
    pg.zoomAbout(z, fx || 0, fy || 0)
    return JSON.stringify({ zoom: Math.round(pg.zoom * 100) / 100,
                            ox: Math.round(pg.ox), oy: Math.round(pg.oy),
                            zoomed: pg.zoomed, pagerInteractive: pager.interactive,
                            fit: [pg.fitW, pg.fitH], page: [pg.width, pg.height] })
  }
  function debugZoomReset() {
    var pg = root.currentPage()
    if (pg) pg.resetZoom()
    return pg ? "reset" : "no page"
  }
  property real wheelAcc: 0
  property double wheelLast: 0
  function wheelNav(dy) {
    root.wheelAcc += dy
    if (Math.abs(root.wheelAcc) < 120) return
    var dir = root.wheelAcc < 0 ? 1 : -1
    root.wheelAcc = 0
    var now = Date.now()
    if (now - root.wheelLast < 160) return
    root.wheelLast = now
    if (dir > 0 && pager.currentIndex < root.images.length - 1) pager.currentIndex++
    else if (dir < 0 && pager.currentIndex > 0) pager.currentIndex--
  }
  MouseArea {
    anchors.fill: parent
    onClicked: root.close()
    onWheel: function(w) { root.wheelNav(w.angleDelta.y); w.accepted = true }
  }

  ListView {
    id: pager
    anchors.fill: parent
    anchors.topMargin: Style.space(58)
    anchors.bottomMargin: Style.space(104)   // leave room for the scrubber row
    orientation: ListView.Horizontal
    clip: true
    spacing: Style.space(12)
    model: root.images
    snapMode: ListView.SnapOneItem
    highlightRangeMode: ListView.StrictlyEnforceRange
    // While magnified, a sideways drag pans the picture rather than turning the page.
    interactive: !root.pageZoomed
    readonly property real itemW: width - Style.space(56)
    preferredHighlightBegin: (width - itemW) / 2
    preferredHighlightEnd: (width - itemW) / 2 + itemW
    highlightMoveDuration: 200
    onCurrentIndexChanged: if (currentIndex >= 0) { root.cur = currentIndex; root.fetchFull(currentIndex) }
    delegate: Item {
      id: pageItem
      required property var modelData
      required property int index
      readonly property bool playing: root.playShm !== "" && root.playingEvent === modelData.eventId
      width: pager.itemW; height: pager.height
      // The pager's `clip` bounds only the list, so a neighbour drew over a zoomed page.
      clip: true
      z: index === pager.currentIndex ? 1 : 0
      readonly property real iw: (modelData.media && modelData.media.width) ? modelData.media.width : 800
      readonly property real ih: (modelData.media && modelData.media.height) ? modelData.media.height : 600
      readonly property real s: Math.min(width / iw, height / ih)

      // Zoom. Pinch, double-tap and ctrl+wheel drive the same two numbers, so platforms cannot drift.
      property real zoom: 1
      property real ox: 0
      property real oy: 0
      readonly property real maxZoom: 6
      readonly property real fitW: Math.max(1, Math.round(iw * s))
      readonly property real fitH: Math.max(1, Math.round(ih * s))
      readonly property bool zoomed: pageItem.zoom > 1.01

      /// Never let the picture leave its frame; an axis that fits entirely is pinned to centre.
      function clampOffsets() {
        var mx = Math.max(0, (pageItem.fitW * pageItem.zoom - pageItem.width) / 2)
        var my = Math.max(0, (pageItem.fitH * pageItem.zoom - pageItem.height) / 2)
        pageItem.ox = Math.max(-mx, Math.min(mx, pageItem.ox))
        pageItem.oy = Math.max(-my, Math.min(my, pageItem.oy))
      }

      /// Zoom about a point, keeping what is under the finger under the finger. `fx`/`fy` are
      /// relative to the centre. Scaled about the centre then translated, so `p` lands at
      /// `p*z + o`; holding `f` fixed across a zoom gives `o1 = f - (f - o0) * z1/z0`.
      function zoomAbout(z, fx, fy) {
        var z0 = pageItem.zoom
        var z1 = Math.max(1, Math.min(pageItem.maxZoom, z))
        if (Math.abs(z1 - z0) < 0.0001) return
        pageItem.ox = fx - (fx - pageItem.ox) * (z1 / z0)
        pageItem.oy = fy - (fy - pageItem.oy) * (z1 / z0)
        pageItem.zoom = z1
        pageItem.clampOffsets()
      }
      function resetZoom() { pageItem.zoom = 1; pageItem.ox = 0; pageItem.oy = 0 }

      // Changing picture starts fresh; arriving already magnified is disorienting.
      Connections {
        target: pager
        function onCurrentIndexChanged() {
          if (pageItem.index !== pager.currentIndex) pageItem.resetZoom()
        }
      }
      // While zoomed the horizontal flick belongs to the picture, not the filmstrip.
      onZoomedChanged: if (pageItem.index === pager.currentIndex) root.pageZoomed = pageItem.zoomed
      Item {
        id: media
        anchors.centerIn: parent
        width: pageItem.fitW; height: pageItem.fitH
        transform: [
          Scale { origin.x: media.width / 2; origin.y: media.height / 2
                  xScale: pageItem.zoom; yScale: pageItem.zoom },
          Translate { x: pageItem.ox; y: pageItem.oy }
        ]
        // A GIF needs AnimatedImage; Image decodes the first frame and stops.
        readonly property bool isGif: {
          var m = modelData.media && modelData.media.mime ? String(modelData.media.mime) : ""
          if (m === "image/gif") return true
          return /\.gif$/i.test(root.pathOf(modelData) || "")
        }
        Image {
          id: dimg
          anchors.fill: parent
          fillMode: Image.Stretch
          asynchronous: true; cache: true
          source: (!parent.isGif && root.pathOf(modelData) !== "") ? "file://" + root.pathOf(modelData) : ""
          visible: false
        }
        AnimatedImage {
          id: dgif
          anchors.fill: parent
          fillMode: Image.Stretch
          cache: true
          playing: true
          source: (parent.isGif && root.pathOf(modelData) !== "") ? "file://" + root.pathOf(modelData) : ""
          visible: false
        }
        MultiEffect {
          anchors.fill: parent
          source: parent.isGif ? dgif : dimg
          maskEnabled: true; maskThresholdMin: 0.5; maskSpreadAtMin: 1.0; maskSource: dmask
          visible: (parent.isGif ? dgif.status === Image.Ready : dimg.status === Image.Ready) && !pageItem.playing
        }
        Loader {
          id: vidLoader
          anchors.fill: parent
          active: pageItem.playing
          // Relative, so it resolves against this file wherever the plugin is
          // installed. It stays behind a Loader because VideoTile pulls in the
          // shm video plugin, which only exists on Linux.
          source: "../calls/VideoTile.qml"
          onLoaded: item.shmPath = Qt.binding(function() { return root.playShm })
          layer.enabled: true
          layer.smooth: true
          layer.effect: MultiEffect { maskEnabled: true; maskThresholdMin: 0.5; maskSpreadAtMin: 1.0; maskSource: vidRound }
        }
        Rectangle {
          id: vidRound
          anchors.fill: parent
          radius: Style.space(16)
          antialiasing: true
          color: "black"; visible: false; layer.enabled: true; layer.smooth: true
        }
        Rectangle {
          visible: modelData.kind === "video" && !pageItem.playing
          anchors.centerIn: parent
          width: Style.space(60); height: Style.space(60); radius: width / 2
          color: Util.alpha("#000000", 0.55)
          IconLabel { anchors.centerIn: parent; icon: Icons.play; color: "#ffffff"; size: Style.space(26) }
        }
        Rectangle { id: dmask; anchors.fill: parent; radius: Style.space(16); color: "black"; visible: false; layer.enabled: true; layer.smooth: true }
        MouseArea { anchors.fill: parent; cursorShape: modelData.kind === "video" ? Qt.PointingHandCursor : Qt.ArrowCursor; onClicked: if (modelData.kind === "video") root.togglePlayback() }
      }
      // Pinch: the touch and trackpad route.
      PinchHandler {
        target: null
        enabled: modelData.kind !== "video"
        onScaleChanged: function (delta) {
          var c = pageItem.mapFromItem(null, pinchPoint.scenePosition.x, pinchPoint.scenePosition.y)
          pageItem.zoomAbout(pageItem.zoom * delta, c.x - pageItem.width / 2, c.y - pageItem.height / 2)
        }
      }

      // Double-tap: works everywhere, mouse or touch. Toggles fit <-> 2.5x at the point tapped.
      TapHandler {
        enabled: modelData.kind !== "video"
        acceptedButtons: Qt.LeftButton
        onDoubleTapped: function (ev) {
          if (pageItem.zoomed) pageItem.resetZoom()
          else pageItem.zoomAbout(2.5, ev.position.x - pageItem.width / 2,
                                       ev.position.y - pageItem.height / 2)
        }
      }

      // Pan, once magnified. A PointHandler takes only a passive grab, so this
      // cannot be cancelled out from under the drag the way an exclusive grab
      // can — the same reason the map pans this way. See docs/portability.md.
      PointHandler {
        id: panner
        enabled: pageItem.zoomed
        acceptedButtons: Qt.LeftButton
        target: null
        property real lx: 0
        property real ly: 0
        onActiveChanged: {
          if (panner.active) { panner.lx = panner.point.position.x; panner.ly = panner.point.position.y }
        }
        onPointChanged: {
          if (!panner.active) return
          pageItem.ox += panner.point.position.x - panner.lx
          pageItem.oy += panner.point.position.y - panner.ly
          panner.lx = panner.point.position.x
          panner.ly = panner.point.position.y
          pageItem.clampOffsets()
        }
      }

      Spinner { anchors.centerIn: parent; visible: root.pathOf(modelData) === "" && modelData.kind !== "video"; color: Util.alpha(root.fg, 0.5) }
    }
  }

  // This higher-z overlay claims wheel events outright, so one notch is exactly one picture.
  // Ctrl+wheel zooms; once magnified a plain wheel zooms too, or there is no way out with a mouse.
  Item {
    anchors.fill: pager
    z: 10
    WheelHandler {
      onWheel: function (ev) {
        var pg = root.currentPage()
        if (pg && (ev.modifiers & Qt.ControlModifier || pg.zoomed)) {
          var c = pg.mapFromItem(null, ev.x, ev.y)
          pg.zoomAbout(pg.zoom * (ev.angleDelta.y > 0 ? 1.25 : 0.8),
                       c.x - pg.width / 2, c.y - pg.height / 2)
          return
        }
        root.wheelNav(ev.angleDelta.y)
      }
    }
  }

  Item {
    id: topBar
    anchors.top: parent.top; anchors.left: parent.left; anchors.right: parent.right
    height: Style.space(56)
    PanelActionButton { id: closeBtn; anchors.left: parent.left; anchors.leftMargin: Style.space(8); anchors.verticalCenter: parent.verticalCenter; fontFamily: Fonts.icon; iconText: Icons.close; foreground: root.fg; onClicked: root.close() }
    Column {
      anchors.left: closeBtn.right; anchors.leftMargin: Style.space(8); anchors.verticalCenter: parent.verticalCenter
      Text { text: root.curItem ? (root.curItem.isOwn ? "You" : (root.curItem.senderName || "")) : ""; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.subtitle; font.bold: true }
      Text { text: root.curItem ? root.fmtTs(root.curItem.ts) : ""; color: Util.alpha(root.fg, 0.55); font.family: Fonts.ui; font.pixelSize: Style.font.caption }
    }
    Row {
      anchors.right: parent.right; anchors.rightMargin: Style.space(8); anchors.verticalCenter: parent.verticalCenter
      spacing: Style.space(2)
      PanelActionButton { anchors.verticalCenter: parent.verticalCenter; fontFamily: Fonts.icon; iconText: Icons.download; foreground: root.fg; id: dlBtn; tooltipText: ""; onClicked: root.download()
        HoverHandler { onHoveredChanged: root.showTip(dlBtn, hovered, "Download") } }
      PanelActionButton { anchors.verticalCenter: parent.verticalCenter; visible: root.curItem !== null && root.curItem.canRedact; fontFamily: Fonts.icon; iconText: Icons.trash; foreground: root.fg; id: delBtn; tooltipText: ""; onClicked: root.del()
        HoverHandler { onHoveredChanged: root.showTip(delBtn, hovered, "Delete") } }
      PanelActionButton { id: moreBtn; anchors.verticalCenter: parent.verticalCenter; fontFamily: Fonts.icon; iconText: Icons.moreVertical; foreground: root.fg; onClicked: moreMenu.open = !moreMenu.open }
    }
  }

  // ⋮ menu: Forward / Share. Drawn inside the viewer — as an xdg-popup it rendered outside.
  Item {
    id: moreMenu
    property bool open: false
    anchors.fill: parent
    z: 60
    visible: open || moreCard.opacity > 0.01
    MouseArea { anchors.fill: parent; visible: moreMenu.open; onClicked: moreMenu.open = false }
    Rectangle {
      id: moreCard
      width: Style.space(160)
      height: mmCol.implicitHeight + Style.space(12)
      // Anchored, not mapped: mapToItem() is one-shot, so a binding on it keeps a stale position.
      anchors.right: parent.right
      anchors.rightMargin: Style.space(10)
      y: topBar.y + (topBar.height + moreBtn.height) / 2 + Style.space(3)
      radius: Style.space(12)
      antialiasing: true
      color: Util.alpha(Qt.lighter(Color.menu.background, 1.4), 0.99)
      border.width: 1
      border.color: Util.alpha(root.fg, 0.12)
      transformOrigin: Item.TopRight
      opacity: moreMenu.open ? 1 : 0
      scale: moreMenu.open ? 1 : 0.85
      Behavior on opacity { NumberAnimation { duration: 110 } }
      Behavior on scale { NumberAnimation { duration: 140; easing.type: Easing.OutCubic } }
      MouseArea { anchors.fill: parent }
      Column {
        id: mmCol
        x: Style.space(6); y: Style.space(6)
        width: parent.width - Style.space(12)
        Repeater {
          model: [ { t: "Forward", a: "fwd", icon: Icons.retry }, { t: "Share", a: "share", icon: Icons.share } ]
          delegate: Rectangle {
            required property var modelData
            width: parent.width; height: Style.space(30); radius: Style.space(8)
            color: mh.containsMouse ? Util.alpha(root.fg, 0.1) : "transparent"
            IconLabel { anchors.verticalCenter: parent.verticalCenter; anchors.left: parent.left; anchors.leftMargin: Style.space(10); icon: modelData.icon; color: root.fg; opacity: 0.85; size: Style.font.icon }
            Text { anchors.verticalCenter: parent.verticalCenter; anchors.left: parent.left; anchors.leftMargin: Style.space(36); text: modelData.t; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body }
            MouseArea { id: mh; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor
              onClicked: { moreMenu.open = false; if (modelData.a === "share") root.share(); else fwdMenu.open = true } }
          }
        }
      }
    }
  }

  // Forward: pick a room, send the cached file there
  Item {
    id: fwdMenu
    property bool open: false
    anchors.fill: parent
    z: 61
    visible: open || fwdCard.opacity > 0.01
    MouseArea { anchors.fill: parent; visible: fwdMenu.open; onClicked: fwdMenu.open = false }
    Rectangle {
      id: fwdCard
      width: Style.space(240)
      height: Math.min(Style.space(330), fwdCol.implicitHeight + Style.space(12))
      anchors.right: parent.right
      anchors.rightMargin: Style.space(10)
      y: topBar.y + (topBar.height + moreBtn.height) / 2 + Style.space(3)
      radius: Style.space(12)
      antialiasing: true
      color: Util.alpha(Qt.lighter(Color.menu.background, 1.4), 0.99)
      border.width: 1
      border.color: Util.alpha(root.fg, 0.12)
      transformOrigin: Item.TopRight
      opacity: fwdMenu.open ? 1 : 0
      scale: fwdMenu.open ? 1 : 0.85
      Behavior on opacity { NumberAnimation { duration: 110 } }
      Behavior on scale { NumberAnimation { duration: 140; easing.type: Easing.OutCubic } }
      MouseArea { anchors.fill: parent }
      Column {
        id: fwdCol
        x: Style.space(6); y: Style.space(6)
        width: parent.width - Style.space(12)
        Text { text: "Forward to"; color: Util.alpha(root.fg, 0.6); font.family: Fonts.ui; font.pixelSize: Style.font.caption; font.bold: true; bottomPadding: Style.space(4) }
        Repeater {
          model: root.svc ? root.svc.rooms.filter(function(r) { return !r.isSpace && !r.isInvite }).slice(0, 8) : []
          delegate: Rectangle {
            required property var modelData
            width: parent.width; height: Style.space(30); radius: Style.space(8)
            color: fh.containsMouse ? Util.alpha(root.fg, 0.1) : "transparent"
            Text { anchors.verticalCenter: parent.verticalCenter; anchors.left: parent.left; anchors.leftMargin: Style.space(10); width: parent.width - Style.space(20); elide: Text.ElideRight; text: modelData.name || modelData.id; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body }
            MouseArea {
              id: fh; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor
              onClicked: {
                fwdMenu.open = false
                var pth = root.pathOf(root.curItem)
                if (pth !== "" && root.svc) { root.svc.sendFiles(modelData.id, [pth]); root.note("Forwarded to " + (modelData.name || "room")) }
              }
            }
          }
        }
      }
    }
  }

  // Scrubber (visible while a video is playing). z above the pager's wheel
  // catcher (z:10) — otherwise that overlay swallows the drag events.
  Item {
    z: 20
    visible: root.playingEvent !== ""
    anchors.left: parent.left; anchors.right: parent.right
    anchors.bottom: parent.bottom; anchors.bottomMargin: Style.space(62)
    anchors.leftMargin: Style.space(16); anchors.rightMargin: Style.space(16)
    height: Style.space(34)
    Text {
      id: posLabel
      anchors.left: parent.left; anchors.verticalCenter: parent.verticalCenter
      text: root.fmtPos(root.playPos); color: Util.alpha(root.fg, 0.85)
      font.family: Fonts.ui; font.pixelSize: Style.font.caption
    }
    Text {
      id: durLabel
      anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter
      text: root.playDuration > 0 ? root.fmtPos(root.playDuration) : "--:--"; color: Util.alpha(root.fg, 0.85)
      font.family: Fonts.ui; font.pixelSize: Style.font.caption
    }
    Item {
      id: track
      anchors.left: posLabel.right; anchors.right: durLabel.left
      anchors.leftMargin: Style.space(10); anchors.rightMargin: Style.space(10)
      anchors.verticalCenter: parent.verticalCenter
      height: Style.space(18)
      readonly property real frac: root.playDuration > 0 ? Math.max(0, Math.min(1, root.playPos / root.playDuration)) : 0
      Rectangle {
        anchors.verticalCenter: parent.verticalCenter
        width: parent.width; height: Style.space(4); radius: 2
        color: Util.alpha(root.fg, 0.25)
        Rectangle { width: parent.width * track.frac; height: parent.height; radius: 2; color: root.accent }
      }
      Rectangle {
        width: Style.space(13); height: Style.space(13); radius: width / 2
        anchors.verticalCenter: parent.verticalCenter
        x: track.frac * (track.width - width)
        color: root.accent
        antialiasing: true
      }
      MouseArea {
        anchors.fill: parent
        anchors.topMargin: -Style.space(10); anchors.bottomMargin: -Style.space(10)
        enabled: root.playDuration > 0
        cursorShape: Qt.PointingHandCursor
        preventStealing: true
        function posAt(mx) { return Math.max(0, Math.min(1, mx / track.width)) * root.playDuration }
        onPressed: function(m) { root.scrubbing = true; root.scrubPos = posAt(m.x) }
        onPositionChanged: function(m) { if (pressed) root.scrubPos = posAt(m.x) }
        onReleased: function(m) { var t = posAt(m.x); root.scrubbing = false; root.seekTo(t) }
        onCanceled: root.scrubbing = false
      }
    }
  }

  Row {
    z: 20
    anchors.bottom: parent.bottom; anchors.horizontalCenter: parent.horizontalCenter; anchors.bottomMargin: Style.space(12)
    spacing: Style.space(10)
    Repeater {
      model: ["👍", "❤️", "😂", "😮", "😢", "😡"]
      delegate: Rectangle {
        required property var modelData
        width: Style.space(42); height: Style.space(42); radius: height / 2
        color: rh.containsMouse ? Util.alpha(root.accent, 0.25) : Util.alpha(root.fg, 0.08)
        TextMetrics { id: em2; font.pixelSize: Style.space(20); text: modelData }
        Text { text: modelData; font.pixelSize: Style.space(20)
               anchors.verticalCenter: parent.verticalCenter
               x: (parent.width - em2.tightBoundingRect.width) / 2 - em2.tightBoundingRect.x }
        MouseArea {
          id: rh; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor
          onClicked: { if (root.svc && root.curItem) { root.svc.react(root.roomId, root.curItem.eventId, modelData); root.note("Reacted " + modelData) } }
        }
      }
    }
  }
  Text {
    anchors.bottom: parent.bottom; anchors.horizontalCenter: parent.horizontalCenter; anchors.bottomMargin: Style.space(58)
    visible: root.toast !== ""
    text: root.toast
    color: Util.alpha(root.fg, 0.7); font.family: Fonts.ui; font.pixelSize: Style.font.caption
  }
}
