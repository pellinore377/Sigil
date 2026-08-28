import QtQuick
import QtQuick.Effects
import QtQuick.Controls as QQC
import Quickshell.Io
import qs.Commons
import qs.Ui
import "../components"
import ".."

// Conversation page: back header with call buttons, bubble timeline, pill composer.
Item {
  id: root
  property var svc: null
  property color fg: Color.menu.text
  property string roomId: ""
  property bool visibleToUser: false
  property bool debugInvite: false
  /// Threads and the pinned list are timelines keyed by a string starting with
  /// the room id. Pins are NOT a view kind: the SDK's pinned-events focus does
  /// not work (see the engine's pins.rs), so that path is deleted.
  readonly property string viewKind: (svc && svc.isThreadKey(roomId)) ? "thread" : "room"
  readonly property bool isRoomView: root.viewKind === "room"
  /// The actual room, whatever kind of view this is.
  readonly property string baseRoomId: svc ? svc.roomOfKey(roomId) : roomId
  readonly property var room: root.debugInvite ? { isInvite: true, name: "Test Room", inviter: "@alice:example.com", joinedMembers: 4 } : (svc ? svc.room(root.baseRoomId) : null)
  signal backRequested()
  signal startCall(bool video)
  signal attachRequested()
  signal openLocation(var item, var from)
  signal openAudio(var item)
  signal openDmWith(string userId)
  signal shareVcf(string userId, string displayName)
  signal openDocument(var item)
  signal openImage(var item, var from)
  signal playVideo(var item, var from)
  signal menuRequested(var item, real sceneX, real sceneY, real bw, real bh, var bubbleObj)
  signal closeSheetRequested()
  signal navRequested(string what)
  signal openThreadRequested(string rootId)
  property var chatTheme: ({})
  property bool menuOpen: false
  readonly property bool themed: (root.chatTheme.accent || "") !== ""
  readonly property color accC: root.themed ? Qt.color(root.chatTheme.accent) : Color.accent
  function mixc(a, b, t) { return Qt.rgba(a.r * (1 - t) + b.r * t, a.g * (1 - t) + b.g * t, a.b * (1 - t) + b.b * t, 1) }
  readonly property color themedSend: root.themed ? Qt.darker(root.accC, 1.15) : Util.alpha(Color.accent, 0.9)
  // ONE literal surface color for all themed chrome, or the tones never match.
  readonly property real tintAmt: 0.35
  readonly property color surfaceC: root.themed ? root.mixc(Qt.lighter(Color.menu.background, 1.35), root.accC, root.tintAmt) : Color.popups.background
  readonly property color chromeC: root.themed ? root.surfaceC : Qt.lighter(Color.menu.background, 1.35)
  // Composer chip and sheet-control fill; unthemed rooms need a deeper tone.
  readonly property color chipC: root.themed ? Util.alpha(root.convoC, 0.92)
                                             : Util.alpha(Qt.darker(Color.menu.background, 2.1), 0.96)

  // Alpha-weighted luminance sunk to a target, so grey-on-light labels survive.
  function lum(c) { return (0.299 * c.r + 0.587 * c.g + 0.114 * c.b) * c.a }
  readonly property color deepChipC: {
    var l = root.lum(root.chipC)
    return l > 0.19 ? Qt.darker(root.chipC, l / 0.19) : root.chipC
  }

  readonly property color convoC: {
    var d = Qt.darker(Color.menu.background, 1.35)
    if ((root.chatTheme.accent || "") === "") return d
    var a = Qt.color(root.chatTheme.accent)
    return Qt.rgba(d.r * 0.82 + a.r * 0.18, d.g * 0.82 + a.g * 0.18, d.b * 0.82 + a.b * 0.18, 1)
  }
  function themeGradPair(i) {
    var base = (root.chatTheme.accent || "") !== "" ? Qt.color(root.chatTheme.accent) : Color.accent
    var h = base.hslHue < 0 ? 0.6 : base.hslHue
    var hh = (h + [-0.04, 0, 0.04][i % 3] + 1) % 1
    var row = Math.floor(i / 3)
    var sat = Math.max(0.5, base.hslSaturation)
    var l = Math.max(0.25, Math.min(0.55, base.hslLightness))
    var top = Math.min(0.62, [l * 1.15, l * 0.85, l * 0.6][row])
    var bot = [l * 0.45, l * 0.3, l * 0.18][row]
    return [Qt.hsla(hh, sat, top, 1), Qt.hsla(hh, sat, bot, 1)]
  }

  // Playback
  property string playingVoice: ""
  /// Last thing started, running or not; playingVoice clears at the end.
  property string playedVoice: ""
  property real voicePos: 0
  property real voiceBase: 0
  /// Length of the thing playing: the engine sends no "finished" event.
  property real voiceDur: 0
  Timer {
    interval: 200; repeat: true; running: root.playingVoice !== ""
    onTriggered: {
      root.voicePos += 0.2
      if (root.voiceDur > 0 && root.voicePos >= root.voiceDur) root.endPlayback()
    }
  }
  function endPlayback() {
    if (root.svc) root.svc.stopAudio()
    root.voicePos = root.voiceDur
    root.playingVoice = ""
  }
  /// Voice notes carry their duration in the event; tracks get it from the engine.
  function durationOf(it) {
    if (it && it.media && it.media.duration) return it.media.duration / 1000
    if (!root.svc || !it || !it.eventId) return 0
    var t = root.svc.audioInfos[root.svc.docThumbKey(root.roomId, it.eventId)]
    return (t && t.duration) ? t.duration / 1000 : 0
  }
  function toggleVoice(it) {
    if (!it || !root.svc) return
    if (root.playingVoice === it.eventId) { root.svc.stopAudio(); root.playingVoice = ""; return }
    root.playVoiceAt(it, 0)
  }
  function playVoiceAt(it, pos) {
    if (!it || !root.svc) return
    root.playingVoice = it.eventId
    root.playedVoice = it.eventId
    root.voiceDur = root.durationOf(it)
    root.voicePos = pos
    root.svc.playAudio(root.roomId, it.eventId, pos, function(r, e) { if (e) root.playingVoice = "" })
  }

  function debugVoicePanel(on) { root.recorderOpen = on }
  function debugVoiceRecord() { recorder.startRecording() }
  function debugVoiceAttach() {
    recorder.stopRecording()
    attachWait.restart()
  }
  Timer { id: attachWait; interval: 400; repeat: true; property int tries: 0
    onTriggered: {
      tries++
      if (recorder.state === "ready") { recorder.attach(); stop(); tries = 0 }
      else if (tries > 12) { stop(); tries = 0 }
    }
  }

  function debugVoiceClear() { root.voicePath = ""; root.voiceDuration = 0; root.voiceWaveform = []; root.recorderOpen = false; recorder.reset() }

  // Pixels the timeline travels per mouse-wheel notch.
  property real wheelStep: Style.space(35)
  // Ceiling on how fast the timeline may travel, px/s.
  property real wheelMaxSpeed: Style.space(9000)
  /// How much travel one spin may queue, in screens.
  property real wheelLeadScreens: 4

  // Newest message we sent, so its delegate can keep the receipt line open.
  property string latestOwnId: ""
  // A receipt usually lands on a state or call event, which the timeline hides,
  // so match each reader to the newest *own message* at or before their receipt.
  property string receiptEventId: ""
  property var receiptReaders: []
  property var readerMarks: ({})       // kept for the details row's own lookups
  // Marks animate only while you are watching; on room open they are simply there.
  property bool animateMarks: false
  // "eventId|userId" pairs already drawn, so a re-broadcast is not fresh news.
  property var knownReaders: ({})
  // Each event may claim the entry animation once, so a rebuilt delegate is still.
  property bool entryAnimAllowed: false
  property var animatedIds: ({})
  Timer { id: entrySettle; interval: 600; onTriggered: root.entryAnimAllowed = true }
  function claimEntry(eid) {
    if (!root.entryAnimAllowed || !eid) return false
    if (root.animatedIds[eid]) return false
    var a = root.animatedIds
    a[eid] = true
    root.animatedIds = a
    return true
  }
  // Test hook: reactions on the newest message, in the local model only.
  function debugReact(keys, idx) {
    var m = root.tl ? root.tl.model : null
    var at = Math.max(0, Math.min(m ? m.count - 1 : 0, Number(idx) || 0))
    if (!m || m.count === 0) return "none"
    var made = []
    var parts = String(keys).split(",").filter(function(x) { return x !== "" })
    for (var i = 0; i < parts.length; i++) {
      var kv = parts[i].split(":")
      made.push({ key: kv[0], count: kv.length > 1 ? Number(kv[1]) : 1, mine: i === 0 })
    }
    m.setProperty(at, "reactions", made)
    var back = m.get(at).reactions
    var it = list.itemAtIndex(at)
    return JSON.stringify({ body: m.get(at).body, has: !!back,
      n: back ? (back.count !== undefined ? back.count : back.length) : -1,
      delegate: it ? it.reactionCount : "no-item", lift: it ? it.reactionLift : -1 })
  }
  function debugTapAudio() {
    var m = root.tl ? root.tl.model : null
    if (!m) return "none"
    for (var i = 0; i < m.count; i++) {
      var it = m.get(i)
      if (it.kind !== "audio") continue
      if (it.media && it.media.waveform) continue      // a voice note, not a track
      root.openAudio(it)
      return (it.media ? (it.media.filename || "") : "") + " @" + i
    }
    return "no track in room"
  }
  function debugTapDoc(which) {
    var m = root.tl ? root.tl.model : null
    if (!m) return "none"
    var want = String(which || "")
    for (var i = 0; i < m.count; i++) {
      var it = m.get(i)
      if (it.kind !== "file" || !it.media || !it.media.previewable) continue
      if (want !== "" && String(it.media.filename || "").indexOf(want) < 0) continue
      root.openDocument(it)
      return (it.media.filename || "") + " @" + i
    }
    return "no previewable file in room"
  }
  // Test hook: open the newest shared location. `which`: ""/"location" or "live".
  function debugTapLocation(which) {
    var m = root.tl ? root.tl.model : null
    if (!m) return "none"
    var want = (which === "live") ? "liveLocation" : "location"
    for (var i = 0; i < m.count; i++) {
      if (m.get(i).kind !== want) continue
      root.openLocation(m.get(i), undefined)
      return "ok @" + i
    }
    return "no location in room"
  }
  function debugTapMedia() {
    var m = root.tl ? root.tl.model : null
    if (!m) return "none"
    for (var i = 0; i < m.count; i++) {
      var k = m.get(i).kind
      if (k !== "image" && k !== "video") continue
      var it = list.itemAtIndex(i)
      if (it && it.debugOpenMedia) return String(it.debugOpenMedia()) + " @" + i
    }
    return "no media in view"
  }
  // Test hook: inject a message locally; the engine's next diff corrects it.
  function debugFakeMessage(own, text) {
    var m = root.tl ? root.tl.model : null
    if (!m || m.count === 0) return "no model"
    var src = m.get(0)
    var copy = {}
    for (var k in src) copy[k] = src[k]
    copy.id = "fake-" + m.count
    copy.eventId = null
    copy.txnId = "fake"
    copy.body = String(text || "test message")
    copy.html = ""
    copy.kind = "text"
    copy.isOwn = own === "1"
    copy.sendState = own === "1" ? "sending" : "sent"
    copy.reactions = []
    copy.media = null
    copy.replyTo = null
    copy.ts = root.svc ? Date.now() : src.ts
    m.insert(0, copy)
    return "inserted"
  }
  function debugItemInfo(idx) {
    var m = root.tl ? root.tl.model : null
    if (!m || m.count === 0) return "no model"
    var at = Math.max(0, Math.min(m.count - 1, Number(idx) || 0))
    var it = m.get(at)
    return JSON.stringify({ id: it.id, kind: it.kind, eventId: it.eventId, live: it.liveShare || null,
                            asset: it.location ? (it.location.asset || "") : "",
                            thumb: it.media ? (it.media.thumbnailPath || "") : "",
                            parts: (it.partsJson || "").length,
                            body: (it.body || "").slice(0, 20) })
  }
  function debugReaders() {
    return JSON.stringify({ marks: root.animateMarks, receiptOn: root.receiptEventId,
                            readers: root.receiptReaders })
  }
  function debugGeomAt(idx) {
    var it = list.itemAtIndex(Math.max(0, Number(idx) || 0))
    return (it && it.debugGeom) ? it.debugGeom() : "none"
  }
  function debugReplayEntry(idx) {
    var it = list.itemAtIndex(Math.max(0, Number(idx) || 0))
    if (!it || !it.playEntry) return "none"
    it.playEntry()
    return "ok"
  }
  // Test hook: tap the newest bubble as a finger would, exercising the re-pin path.
  function debugTapNewest() {
    var it = list.itemAtIndex(0)
    if (!it || !it.toggleDetails) return "none"
    it.toggleDetails()
    return "ok"
  }
  // Test hook: force every bubble's detail row open (sigilui details 1).
  property bool debugDetailsAll: false

  /// A call we are part of, running in this room.
  property var debugCall: null
  readonly property bool callHere: {
    var c = root.debugCall ? root.debugCall : (root.svc ? root.svc.call : null)
    if (!c || root.roomId === "") return false
    if (c.roomId !== root.roomId) return false
    return c.state === "joining" || c.state === "connected" || c.state === "reconnecting" || c.state === "leaving"
  }
  signal joinCallRequested()
  /// Another page is on top of us; the composer must stop claiming keys.
  property bool covered: false
  // After a moment in the room, later dividers stop being drawn.
  Timer { id: dividerHold; interval: 1200; onTriggered: root.unreadDividerAllowed = false }
  Timer { id: markSettle; interval: 1000; onTriggered: root.animateMarks = true }

  function recomputeReceiptMarks() {
    var m = root.tl ? root.tl.model : null
    if (!m) { root.receiptEventId = ""; root.receiptReaders = []; return }
    var newest = null
    for (var i = 0; i < Math.min(m.count, 200); i++) {
      var it = m.get(i)
      if (!it || !it.eventId) continue
      if (it.kind === "state" || it.kind === "readMarker" || it.kind === "dayDivider" || it.kind === "rtcNotification") continue
      newest = it
      break
    }
    if (!newest || !newest.isOwn) { root.receiptEventId = ""; root.receiptReaders = []; return }
    root.receiptEventId = newest.eventId
    var all = (root.svc && root.roomId && root.svc.receiptsByRoom[root.roomId]) ? root.svc.receiptsByRoom[root.roomId] : []
    var me = root.svc ? root.svc.userId : ""
    var out = []
    for (var r = 0; r < all.length; r++) {
      var rd = all[r]
      if (!rd || rd.userId === me) continue
      if ((rd.ts || 0) < (newest.ts || 0)) continue
      var key = newest.eventId + "|" + rd.userId
      // animateMarks stays false until the room settles, so the backlog is silent.
      var isNew = root.animateMarks && !root.knownReaders[key]
      if (isNew) root.knownReaders[key] = true
      out.push({ userId: rd.userId, displayName: rd.displayName,
                 avatarPath: rd.avatarPath, ts: rd.ts, fresh: isNew })
    }
    root.receiptReaders = out.slice(0, 4)
  }

  Connections {
    target: root.svc
    ignoreUnknownSignals: true
    function onReceiptsByRoomChanged() { root.recomputeReceiptMarks() }
  }

  function recomputeLatestOwn() {
    var m = root.tl ? root.tl.model : null
    if (!m) { root.latestOwnId = ""; return }
    // Scan the whole loaded timeline: your newest message can be hundreds back.
    var n = Math.min(m.count, 500)
    for (var i = 0; i < n; i++) {
      var it = m.get(i)
      if (!it || !it.isOwn) continue
      if (it.kind === "state" || it.kind === "readMarker") continue
      root.latestOwnId = it.eventId || ""
      return
    }
    root.latestOwnId = ""
    root.recomputeReceiptMarks()
  }
  Connections {
    target: root.tl ? root.tl.model : null
    ignoreUnknownSignals: true
    function onCountChanged() { root.recomputeLatestOwn() }
  }
  onTlChanged: Qt.callLater(root.recomputeLatestOwn)

  property var debugTypers: null
  function debugTyping(on) { root.debugTypers = on ? [{ userId: root.svc ? root.svc.userId : "@peer:x", displayName: "Test Peer", avatarPath: root.svc ? root.svc.avatarPath : "" }] : null }

  function debugPicker() { return attachSheet.debugPicker() }
  function debugAttach(page) { root.attachOpen = page !== ""; if (root.attachOpen) { attachSheet.reset(); if (page !== "grid") attachSheet.activate(page) } }

  function debugCtxMenu(x, y) { composerCtxMenu.openMenu(x, y) }
  function debugScroll(notches) { list.wheelByAngle(notches * 120); return Math.round(list.contentY) }
  // Per-frame scroll trace: net travel over an interval hides stutter.
  property bool traceOn: false
  /// Realised window, in screens. Large on purpose — see the cacheBuffer comment.
  property real cacheMul: 20
  property bool debugNoNotices: false
  /// Grow the realised window in STEPS once the room settles: opening straight at
  /// the full size builds ~150 delegates in a frame or two, which is entry lag.
  property real warmSteps: 2
  Timer {
    id: warmTimer
    interval: 180
    repeat: true
    running: false
    onTriggered: {
      if (root.warmSteps >= root.cacheMul) { running = false; return }
      // Never grow mid-gesture: it re-lays out everything already realised.
      if (root.scrolling) return
      root.warmSteps = Math.min(root.cacheMul, root.warmSteps * 2)
    }
  }
  function restartWarm() { root.warmSteps = 2; warmTimer.restart() }
  /// True while a wheel scroll is in flight, so expensive bodies can wait.
  readonly property bool scrolling: list.wheelRemaining !== 0 || list.moving || list.flicking
  property var trace: []
  /// Which delegate kinds were constructed since the last frame.
  property var pendingBuilds: []
  property real traceLastY: 0
  function noteBuild(kind) { if (root.traceOn) root.pendingBuilds.push(kind) }
  function debugTrace(on) {
    if (on) { root.trace = []; root.traceOn = true; return "recording" }
    root.traceOn = false
    var t = root.trace
    root.trace = []
    return JSON.stringify(t)
  }

  // Frame recorder, outside the wheel animation so it also sees flicks and drags.
  FrameAnimation {
    id: traceDrive
    running: root.traceOn
    onTriggered: {
      // Record only while the list is actually moving; idle frames fill the buffer.
      var moved = Math.abs(list.contentY - root.traceLastY) > 0.5
      root.traceLastY = list.contentY
      if (!moved && !list.moving && !list.flicking) { root.pendingBuilds = []; return }
      var built = root.pendingBuilds
      root.pendingBuilds = []
      var mid = list.contentY + list.height / 2
      var vi = list.indexAt(list.width / 2, mid)
      var vit = vi >= 0 ? list.itemAtIndex(vi) : null
      if (root.trace.length < 3000)
        root.trace.push([Math.round(frameTime * 10000) / 10,
                         Math.round(list.contentY),
                         Math.round(list.originY),
                         Math.round(list.contentHeight),
                         list.count, vi,
                         vit ? Math.round(vit.y - list.contentY) : -99999,
                         built,
                         vit ? Math.round(vit.height) : -1])
    }
  }

  function debugList() {
    return JSON.stringify({
      count: list.count,
      contentY: Math.round(list.contentY), originY: Math.round(list.originY),
      contentH: Math.round(list.contentHeight), h: Math.round(list.height),
      fromStart: Math.round(list.fromStart), fromEnd: Math.round(list.fromEnd),
      atTop: list.atYBeginning, atBottom: list.atBottom,
      pagination: root.pagination, wheelRunning: wheelDrive.running,
      wheelRemaining: Math.round(list.wheelRemaining)
    })
  }

  function debugVoiceState() { return root.recorderOpen + "/" + recorder.state + "/" + (root.svc ? "svc" : "nosvc") }

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

  function fmtDur(t) {
    var s = Math.max(0, Math.round(t)), m = Math.floor(s / 60)
    return (m < 10 ? "0" : "") + m + ":" + ((s % 60) < 10 ? "0" : "") + (s % 60)
  }

  /// The message a jump landed on, flashed briefly so the reader can find it.
  property string jumpedTo: ""
  Timer { id: jumpFlash; interval: 1600; onTriggered: root.jumpedTo = "" }

  function indexOfEvent(eventId) {
    if (!root.tl || !root.tl.model) return -1
    var m = root.tl.model
    for (var i = 0; i < m.count; i++) if (m.get(i).eventId === eventId) return i
    return -1
  }

  /// Go to a message, loading history until it is found.
  /// A pinned message is usually old and NOT in the loaded model, so paginate and
  /// retry, bounded, rather than searching once or spinning forever.
  function scrollToEvent(eventId, attempt) {
    if (!eventId || !root.tl) return
    attempt = attempt || 0
    var i = root.indexOfEvent(eventId)
    if (i >= 0) {
      // Refill the realised window gradually: after a jump the whole buffer is cold.
      root.restartWarm()
      // Keep re-positioning for a few frames. On a BottomToTop list whose extent is
      // estimated, one positionViewAtIndex lands elsewhere: realising the target moves everything.
      list.positionViewAtIndex(i, ListView.Center)
      root.jumpedTo = eventId
      jumpSettle.target = eventId
      jumpSettle.tries = 0
      jumpSettle.restart()
      jumpFlash.restart()
      return
    }
    if (attempt >= 20 || root.pagination === "timelineStart") return
    if (root.svc) root.svc.paginate(root.baseRoomId === root.roomId ? root.roomId : root.roomId)
    jumpRetry.eventId = eventId
    jumpRetry.attempt = attempt + 1
    jumpRetry.restart()
  }
  /// Re-centre until the target is genuinely on screen.
  Timer {
    id: jumpSettle
    property string target: ""
    property int tries: 0
    interval: 60
    repeat: true
    running: false
    onTriggered: {
      var i = root.indexOfEvent(jumpSettle.target)
      if (i < 0 || jumpSettle.tries >= 12) { running = false; return }
      jumpSettle.tries++
      var it = list.itemAtIndex(i)
      if (it) {
        var top = it.y - list.contentY
        // Good enough once it is inside the middle half of the viewport.
        if (top > list.height * 0.15 && top + it.height < list.height * 0.9) { running = false; return }
      }
      list.positionViewAtIndex(i, ListView.Center)
    }
  }
  Timer {
    id: jumpRetry
    property string eventId: ""
    property int attempt: 0
    interval: 220
    onTriggered: root.scrollToEvent(jumpRetry.eventId, jumpRetry.attempt)
  }

  function focusInput() { input.forceActiveFocus() }
  // `@` for people, `#` for rooms. No `:` for emoji — the picker covers that.
  property string acKind: ""          // "" | "user" | "room"
  property string acQuery: ""
  property int acFrom: -1
  property var acItems: []
  property int acIndex: 0
  property bool acFetching: false
  /// `@::` attaches a contact card: a different action from an `@` mention, and
/// the longer match wins.
  property bool acContact: false
  property int acSeq: 0
  Timer {
    id: acDebounce
    interval: 200
    onTriggered: root.runDirectorySearch()
  }

  /// Wrap or unwrap the selection in markers. Without the toggle, Ctrl+B on bold
/// text adds a second pair and gives `****text****`.
  function wrapSelection(open, close) {
    close = close || open
    var a = input.selectionStart, b = input.selectionEnd
    if (a === b) {
      var t = input.text
      while (a > 0 && !/\s/.test(t.charAt(a - 1))) a--
      while (b < t.length && !/\s/.test(t.charAt(b))) b++
      if (a === b) {
        input.insert(input.cursorPosition, open + close)
        input.cursorPosition -= close.length
        return
      }
    }
    var sel = input.text.substring(a, b)
    var wrapped = sel.length >= open.length + close.length
                  && sel.substring(0, open.length) === open
                  && sel.substring(sel.length - close.length) === close
    var out = wrapped ? sel.substring(open.length, sel.length - close.length) : open + sel + close
    input.remove(a, b)
    input.insert(a, out)
    input.select(a, a + out.length)
  }

  /// A Sigil modifier around the selection: `red::selected;`.
  function applyModifier(mod) {
    var a = input.selectionStart, b = input.selectionEnd
    if (a === b) return
    var sel = input.text.substring(a, b)
    var out = mod + "::" + sel + ";"
    input.remove(a, b)
    input.insert(a, out)
    input.select(a, a + out.length)
  }

  function updateAutocomplete() {
    var t = input.text, pos = input.cursorPosition
    var i = pos - 1
    // Walk back to the trigger. A space ends the search: an address is one token.
    while (i >= 0 && !/\s/.test(t.charAt(i))) {
      var c = t.charAt(i)
      if (c === "@" || c === "#") {
        // Only at a word boundary, so an email address is left alone.
        if (i > 0 && !/\s/.test(t.charAt(i - 1))) break
        // `@::` is the contact picker; a bare `@` is still the mention list.
        var isContact = c === "@" && t.substring(i + 1, i + 3) === "::"
        root.acContact = isContact
        root.acKind = c === "@" ? "user" : "room"
        root.acFrom = i
        root.acQuery = t.substring(i + (isContact ? 3 : 1), pos)
        if (isContact) {
          // Debounced: the directory is a server round trip.
          acDebounce.restart()
          if (root.acItems.length === 0) root.runDirectorySearch()
        } else {
          root.refreshAutocomplete()
        }
        return
      }
      i--
    }
    root.acKind = ""
    root.acContact = false
    root.acItems = []
    acDebounce.stop()
  }

  /// Who can be addressed with `@::`.
  /// Room members first, then the homeserver's user directory. The local half
  /// matters: Synapse only indexes the directory for users who share a room with
  /// you or sit in a public room. An empty query asks for the roster.
  function runDirectorySearch() {
    if (!root.acContact || !root.svc) return
    var seq = ++root.acSeq
    root.svc.searchDirectory(root.acQuery, function (r, e) {
      // A slower earlier reply must not overwrite a newer one.
      if (seq !== root.acSeq || !root.acContact) return
      var out = []
      var users = (r && r.users) ? r.users : []
      for (var i = 0; i < users.length && out.length < 8; i++) {
        if (users[i].userId === root.selfId) continue
        out.push({
          label: users[i].displayName || users[i].userId,
          insert: users[i].userId,
          sub: users[i].userId,
          avatar: users[i].avatarPath || "",
          avatarUrl: users[i].avatarUrl || "",
          contact: true
        })
      }
      root.acItems = out
      root.acIndex = 0
    })
  }

  /// You are never a useful suggestion.
  readonly property string selfId: root.svc ? (root.svc.userId || "") : ""

  function refreshAutocomplete() {
    if (!root.svc) return
    var q = root.acQuery.toLowerCase()
    var out = []
    if (root.acKind === "user") {
      var members = root.svc.membersByRoom[root.roomId] || []
      // Members are fetched lazily, so the first `@` in a room finds nothing.
      if (members.length === 0 && !root.acFetching) {
        root.acFetching = true
        root.svc.fetchMembers(root.roomId, function () {
          root.acFetching = false
          if (root.acKind === "user") root.refreshAutocomplete()
        })
      }
      for (var i = 0; i < members.length && out.length < 6; i++) {
        var m = members[i]
        if (m.userId === root.selfId) continue
        var name = m.displayName || m.userId || ""
        if (q === "" || name.toLowerCase().indexOf(q) >= 0 || (m.userId || "").toLowerCase().indexOf(q) >= 0)
          out.push({ label: name, insert: name, sub: m.userId || "", avatar: m.avatarPath || "" })
      }
    } else if (root.acKind === "room") {
      var rooms = root.svc.rooms || []
      for (var r = 0; r < rooms.length && out.length < 6; r++) {
        var rm = rooms[r]
        if (rm.isSpace) continue
        // Linking a room to itself is not a thing anyone means to do.
        if (rm.id === root.roomId) continue
        var rn = rm.name || ""
        if (q === "" || rn.toLowerCase().indexOf(q) >= 0)
          out.push({ label: rn, insert: rm.canonicalAlias || rn, sub: rm.canonicalAlias || "", avatar: rm.avatarPath || "" })
      }
    }
    root.acItems = out
    root.acIndex = 0
  }

  function acceptAutocomplete(idx) {
    if (root.acFrom < 0 || root.acItems.length === 0) return
    var it = root.acItems[Math.max(0, Math.min(root.acItems.length - 1, idx))]
    // A contact is not text: the `@::query` leaves the composer and is staged.
    if (root.acContact) {
      input.remove(root.acFrom, input.cursorPosition)
      root.pendingContact = { userId: it.insert, displayName: it.label,
                              avatarUrl: it.avatarUrl || "", avatarPath: it.avatar || "" }
      root.acKind = ""
      root.acContact = false
      root.acItems = []
      acDebounce.stop()
      return
    }
    var text = (root.acKind === "user" ? "@" : "#") + it.insert + " "
    input.remove(root.acFrom, input.cursorPosition)
    input.insert(root.acFrom, text)
    root.acKind = ""
    root.acItems = []
  }

  /// A contact staged for sending. Cleared on send or cancel.
  property var pendingContact: null

  function setText(t) { input.text = t; input.cursorPosition = input.length }
  function textValue() { return input.text }
  function clearComposer() { root.pendingContact = null; input.text = ""; root.pendingFiles = []; root.replyTo = ""; root.editOf = ""; root.captionOf = ""; root.replyBody = ""; root.voicePath = ""; root.voiceDuration = 0; root.voiceWaveform = []; root.recorderOpen = false }
  property string replyTo: ""
  property string replyName: ""
  property string replyBody: ""
  property string editOf: ""
  property string captionOf: ""
  property bool recorderOpen: false
  property string voicePath: ""
  property real voiceDuration: 0
  property var voiceWaveform: []
  property bool voiceSending: false
  property bool clipPlaying: false
  property real clipPos: 0
  readonly property real clipFrac: root.voiceDuration > 0 ? Math.max(0, Math.min(1, root.clipPos / root.voiceDuration)) : 0
  Timer {
    id: clipTimer; interval: 100; repeat: true; running: root.clipPlaying
    onTriggered: {
      root.clipPos += 0.1
      if (root.clipPos >= root.voiceDuration) { root.clipPlaying = false; root.clipPos = 0 }
    }
  }

  readonly property var tl: (svc && roomId) ? svc.timelineFor(roomId) : null

  /// Thread roots in this room, keyed by root event id.
  /// From `threads.list`, not the timeline item: matrix-sdk-ui's `thread_summary`
  /// is None for every item against Synapse, while the server's bundled `m.thread`
  /// aggregation has the count and the latest reply. One request per room open.
  property var threadRoots: ({})
  function threadFor(eventId) { return eventId ? (root.threadRoots[eventId] || null) : null }
  function loadThreads() {
    if (!root.svc || !root.roomId || !root.isRoomView) return
    root.svc.listThreads(root.baseRoomId, function (r, e) {
      var m = {}
      var list = (r && r.threads) ? r.threads : []
      for (var i = 0; i < list.length; i++) m[list[i].rootId] = list[i]
      root.threadRoots = m
    })
  }
  Timer { id: threadsTimer; interval: 700; onTriggered: root.loadThreads() }
  // A reply in any thread refreshes the counts, debounced to one request.
  Connections {
    target: root.svc
    ignoreUnknownSignals: true
    function onThreadsChanged(roomId) {
      if (root.isRoomView && roomId === root.baseRoomId) threadsTimer.restart()
    }
  }

  /// Ask for every link preview in the room up front.
  /// Prefetching means a preview card is built at its final height the first time
  /// it is seen, instead of growing under the reader. `Service.linkPreview`
  /// dedupes and caches by URL.
  function prefetchLinkPreviews() {
    if (!root.svc || !root.tl || !root.tl.model) return
    var m = root.tl.model
    var seen = {}, asked = 0
    for (var i = 0; i < m.count && asked < 200; i++) {
      var it = m.get(i)
      if (!it || it.kind !== "text") continue
      var hit = String(it.body || "").match(/https?:\/\/[^\s<>"]+/)
      if (!hit || seen[hit[0]]) continue
      seen[hit[0]] = true
      asked++
      root.svc.linkPreview(hit[0])
    }
  }
  Timer {
    id: prefetchTimer
    interval: 600          // after the first paint, never competing with it
    onTriggered: root.prefetchLinkPreviews()
  }
  readonly property alias listContentY: list.contentY
  readonly property alias listShiftY: listShiftT.y
  Translate { id: listShiftT; y: 0 }
  readonly property Item listItem: list
  // The sheet translates the list, and a translation moves the clip rectangle
  // with it, so anything clipped at press time stays clipped. Unclip during it.
  // Hover tips go through the panel's in-card layer; the shared tooltip is a
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

  // The unread divider is a snapshot of entry; later arrivals must not add one.
  property bool unreadDividerAllowed: true
  property bool attachOpen: false     // attachment sheet under the composer
  // Only one composer sheet at a time, whichever route opened it.
  onAttachOpenChanged: if (root.attachOpen) root.recorderOpen = false
  onRecorderOpenChanged: if (root.recorderOpen) root.attachOpen = false
  // Files chosen but not sent yet: sent with the typed text as their caption.
  property var pendingFiles: []
  function addAttachments(paths) {
    var out = root.pendingFiles.slice()
    for (var i = 0; i < paths.length; i++) if (out.indexOf(paths[i]) < 0) out.push(paths[i])
    root.pendingFiles = out
    root.attachOpen = false
    Qt.callLater(root.focusInput)
  }
  function removeAttachment(path) {
    root.pendingFiles = root.pendingFiles.filter(function(p) { return p !== path })
  }
  function isImagePath(p) { return /\.(png|jpe?g|gif|webp|bmp|avif)$/i.test(String(p)) }
  function fileExt(p) { var n = root.baseName(p), i = n.lastIndexOf("."); return i > 0 ? n.slice(i + 1).toUpperCase() : "FILE" }
  /// What the staged bar calls the thing, in the same voice the timeline uses.
  function fileKind(p) {
    var e = root.fileExt(p).toLowerCase()
    if (root.isImagePath(p)) return e === "gif" ? "GIF" : "Image"
    if (/^(mp4|mkv|mov|webm|avi|m4v)$/.test(e)) return "Video"
    if (/^(mp3|m4a|flac|wav|ogg|opus|aac)$/.test(e)) return "Audio"
    if (e === "vcf") return "Contact"
    if (e === "pdf") return "PDF document"
    if (/^(docx?|odt|rtf)$/.test(e)) return "Document"
    if (/^(pptx?|odp)$/.test(e)) return "Presentation"
    if (/^(xlsx?|ods|csv)$/.test(e)) return "Spreadsheet"
    if (/^(zip|tar|gz|xz|7z|rar|zst)$/.test(e)) return "Archive"
    return e.toUpperCase() + " file"
  }
  function baseName(p) { var s = String(p); var i = s.lastIndexOf("/"); return i >= 0 ? s.slice(i + 1) : s }
  /// A one-line confirmation: saving a file out of sight has to say where it went.
  property string toast: ""
  Timer { id: toastTimer; interval: 2600; onTriggered: root.toast = "" }
  function note(t) { root.toast = t; toastTimer.restart() }

  property bool sheetOpen: false
  property bool pageSliding: false     // set while this page slides in or out
  readonly property string pagination: (svc && roomId && svc.paginationByRoom[roomId]) ? svc.paginationByRoom[roomId] : "idle"
  // Older messages arrive by pagination long after open, so keep asking as
  // batches land; a one-shot pass only covers the newest screenful.
  onPaginationChanged: if (root.pagination === "idle") { Qt.callLater(root.maybePaginate); prefetchTimer.restart() }
  /// Ask for older messages if the oldest loaded is near the viewport. Guarded,
  /// so it is safe to call on every frame of a scroll.
  function maybePaginate() {
    if (!root.svc || !root.roomId) return
    if (root.pagination !== "idle") return
    if (list.contentHeight <= list.height) return
    if (list.fromStart > list.height * 1.5) return
    root.svc.paginate(root.roomId)
  }

  // In-app dropdown (stays inside the card, unlike an xdg-popup)
  Item {
    anchors.fill: parent
    visible: root.menuOpen
    z: 30
    MouseArea { anchors.fill: parent; onClicked: root.menuOpen = false }
    Rectangle {
      x: parent.width - width - Style.space(10)
      y: Style.space(50)
      width: Style.space(170)
      height: menuCol2.implicitHeight + Style.space(12)
      radius: Style.space(14)
      antialiasing: true
      color: root.themed ? root.surfaceC : Util.alpha(Color.popups.background, 0.98)
      opacity: root.menuOpen ? 1 : 0
      scale: root.menuOpen ? 1 : 0.92
      transformOrigin: Item.TopRight
      Behavior on opacity { NumberAnimation { duration: 120 } }
      Behavior on scale { NumberAnimation { duration: 120; easing.type: Easing.OutCubic } }
      Column {
        id: menuCol2
        anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
        anchors.margins: Style.space(6)
        Repeater {
          model: [ { t: "Search", a: "search", icon: Icons.search },
                   { t: "Threads", a: "threads", icon: Icons.thread },
                   { t: "Pins", a: "pins", icon: Icons.pin },
                   { t: "Chat theme", a: "chattheme", icon: Icons.palette },
                   { t: "Settings", a: "roomsettings", icon: Icons.settings } ]
          delegate: Rectangle {
            required property var modelData
            width: parent.width; height: Style.space(34); radius: Style.space(9)
            color: cmh.containsMouse ? Util.alpha(root.fg, 0.08) : "transparent"
            IconLabel { anchors.verticalCenter: parent.verticalCenter; anchors.left: parent.left; anchors.leftMargin: Style.space(12); icon: modelData.icon; color: root.fg; opacity: 0.85; filled: true; size: Style.font.icon }
            Text { anchors.verticalCenter: parent.verticalCenter; anchors.left: parent.left; anchors.leftMargin: Style.space(38); text: modelData.t; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body }
            MouseArea { id: cmh; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: { root.menuOpen = false; root.navRequested(modelData.a) } }
          }
        }
      }
    }
  }

  // Mask for the timeline: same rounded top corners as the container below it.
  Item {
    id: listMask
    x: root.listItem ? root.listItem.x : 0
    y: root.listItem ? root.listItem.y : 0
    width: root.listItem ? root.listItem.width : 0
    height: root.listItem ? root.listItem.height : 0
    visible: false
    layer.enabled: true
    layer.smooth: true
    Rectangle {
      anchors.fill: parent
      topLeftRadius: Style.space(24); topRightRadius: Style.space(24)
      antialiasing: true
      color: "black"
    }
  }

  // Conversation container: rounded top corners, fading from the chrome tone.
  Rectangle {
    anchors.fill: parent
    anchors.topMargin: Style.space(52)
    topLeftRadius: Style.space(24); topRightRadius: Style.space(24)
    bottomLeftRadius: Style.space(22); bottomRightRadius: Style.space(22)
    antialiasing: true
    color: Util.alpha(root.convoC, 0.94)
    // Per-chat wallpaper (photo or gradient preset), rounded to the container
    Item {
      anchors.fill: parent
      visible: (root.chatTheme.wallpaper || "") !== ""
      layer.enabled: true
      layer.smooth: true
      layer.effect: MultiEffect { maskEnabled: true; maskThresholdMin: 0.5; maskSpreadAtMin: 1.0; maskSource: convoMask }
      Image {
        anchors.fill: parent
        visible: (root.chatTheme.wallpaper || "").indexOf("grad:") !== 0 && (root.chatTheme.wallpaper || "") !== ""
        fillMode: Image.PreserveAspectCrop
        asynchronous: true
        source: visible ? "file://" + root.chatTheme.wallpaper : ""
      }
      Rectangle {
        id: wallGrad
        anchors.fill: parent
        visible: (root.chatTheme.wallpaper || "").indexOf("grad:") === 0
        readonly property var gcols: root.themeGradPair(Math.min(8, Math.max(0, parseInt((root.chatTheme.wallpaper || "grad:0").substring(5)) || 0)))
        gradient: Gradient {
          GradientStop { position: 0; color: wallGrad.gcols[0] }
          GradientStop { position: 1; color: wallGrad.gcols[1] }
        }
      }
      Rectangle { anchors.fill: parent; visible: (root.chatTheme.wallpaper || "").indexOf("grad:") !== 0; color: Util.alpha("#000000", 0.45) }
    }
    Item {
      id: convoMask
      anchors.fill: parent
      layer.enabled: true
      layer.smooth: true
      visible: false
      Rectangle {
        anchors.fill: parent
        topLeftRadius: Style.space(24); topRightRadius: Style.space(24)
        bottomLeftRadius: Style.space(22); bottomRightRadius: Style.space(22)
        antialiasing: true
        color: "black"
      }
    }
    Rectangle {
      anchors.top: parent.top; anchors.left: parent.left; anchors.right: parent.right
      height: Style.space(46)
      topLeftRadius: Style.space(24); topRightRadius: Style.space(24)
      antialiasing: true
      gradient: Gradient {
        GradientStop { position: 0; color: Util.alpha(Qt.lighter(Color.menu.background, 1.35), 0.3) }
        GradientStop { position: 1; color: "transparent" }
      }
    }
  }

  Column {
    anchors.fill: parent
    spacing: 0

    // Header (z above the list: it may render unclipped under the sheet)
    Item {
      z: 2
      width: parent.width; height: Style.space(52)
      Rectangle {
        anchors.fill: parent
        visible: root.themed
        topLeftRadius: Style.space(22); topRightRadius: Style.space(22)
        antialiasing: true
        color: root.chromeC
      }
      PanelActionButton { id: backBtn; anchors.left: parent.left; anchors.leftMargin: Style.space(6); anchors.verticalCenter: parent.verticalCenter; fontFamily: Fonts.iconFilled; iconText: Icons.back; foreground: root.fg; onClicked: root.backRequested() }
      Avatar { id: hav; anchors.left: backBtn.right; anchors.leftMargin: Style.space(4); anchors.verticalCenter: parent.verticalCenter; size: Style.space(32); source: root.room ? (root.room.avatarPath || "") : ""; name: root.room ? root.room.name : ""; userId: root.room ? (root.room.isDm ? (root.room.dmUserId || root.room.id) : root.room.id) : ""
        status: (root.room && root.room.isDm && root.svc) ? root.svc.presenceOf(root.room.dmUserId || "") : ""
        statusBackdrop: root.themed ? root.chromeC : Qt.lighter(Color.menu.background, 1.35) }
      Column {
        anchors.left: hav.right; anchors.leftMargin: Style.space(10); anchors.right: callBtns.left; anchors.rightMargin: Style.space(4); anchors.verticalCenter: parent.verticalCenter
        Text {
          width: parent.width; elide: Text.ElideRight
          text: root.viewKind === "thread" ? "Thread"
              : (root.room ? (root.room.name || root.room.id) : "")
          color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.subtitle; font.bold: true
        }
        Text {
          width: parent.width; elide: Text.ElideRight
          readonly property var typing: (root.svc && root.svc.typingByRoom[root.roomId]) ? root.svc.typingByRoom[root.roomId] : []
          visible: text !== ""
          // In a thread or the pins list the subtitle says which room you are in.
          text: !root.isRoomView ? (root.room ? (root.room.name || root.room.id) : "")
              : typing.length > 0 ? (typing.length === 1 ? typing[0].displayName + " is typing…" : "Several people are typing…")
              : (root.room && !root.room.isDm ? root.room.joinedMembers + " members" : "")
          color: typing.length > 0 ? Color.accent : Util.alpha(root.fg, 0.5); font.family: Fonts.ui; font.pixelSize: Style.font.caption
        }
      }
      Row {
        id: callBtns
        anchors.right: parent.right; anchors.rightMargin: Style.space(8); anchors.verticalCenter: parent.verticalCenter
        spacing: 0
        // Calls belong to the room; a thread or the pins list is only a slice of one.
        visible: root.isRoomView
        // A live call in this room takes the place of the two start-a-call buttons.
        Rectangle {
          visible: root.callHere
          anchors.verticalCenter: parent.verticalCenter
          width: visible ? joinRow.implicitWidth + Style.space(22) : 0
          height: Style.space(28)
          radius: height / 2
          color: Util.alpha(root.themed ? root.accC : Color.accent, 0.9)
          Row {
            id: joinRow
            anchors.centerIn: parent
            spacing: Style.space(5)
            IconLabel { icon: Icons.phone; color: "#141414"; anchors.verticalCenter: parent.verticalCenter; filled: true; size: Style.font.caption }
            Text { text: "Join"; color: "#141414"; font.family: Fonts.ui; font.pixelSize: Style.font.caption; font.bold: true; anchors.verticalCenter: parent.verticalCenter }
          }
          MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.joinCallRequested() }
        }
        PanelActionButton { anchors.verticalCenter: parent.verticalCenter; visible: !root.callHere; fontFamily: Fonts.iconFilled; iconText: Icons.phone; foreground: root.fg; id: voiceBtn; tooltipText: ""; enabled: root.svc && !root.svc.inCall; onClicked: root.startCall(false)
          HoverHandler { onHoveredChanged: root.showTip(voiceBtn, hovered, "Voice call") } }
        PanelActionButton { anchors.verticalCenter: parent.verticalCenter; visible: !root.callHere; fontFamily: Fonts.iconFilled; iconText: Icons.videoOn; foreground: root.fg; id: videoBtn; tooltipText: ""; enabled: root.svc && !root.svc.inCall; onClicked: root.startCall(true)
          HoverHandler { onHoveredChanged: root.showTip(videoBtn, hovered, "Video call") } }
        PanelActionButton {
          id: dotsBtn
          visible: root.isRoomView
          width: visible ? implicitWidth : 0
          anchors.verticalCenter: parent.verticalCenter
          fontFamily: Fonts.iconFilled; iconText: Icons.moreVertical; foreground: root.fg; onClicked: root.menuOpen = true
        }
      }
    }

    // Invite banner
    Rectangle {
      width: parent.width
      visible: root.room && root.room.isInvite
      height: visible ? Style.space(52) : 0
      color: Util.alpha(Color.accent, 0.1)
      Row {
        anchors.centerIn: parent; spacing: Style.space(10)
        Button { text: "Accept invite"; foreground: root.fg; bordered: true; onClicked: root.svc.joinRoom(root.roomId) }
        Button { text: "Decline"; foreground: Color.urgent; onClicked: { root.svc.leaveRoom(root.roomId); root.backRequested() } }
      }
    }

    // Timeline (index 0 = newest; laid out bottom-to-top)
    //
    // A plain Item so the frosted popup can sample something other than the
    // ListView: a ShaderEffectSource on a ListView disturbs its own painting.
    Item {
      id: timelineBox
      width: parent.width
      // the recorder panel takes space from the timeline, not from the composer
      height: parent.height - y - composerBox.height - recorderFrame.height - attachFrame.height - typing.height

    ListView {
      id: list
      anchors.fill: parent
      clip: true
      verticalLayoutDirection: ListView.BottomToTop
      model: root.tl ? root.tl.model : null
      spacing: 0
      // ListView sizes the unbuilt part from the average of the part it HAS built,
      // and these items run 39px to 333px. With a small window that average is
      // unrepresentative, so realising items moves originY and everything with it.
      cacheBuffer: Math.max(Style.space(800), height * root.warmSteps)
      // reuseItems is OFF. Permanently: a pooled delegate reused after a room
      // change showed the PREVIOUS room's messages, with stale full-width geometry.
      boundsBehavior: Flickable.StopAtBounds
      // The sheet shifts the CONTENT inside this static clip, never the clip itself.
      Component.onCompleted: contentItem.transform = listShiftT
      // The clip rectangle is square: mask the list to the container's rounded corners.
      // Off while the message sheet is up: the blurred snapshot hides the corners.
      layer.enabled: !root.sheetOpen && !root.pageSliding
      layer.smooth: true
      layer.effect: MultiEffect { maskEnabled: true; maskThresholdMin: 0.5; maskSpreadAtMin: 1.0; maskSource: listMask }
      QQC.ScrollBar.vertical: ScrollBarStyle { id: chatScrollBar }

      // The panel is a layer surface, so the compositor's scroll_factor multiplies
      // every delta. Deltas feed a target that contentY chases at a capped speed.
      /// Pixels still to travel. See pixelBy.
      property real wheelRemaining: 0

      Timer { id: barFade; interval: 700; onTriggered: chatScrollBar.forceActive = false }

      FrameAnimation {
        id: wheelDrive
        running: false
        onTriggered: {
          var d = list.wheelRemaining
          if (Math.abs(d) < 0.5) { list.wheelRemaining = 0; running = false; list.returnToBounds(); return }
          // Frame-rate independent ease (~90% of the gap per 130 ms).
          var dt = Math.min(0.05, Math.max(0.001, frameTime))
          var step = d * (1 - Math.pow(0.1, dt / 0.13))
          var cap = root.wheelMaxSpeed * dt
          if (step > cap) step = cap
          else if (step < -cap) step = -cap
          // Bounds are read fresh every frame: delegates realise and history lands.
          var lo = Math.min(list.originY, list.contentY)
          var hi = Math.max(lo, list.originY + list.contentHeight - list.height, list.contentY)
          var want = list.contentY + step
          var got = Math.max(lo, Math.min(hi, want))
          list.contentY = got
          list.wheelRemaining -= step
          // Ran into an end: drop the rest rather than letting it queue and fire later.
          if (Math.abs(got - want) > 0.001) { list.wheelRemaining = 0; running = false; list.returnToBounds() }
        }
      }

      // Move by `angle` wheel units (120 = one classic notch before scaling).
      function wheelByAngle(angle) {
        if (angle === 0) return
        list.pixelBy(angle / 120 * root.wheelStep)
      }

      // Queue `px` of travel (positive = scrolling down through the timeline).
      //
      // Travel is a REMAINING DISTANCE, not an absolute contentY: ListView
      // re-estimation and landing history both move the content under a fixed target.
      function pixelBy(px) {
        list.cancelFlick()
        list.wheelRemaining -= px
        // One spin should not queue an unbounded journey.
        var lead = list.height * root.wheelLeadScreens
        if (list.wheelRemaining > lead) list.wheelRemaining = lead
        else if (list.wheelRemaining < -lead) list.wheelRemaining = -lead
        wheelDrive.running = true
        chatScrollBar.forceActive = true
        barFade.restart()
      }

      readonly property bool atBottom: atYEnd || contentHeight <= height
      // Distance from the end, in pixels: the jump button waits for a real gap.
      readonly property real fromEnd: Math.max(0, (originY + Math.max(0, contentHeight - height)) - contentY)
      // Unseen history above the viewport: in a BottomToTop list, the way down to originY.
      readonly property real fromStart: Math.max(0, contentY - originY)
      onAtBottomChanged: if (atBottom) root.maybeMarkRead()
      onMovementStarted: { root.jumping = false; wheelDrive.running = false }
      // Fetched a screen and a half early, so the round trip is hidden.
      onFromStartChanged: root.maybePaginate()
      onCountChanged: Qt.callLater(root.maybePaginate)

      // Realising a batch corrects ListView's estimate and moves `originY` with the
      // item count unchanged. Do NOT correct for that delta, and keep the deliberate
      // absence of an `onOriginYChanged` handler: every attempt measured worse.
      header: Item { width: 1; height: Style.space(8) }
      delegate: BubbleDelegate {
        svc: root.svc; fg: root.fg; roomId: root.roomId
        autoDetails: root.debugDetailsAll
        page: root
        onOpenThreadRequested: function (rootId) { if (rootId !== "") root.openThreadRequested(rootId) }
        latestOwnId: root.latestOwnId
        receiptEventId: root.receiptEventId
        receiptReaders: root.receiptReaders
        animateMarks: root.animateMarks
        unreadDividerAllowed: root.unreadDividerAllowed
        dm: root.room ? !!root.room.isDm : false
        encrypted: root.room ? !!root.room.isEncrypted : false
        themeAccent: root.chatTheme.accent || ""
        playingVoice: root.playingVoice
        voicePos: root.voicePos
        onVoiceToggled: function(it) { root.toggleVoice(it) }
        onVoiceSeeked: function(it, pos) { root.playVoiceAt(it, pos) }
        themeSurface: root.themed ? root.surfaceC : "transparent"
        receiptGround: Qt.rgba(root.convoC.r, root.convoC.g, root.convoC.b, 1)
        lastReadTs: root.lastReadTs
        width: list.width
        onOpenLocation: function(it, from) { root.openLocation(it, from) }
        onOpenAudio: function(it) { root.openAudio(it) }
        onOpenDmWith: function(uid) { root.openDmWith(uid) }
        onShareVcf: function(uid, name) { root.shareVcf(uid, name) }
        onOpenDocument: function(it) { root.openDocument(it) }
        onOpenImage: function(it, from) { root.openImage(it, from) }
        onPlayVideo: function(it, from) { root.playVideo(it, from) }
        onMenuRequested: function(it, x, y, w, h, b) { root.menuRequested(it, x, y, w, h, b) }
        onReplyRequested: function(e, n, b) { root.replyTo = e; root.replyName = n; root.replyBody = b || ""; root.editOf = ""; root.focusInput() }
      }
      footer: Item {
        width: list.width; height: root.pagination === "paginating" ? Style.space(34) : Style.space(8)
        Spinner { anchors.centerIn: parent; visible: root.pagination === "paginating"; color: Util.alpha(root.fg, 0.5); size: Style.font.body }
      }
    }
    }

    // Typing indicator: same column as the composer, so the timeline yields space.
    TypingIndicator {
      id: typing
      width: parent.width
      typers: root.debugTypers ? root.debugTypers : ((root.svc && root.roomId && root.svc.typingByRoom[root.roomId]) ? root.svc.typingByRoom[root.roomId] : [])
      fg: root.fg
      surface: root.themed ? root.surfaceC : Color.popups.background
      // Only chase the bottom if already there and once height settles.
      onHeightChanged: if (list.atBottom) Qt.callLater(list.positionViewAtBeginning)
    }


    // Composer
    Item {
      id: composerBox
      z: 2
      width: parent.width
      height: inputPill.height + Style.space(18)
      visible: !(root.room && root.room.isInvite)
      // A child of `composerBox`, a plain Item: the page Column forbids anchoring.
      Rectangle {
        id: toastPill
        z: 40
        visible: opacity > 0.01
        opacity: root.toast !== "" ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: 180; easing.type: Easing.OutCubic } }
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottom: parent.top
        anchors.bottomMargin: Style.space(10)
        width: Math.min(composerBox.width - Style.space(40), toastText.implicitWidth + Style.space(28))
        height: Style.space(34)
        radius: height / 2
        antialiasing: true
        color: Qt.rgba(0, 0, 0, 0.82)
        Text {
          id: toastText
          anchors.centerIn: parent
          width: parent.width - Style.space(24)
          horizontalAlignment: Text.AlignHCenter
          elide: Text.ElideMiddle
          text: root.toast
          color: "#f2f2f2"
          font.family: Fonts.ui; font.pixelSize: Style.font.caption
        }
      }

        // A child of the composer container, not the page Column, which forbids anchoring.
        Item {
        id: acPopup
        visible: root.acItems.length > 0
        z: 30
        anchors.left: parent.left; anchors.right: parent.right
        // Flush with the top of the *footer*, spanning its full width, top corners only.
        anchors.bottom: parent.top
        anchors.bottomMargin: 0
        anchors.leftMargin: 0; anchors.rightMargin: 0
        height: visible ? acCol.implicitHeight + Style.space(12) : 0

        // Frosted: a translucent tint the compositor's blur sits behind. Do NOT point a
        // ShaderEffectSource at the live ListView — it renders as vertical smears.
        Item {
          id: frostMask
          anchors.fill: parent
          visible: false
          layer.enabled: true
          Rectangle {
            anchors.fill: parent
            topLeftRadius: Style.space(20); topRightRadius: Style.space(20)
            antialiasing: true
            color: "black"
          }
        }
        Item {
          anchors.fill: parent
          layer.enabled: true
          layer.smooth: true
          layer.effect: MultiEffect {
            maskEnabled: true
            maskSource: frostMask
            maskThresholdMin: 0.5
            maskSpreadAtMin: 1.0
          }
          // Opaque: over a themed room, in-app blur reads as a smear rather than glass.
          Rectangle { anchors.fill: parent; color: Qt.rgba(root.convoC.r, root.convoC.g, root.convoC.b, 1) }
          Rectangle {
            anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
            height: Style.space(30)
            gradient: Gradient {
              GradientStop { position: 0; color: Util.alpha("#ffffff", 0.07) }
              GradientStop { position: 1; color: "transparent" }
            }
          }
          Rectangle {
            anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
            height: Style.space(28)
            gradient: Gradient {
              GradientStop { position: 0; color: Util.alpha("#ffffff", 0.05) }
              GradientStop { position: 1; color: "transparent" }
            }
          }
        }

        Column {
          id: acCol
          anchors.left: parent.left; anchors.right: parent.right
          anchors.top: parent.top; anchors.margins: Style.space(6)
          anchors.bottomMargin: Style.space(14)
          Repeater {
            model: root.acItems
            delegate: Rectangle {
              required property var modelData
              required property int index
              width: acCol.width
              height: Style.space(38)
              radius: Style.space(11)
              color: index === root.acIndex ? Util.alpha(root.fg, 0.13) : "transparent"
              Avatar {
                id: acFace
                anchors.left: parent.left; anchors.leftMargin: Style.space(8)
                anchors.verticalCenter: parent.verticalCenter
                size: Style.space(24)
                source: modelData.avatar || ""
                name: modelData.label || ""
                userId: modelData.sub || ""
              }
              Column {
                anchors.left: acFace.right; anchors.leftMargin: Style.space(8)
                anchors.right: parent.right; anchors.rightMargin: Style.space(8)
                anchors.verticalCenter: parent.verticalCenter
                Text {
                  width: parent.width; elide: Text.ElideRight
                  text: modelData.label
                  color: root.fg
                  font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
                }
                Text {
                  width: parent.width; elide: Text.ElideRight
                  visible: text !== "" && text !== modelData.label
                  text: modelData.sub || ""
                  color: Util.alpha(root.fg, 0.5)
                  font.family: Fonts.ui; font.pixelSize: Style.font.caption
                }
              }
              MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                onClicked: root.acceptAutocomplete(index)
              }
            }
          }
        }
      }

      Rectangle {
        anchors.fill: parent
        bottomLeftRadius: Style.space(22); bottomRightRadius: Style.space(22)
        antialiasing: true
        color: Util.alpha(root.convoC, 0.55)
      }


      Row {
        id: footerRow
        anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom
        anchors.leftMargin: Style.space(10); anchors.rightMargin: Style.space(10); anchors.bottomMargin: Style.space(10)
        spacing: Style.space(8)

        Rectangle {
          id: inputPill
          width: parent.width - sendBtn.width - Style.space(8)
          readonly property int fileShown: Math.min(3, root.pendingFiles.length)
          readonly property bool hasContact: !!root.pendingContact
          readonly property real quoteH: inputPill.hasContact
            ? Style.space(10) + Style.space(40)
            : ((root.pendingFiles.length > 0)
              ? Style.space(10) + inputPill.fileShown * Style.space(40) + (inputPill.fileShown - 1) * Style.space(4)
                + (root.pendingFiles.length > inputPill.fileShown ? Style.space(18) : 0)
              : ((root.voicePath !== "") ? Style.space(60) : ((root.replyTo !== "" || root.editOf !== "" || root.captionOf !== "") ? Style.space(52) : 0)))
          // The text area's implicitHeight already includes its padding.
          height: Math.min(Style.space(110), Math.max(Style.space(38), input.implicitHeight)) + quoteH
          radius: Style.space(19)
          color: root.themed ? root.surfaceC : Util.alpha(root.fg, 0.07)
          // A staged contact reads as a staged attachment: the reply quote's inset bar.
          Rectangle {
            id: contactChip
            z: 6
            visible: inputPill.hasContact
            anchors.top: parent.top; anchors.left: parent.left; anchors.right: parent.right
            anchors.margins: Style.space(5)
            height: Style.space(40)
            radius: Style.space(13)
            antialiasing: true
            color: Util.alpha(root.convoC, 0.92)
            Avatar {
              id: chipFace
              anchors.left: parent.left; anchors.leftMargin: Style.space(6)
              anchors.verticalCenter: parent.verticalCenter
              size: Style.space(28)
              source: root.pendingContact ? (root.pendingContact.avatarPath || "") : ""
              name: root.pendingContact ? (root.pendingContact.displayName || "") : ""
              userId: root.pendingContact ? (root.pendingContact.userId || "") : ""
            }
            Column {
              anchors.left: chipFace.right; anchors.leftMargin: Style.space(9)
              anchors.right: chipX.left; anchors.rightMargin: Style.space(6)
              anchors.verticalCenter: parent.verticalCenter
              spacing: Style.space(1)
              Text {
                width: parent.width; elide: Text.ElideRight
                text: root.pendingContact ? (root.pendingContact.displayName || root.pendingContact.userId) : ""
                color: root.fg
                font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall; font.bold: true
              }
              Text {
                width: parent.width; elide: Text.ElideMiddle
                text: "Contact"
                color: Util.alpha(root.fg, 0.6)
                font.family: Fonts.ui; font.pixelSize: Style.font.caption
              }
            }
            Item {
              id: chipX
              width: Style.space(24); height: Style.space(24)
              anchors.right: parent.right; anchors.rightMargin: Style.space(7)
              anchors.verticalCenter: parent.verticalCenter
              Rectangle { anchors.fill: parent; radius: width / 2; color: cch.containsMouse ? Util.alpha(root.fg, 0.12) : "transparent" }
              IconLabel { anchors.centerIn: parent; icon: Icons.close; color: root.fg; size: Style.font.bodySmall }
              MouseArea { id: cch; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: root.pendingContact = null }
            }
          }

          // Staged attachments use the reply quote's inset bar, one per file.
          Column {
            id: fileChips
            z: 6
            visible: root.pendingFiles.length > 0 && !inputPill.hasContact
            anchors.top: parent.top; anchors.left: parent.left; anchors.right: parent.right
            anchors.margins: Style.space(5)
            spacing: Style.space(4)
            Repeater {
              // Past three the bar would out-grow the message; the rest are counted.
              model: root.pendingFiles.slice(0, inputPill.fileShown)
              delegate: Rectangle {
                required property var modelData
                width: fileChips.width
                height: Style.space(40)
                radius: Style.space(13)
                antialiasing: true
                color: Util.alpha(root.convoC, 0.92)

                // A rounded mask, not `clip`, which is rectangular and would square the corners.
                Rectangle {
                  id: thumbBox
                  anchors.left: parent.left; anchors.leftMargin: Style.space(6)
                  anchors.verticalCenter: parent.verticalCenter
                  width: Style.space(28); height: width
                  radius: Style.space(9)
                  antialiasing: true
                  color: root.themed ? Qt.darker(root.surfaceC, 1.25) : Util.alpha(root.fg, 0.14)
                  Image {
                    anchors.fill: parent
                    visible: root.isImagePath(modelData)
                    source: root.isImagePath(modelData) ? "file://" + modelData : ""
                    fillMode: Image.PreserveAspectCrop
                    asynchronous: true
                    cache: true
                    layer.enabled: true
                    layer.smooth: true
                    layer.effect: MultiEffect {
                      maskEnabled: true
                      maskThresholdMin: 0.5
                      maskSpreadAtMin: 1.0
                      maskSource: thumbMask
                    }
                  }
                  Rectangle {
                    id: thumbMask
                    anchors.fill: parent
                    radius: thumbBox.radius
                    antialiasing: true
                    color: "black"
                    visible: false
                    layer.enabled: true
                    layer.smooth: true
                  }
                  Text {
                    anchors.centerIn: parent
                    visible: !root.isImagePath(modelData)
                    text: root.fileExt(modelData)
                    color: Util.alpha(root.fg, 0.8)
                    font.family: Fonts.ui; font.pixelSize: Style.space(8); font.bold: true
                  }
                }

                Column {
                  anchors.left: thumbBox.right; anchors.leftMargin: Style.space(9)
                  anchors.right: fileClose.left; anchors.rightMargin: Style.space(6)
                  anchors.verticalCenter: parent.verticalCenter
                  spacing: Style.space(1)
                  Text {
                    width: parent.width; elide: Text.ElideMiddle
                    text: root.baseName(modelData)
                    color: root.fg
                    font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall; font.bold: true
                  }
                  Text {
                    width: parent.width; elide: Text.ElideRight
                    text: root.fileKind(modelData)
                    color: Util.alpha(root.fg, 0.6)
                    font.family: Fonts.ui; font.pixelSize: Style.font.caption
                  }
                }

                Item {
                  id: fileClose
                  width: Style.space(24); height: Style.space(24)
                  anchors.right: parent.right; anchors.rightMargin: Style.space(7)
                  anchors.verticalCenter: parent.verticalCenter
                  Rectangle { anchors.fill: parent; radius: width / 2; color: fch.containsMouse ? Util.alpha(root.fg, 0.12) : "transparent" }
                  IconLabel { anchors.centerIn: parent; icon: Icons.close; color: root.fg; size: Style.font.bodySmall }
                  MouseArea { id: fch; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: root.removeAttachment(modelData) }
                }
              }
            }
            Text {
              visible: root.pendingFiles.length > inputPill.fileShown
              width: fileChips.width
              horizontalAlignment: Text.AlignHCenter
              text: "+" + (root.pendingFiles.length - inputPill.fileShown) + " more"
              color: Util.alpha(root.fg, 0.55)
              font.family: Fonts.ui; font.pixelSize: Style.font.caption
            }
          }

          // Attached voice clip: one tidy row inside the input surface
          Rectangle {
            id: clipChip
            z: 5
            visible: root.voicePath !== ""
            // A LIGHT card inset in the darker pill, so the pill reads as padding.
            anchors.top: parent.top; anchors.left: parent.left; anchors.right: parent.right
            anchors.margins: Style.space(6)
            height: Style.space(48)
            radius: Style.space(14)
            antialiasing: true
            readonly property color ink: "#17181a"
            color: Qt.lighter(root.themed ? root.accC : Color.accent, 1.35)
            // A hairline in the pill's colour covers the seam where two antialiased
            // rounded rects meet.
            Rectangle {
              id: clipPlay
              anchors.left: parent.left; anchors.leftMargin: Style.space(7); anchors.verticalCenter: parent.verticalCenter
              width: Style.space(32); height: Style.space(32); radius: width / 2
              color: Util.alpha("#ffffff", 0.9)
              IconLabel { anchors.centerIn: parent; icon: root.clipPlaying ? Icons.pause : Icons.play; color: "#1a1a1a"; size: Style.font.icon }
              MouseArea {
                anchors.fill: parent; cursorShape: Qt.PointingHandCursor
                onClicked: {
                  if (!root.svc || root.voicePath === "") return
                  if (root.clipPlaying) { root.svc.stopAudio(); root.clipPlaying = false; root.clipPos = 0; return }
                  root.clipPos = 0
                  root.clipPlaying = true
                  root.svc.playAudioFile(root.voicePath, function(r, e) { if (e) root.clipPlaying = false })
                }
              }
            }
            Item {
              id: clipWave
              anchors.left: clipPlay.right; anchors.leftMargin: Style.space(10)
              anchors.right: clipDur.left; anchors.rightMargin: Style.space(8)
              anchors.verticalCenter: parent.verticalCenter
              height: Style.space(26)
              readonly property var bars: root.resampleWave(root.voiceWaveform, Math.max(8, Math.floor(width / Style.space(5))))
              Row {
                id: clipRow
                height: parent.height
                spacing: Style.space(2)
                Repeater {
                  model: clipWave.bars
                  delegate: Item {
                    required property var modelData
                    required property int index
                    width: Style.space(3); height: clipRow.height
                    Rectangle {
                      width: parent.width
                      height: Math.max(Style.space(4), Math.round(Style.space(22) * Math.min(1, modelData)))
                      // exact centring: round the *height* first, then the offset
                      y: (parent.height - height) / 2
                      radius: width / 2
                      color: (index / Math.max(1, clipWave.bars.length)) <= root.clipFrac
                             ? Util.alpha(clipChip.ink, 0.95) : Util.alpha(clipChip.ink, 0.4)
                    }
                  }
                }
              }
              MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                onClicked: function(m) {
                  if (!root.svc || root.voicePath === "" || root.voiceDuration <= 0) return
                  var t = (m.x / width) * root.voiceDuration
                  root.clipPos = t
                  root.clipPlaying = true
                  root.svc.playAudioFileAt(root.voicePath, t, function(r, e) { if (e) root.clipPlaying = false })
                }
              }
            }
            Text {
              id: clipDur
              anchors.right: clipClose.left; anchors.rightMargin: Style.space(8); anchors.verticalCenter: parent.verticalCenter
              text: root.fmtDur(root.clipPlaying ? Math.max(0, root.voiceDuration - root.clipPos) : root.voiceDuration); color: Util.alpha(clipChip.ink, 0.85)
              font.family: Fonts.ui; font.pixelSize: Style.font.caption
            }
            Item {
              id: clipClose
              z: 6
              width: Style.space(26); height: Style.space(26)
              anchors.right: parent.right; anchors.rightMargin: Style.space(8); anchors.verticalCenter: parent.verticalCenter
              Rectangle { anchors.fill: parent; radius: width / 2; color: Util.alpha(clipChip.ink, 0.12) }
              IconLabel { anchors.centerIn: parent; icon: Icons.close; color: clipChip.ink; size: Style.font.bodySmall }
              MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: { if (root.clipPlaying && root.svc) root.svc.stopAudio(); root.clipPlaying = false; root.voicePath = ""; root.voiceDuration = 0; root.voiceWaveform = [] } }
            }
          }

          // GM: the reply quote lives INSIDE the input surface
          Rectangle {
            // never behind the voice chip: its dark edge peeked out as a ring
            visible: inputPill.quoteH > 0 && root.voicePath === "" && root.pendingFiles.length === 0 && !inputPill.hasContact
            anchors.top: parent.top; anchors.left: parent.left; anchors.right: parent.right
            anchors.margins: Style.space(5)
            height: inputPill.quoteH - Style.space(9)
            radius: Style.space(15)
            antialiasing: true
            color: Util.alpha(root.convoC, 0.92)
            Column {
              anchors.left: parent.left; anchors.leftMargin: Style.space(12)
              anchors.right: quoteClose.left; anchors.rightMargin: Style.space(6)
              anchors.verticalCenter: parent.verticalCenter
              spacing: Style.space(1)
              Text { width: parent.width; elide: Text.ElideRight; text: root.captionOf !== "" ? (root.replyBody !== "" ? "Edit caption" : "Add caption") : (root.editOf !== "" ? "Edit message" : root.replyName); color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall; font.bold: true }
              Text { width: parent.width; elide: Text.ElideRight; visible: text !== ""; text: root.replyBody; color: Util.alpha(root.fg, 0.6); font.family: Fonts.ui; font.pixelSize: Style.font.caption }
            }
            Item {
              id: quoteClose
              width: Style.space(26); height: Style.space(26)
              anchors.right: parent.right; anchors.rightMargin: Style.space(8); anchors.verticalCenter: parent.verticalCenter
              Rectangle { anchors.fill: parent; radius: width / 2; color: qch.containsMouse ? Util.alpha(root.fg, 0.12) : "transparent" }
              IconLabel { anchors.centerIn: parent; icon: Icons.close; color: root.fg; size: Style.font.bodySmall }
              MouseArea { id: qch; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: { root.replyTo = ""; root.editOf = ""; root.captionOf = ""; root.replyBody = ""; input.text = "" } }
            }
          }
          Item {
            id: attach
            anchors.left: parent.left; anchors.leftMargin: Style.space(6)
            anchors.bottom: parent.bottom
            anchors.bottomMargin: Math.max(Style.space(3), (Style.space(38) - height) / 2)
            width: Style.space(30); height: Style.space(30)

            readonly property bool on: root.attachOpen

            // plus-circle-outline ↔ close-circle, spun into each other.
            Text {
              anchors.centerIn: parent
              text: attach.on ? Icons.cancel : Icons.plusCircle
              // Fixed light grey: reads on the composer pill in every theme.
              color: attachHover.hovered || attach.on ? "#c6c6c6" : Util.alpha("#c6c6c6", 0.85)
              // Outlined while it invites (add), solid once it means stop (cancel).
              font.family: attach.on ? Fonts.iconFilled : Fonts.icon
              font.pixelSize: Style.font.iconLarge
              rotation: attach.on ? 90 : 0
              Behavior on rotation { NumberAnimation { duration: 180; easing.type: Easing.OutCubic } }
              Behavior on color { ColorAnimation { duration: 140 } }
            }

            HoverHandler { id: attachHover; cursorShape: Qt.PointingHandCursor }
            MouseArea {
              anchors.fill: parent
              cursorShape: Qt.PointingHandCursor
              onClicked: { root.attachOpen = !root.attachOpen; if (root.attachOpen) { root.recorderOpen = false; attachSheet.reset() } }
            }
          }
          FontMetrics { id: inputMetrics; font: input.font }

          QQC.ScrollView {
            id: inputScroll
            anchors.left: attach.right; anchors.right: parent.right; anchors.top: parent.top; anchors.bottom: parent.bottom
            anchors.leftMargin: Style.space(2); anchors.rightMargin: Style.space(10)
            anchors.topMargin: inputPill.quoteH
            clip: true
            QQC.ScrollBar.horizontal.policy: QQC.ScrollBar.AlwaysOff
            QQC.TextArea {
              id: input
              color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body
              wrapMode: TextEdit.Wrap
              placeholderText: root.voicePath !== "" ? "Add text" : "Message"
              placeholderTextColor: Util.alpha(root.fg, 0.45)
              background: Item {}
              // Let the text item centre its own line: an empty document reports almost no
              // contentHeight, which misplaces padding computed from it.
              verticalAlignment: TextEdit.AlignVCenter
              height: Math.max(implicitHeight, inputScroll.height)
              topPadding: Style.space(3)
              bottomPadding: Style.space(3)
              selectionColor: Util.alpha(Color.accent, 0.35); selectedTextColor: root.fg
              QQC.ContextMenu.menu: null
              TextContextMenu { id: composerCtxMenu; editor: parent }
              Keys.onPressed: function(e) {
                // The composer keeps focus under a layered page, so it must not eat its Escape.
                if (root.covered) { e.accepted = false; return }
                // The autocomplete owns the arrows and Tab while it is up.
                if (root.acItems.length > 0) {
                  if (e.key === Qt.Key_Down) { root.acIndex = (root.acIndex + 1) % root.acItems.length; e.accepted = true; return }
                  if (e.key === Qt.Key_Up) { root.acIndex = (root.acIndex + root.acItems.length - 1) % root.acItems.length; e.accepted = true; return }
                  if (e.key === Qt.Key_Tab || e.key === Qt.Key_Return || e.key === Qt.Key_Enter) {
                    root.acceptAutocomplete(root.acIndex); e.accepted = true; return
                  }
                  if (e.key === Qt.Key_Escape) { root.acKind = ""; root.acItems = []; e.accepted = true; return }
                }
                // Strike and code are Shift'd: Ctrl+X is cut and Ctrl+M is Return.
                if (e.modifiers & Qt.ControlModifier) {
                  var shift = (e.modifiers & Qt.ShiftModifier) !== 0
                  if (e.key === Qt.Key_B && !shift) { root.wrapSelection("**"); e.accepted = true; return }
                  if (e.key === Qt.Key_I && !shift) { root.wrapSelection("*"); e.accepted = true; return }
                  if (e.key === Qt.Key_U && !shift) { root.applyModifier("underline"); e.accepted = true; return }
                  if (e.key === Qt.Key_X && shift) { root.wrapSelection("~~"); e.accepted = true; return }
                  if (e.key === Qt.Key_M && shift) { root.wrapSelection("`"); e.accepted = true; return }
                }
                if ((e.key === Qt.Key_Return || e.key === Qt.Key_Enter) && !(e.modifiers & Qt.ShiftModifier)) { root.send(); e.accepted = true; return }
                if (e.key === Qt.Key_Escape) {
                  if (root.menuOpen) root.menuOpen = false
                  else if (root.sheetOpen) root.closeSheetRequested()
                  else if (root.replyTo || root.editOf) { root.replyTo = ""; root.editOf = ""; input.text = "" }
                  else root.backRequested()
                  e.accepted = true
                }
              }
              onTextChanged: {
                if (text !== "" && root.svc && root.roomId) root.svc.setTyping(root.roomId, true)
                root.updateAutocomplete()
              }
              onCursorPositionChanged: root.updateAutocomplete()
            }
          }
        }
        Rectangle {
          id: sendBtn
          width: Style.space(38); height: Style.space(38); radius: height / 2
          anchors.bottom: parent.bottom
          // Empty composer with nothing staged -> voice button; otherwise send.
          readonly property bool sendMode: input.text.trim() !== "" || root.voicePath !== ""
                                           || root.pendingFiles.length > 0 || !!root.pendingContact
          color: root.themedSend
          Text {
            anchors.centerIn: parent
            visible: opacity > 0.01
            opacity: parent.sendMode ? 1 : 0
            scale: parent.sendMode ? 1 : 0.55
            rotation: parent.sendMode ? 0 : -35
            Behavior on opacity { NumberAnimation { duration: 130 } }
            Behavior on scale { NumberAnimation { duration: 180; easing.type: Easing.OutBack; easing.overshoot: 2.0 } }
            Behavior on rotation { NumberAnimation { duration: 180; easing.type: Easing.OutCubic } }
            text: Icons.send
            color: Color.background
            font.family: Fonts.iconFilled; font.pixelSize: Style.font.iconLarge
          }
          Text {
            anchors.centerIn: parent
            visible: opacity > 0.01
            opacity: parent.sendMode ? 0 : 1
            scale: parent.sendMode ? 0.55 : 1
            Behavior on opacity { NumberAnimation { duration: 130 } }
            Behavior on scale { NumberAnimation { duration: 180; easing.type: Easing.OutBack; easing.overshoot: 2.0 } }
            text: Icons.voiceMemo
            color: Color.background
            font.family: Fonts.iconFilled; font.pixelSize: Style.font.iconLarge
          }
          MouseArea {
            anchors.fill: parent; cursorShape: Qt.PointingHandCursor
            onClicked: {
              if (parent.sendMode) root.send()
              else { root.recorderOpen = !root.recorderOpen; if (root.recorderOpen) { root.attachOpen = false; Qt.callLater(recorder.reset) } }
            }
          }
          Behavior on color { ColorAnimation { duration: 120 } }
        }
      }
    }

    // The clipping frame animates its height while the sheet keeps its natural
    // size; animating the sheet's own height squashes its contents.
    Item {
      id: attachFrame
      width: parent.width
      clip: true
      visible: height > 1 && !(root.room && root.room.isInvite)
      height: root.attachOpen ? attachSheet.implicitHeight : 0
      Behavior on height { NumberAnimation { duration: 220; easing.type: Easing.OutCubic } }
      onHeightChanged: if (list.atBottom || root.attachOpen) list.positionViewAtBeginning()

      AttachMenu {
        id: attachSheet
        width: parent.width
        height: implicitHeight
        anchors.top: parent.top
        svc: root.svc; roomId: root.roomId
        // The theme accent proper, not the darker send variant, for marks on dark discs.
        fg: root.fg; accent: root.themed ? root.accC : Color.accent
        surface: root.themed ? root.surfaceC : Util.alpha(root.fg, 0.07)
        // Controls inside the sheet use the composer reply chip's tone.
        chip: root.chipC
        deepChip: root.deepChipC
        onPickFiles: root.attachRequested()
        onInsertEmoji: function(ch) { input.insert(input.cursorPosition, ch); input.forceActiveFocus() }
        onCloseRequested: root.attachOpen = false
      }
    }

    Item {
      id: recorderFrame
      width: parent.width
      clip: true
      visible: height > 1 && !(root.room && root.room.isInvite)
      height: root.recorderOpen ? recorder.implicitHeight : 0
      Behavior on height { NumberAnimation { duration: 220; easing.type: Easing.OutCubic } }
      onHeightChanged: if (list.atBottom || root.recorderOpen) list.positionViewAtBeginning()

      VoiceRecorder {
        id: recorder
        width: parent.width
        height: implicitHeight
        anchors.top: parent.top
        svc: root.svc; fg: root.fg; accent: root.themedSend
        surface: root.themed ? root.surfaceC : Util.alpha(root.fg, 0.07)
        // Cancel/Attach need to read against the sheet's own tint.
        chip: root.deepChipC
        onAttached: function(path, dur, wave) {
          root.voicePath = path; root.voiceDuration = dur; root.voiceWaveform = wave
          root.recorderOpen = false
          root.focusInput()
        }
        onCancelled: root.recorderOpen = false
      }
    }

  }

  // Jump-to-latest: the list grows as delegates instantiate, so ease toward the
  // *current* end, re-evaluated every frame, then snap to index 0.
  property bool jumping: false
  FrameAnimation {
    running: root.jumping
    onTriggered: {
      var target = list.originY + Math.max(0, list.contentHeight - list.height)
      var d = target - list.contentY
      if (Math.abs(d) < 0.5) { list.contentY = target; root.jumping = false; list.returnToBounds(); return }
      // frame-rate independent: ~88% of the gap closed every 0.25 s
      var k = 1 - Math.pow(0.0001, Math.min(0.05, frameTime))
      list.contentY += d * k
    }
  }
  function debugJumpReport() { console.log("JUMP atYEnd", list.atYEnd, "contentY", Math.round(list.contentY), "target", Math.round(list.originY + Math.max(0, list.contentHeight - list.height))) }
  function jumpToLatest() { root.jumping = true }


  Rectangle {
    id: jumpBtn
    readonly property bool shown: list.count > 0 && list.fromEnd > list.height * 0.75
    visible: scale > 0.01
    anchors.horizontalCenter: parent.horizontalCenter
    anchors.bottom: parent.bottom; anchors.bottomMargin: composerBox.height + recorderFrame.height + attachFrame.height + Style.space(10)
    width: Style.space(38); height: Style.space(38); radius: height / 2
    antialiasing: true
    color: root.themed ? root.surfaceC : Color.popups.background
    scale: shown ? 1 : 0
    opacity: shown ? 1 : 0
    Behavior on scale { NumberAnimation { duration: 220; easing.type: Easing.OutBack; easing.overshoot: 2.2 } }
    Behavior on opacity { NumberAnimation { duration: 140 } }
    IconLabel { anchors.centerIn: parent; icon: Icons.arrowDown; color: root.fg; size: Style.font.icon }
    MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.jumpToLatest() }
  }

  function send() {
    var t = input.text.trim()
    // A staged contact goes first and on its own: the card is the message.
    if (root.pendingContact && root.svc && root.roomId) {
      root.svc.sendContact(root.roomId, root.pendingContact.userId,
                           root.pendingContact.displayName,
                           root.pendingContact.avatarUrl || "", function (r, e) {})
      root.pendingContact = null
      if (t === "") { root.clearComposer(); root.jumpToLatest(); return }
    }
    if (root.pendingFiles.length > 0 && root.svc && root.roomId) {
      // The typed text rides along as the caption on the first file.
      var files = root.pendingFiles.slice()
      root.svc.sendFiles(root.roomId, files, t)
      root.clearComposer()
      root.jumpToLatest()
      return
    }
    if (root.voicePath !== "" && root.svc && root.roomId) {
      if (root.voiceSending) return          // one send per clip
      root.voiceSending = true
      root.svc.voiceSend(root.roomId, root.voicePath, root.voiceDuration, root.voiceWaveform, t,
                         function(r, e) { root.voiceSending = false })
      root.clearComposer()
      root.jumpToLatest()
      return
    }
    if (root.captionOf !== "" && root.svc && root.roomId) {
      root.svc.editCaption(root.roomId, root.captionOf, t)
      root.clearComposer()
      return
    }
    if (t === "" || !root.svc || !root.roomId) return
    var opts = {}
    if (root.editOf) opts.editOf = root.editOf
    else if (root.replyTo) opts.replyTo = root.replyTo
    root.svc.sendText(root.roomId, t, opts)
    root.clearComposer()
    root.svc.setTyping(root.roomId, false)
    list.positionViewAtBeginning()
  }

  // A receipt attaches only to the newest event read; older ones are implicit.
  property real lastReadTs: 0
  function recomputeLastRead() {
    if (!root.tl) { root.lastReadTs = 0; return }
    var m = root.tl.model, best = 0
    var lim = Math.min(m.count, 120)
    for (var i = 0; i < lim; i++) {
      var it = m.get(i)
      if (it.readCount > 0 && it.ts > best) best = it.ts
    }
    root.lastReadTs = best
  }

  // A translation, not a scroll, so the pressed bubble reaches the spotlight.
  NumberAnimation { id: shiftAnim; target: listShiftT; property: "y"; duration: 180; easing.type: Easing.OutCubic }
  function shiftList(dy) {
    shiftAnim.stop()
    shiftAnim.from = listShiftT.y
    shiftAnim.to = listShiftT.y + dy
    shiftAnim.start()
  }

  // Test hook: press an image bubble; "edge" pushes it half past the viewport.
  function debugPressSending() {
    var kids = list.contentItem.children
    for (var i = 0; i < kids.length; i++) {
      var d = kids[i]
      if (d && d.model !== undefined && d.model && d.model.sendState === "sending" && typeof d.pressMenu === "function") { d.pressMenu(); return }
    }
    console.log("no sending delegate")
  }

  function debugPressFailed() {
    var kids = list.contentItem.children
    for (var i = 0; i < kids.length; i++) {
      var d = kids[i]
      if (d && d.model !== undefined && d.model && d.model.sendState === "failed" && typeof d.pressMenu === "function") { d.pressMenu(); return }
    }
    console.log("no failed delegate")
  }

  function debugPressImage(mode) {
    var kids = list.contentItem.children
    for (var i = 0; i < kids.length; i++) {
      var d = kids[i]
      var wantKind = (mode === "text") ? "text" : "image"
      var wantOwn = (mode === "ownimage")
      if (wantOwn) wantKind = "image"
      if (d && d.model !== undefined && d.model && d.model.kind === wantKind && (!wantOwn || d.model.isOwn) && typeof d.pressMenu === "function") {
        if (mode === "edge") { list.contentY = d.y - list.height + Style.space(60) }
        Qt.callLater(function() { d.pressMenu() })
        return
      }
    }
    console.log("debugPressImage: no image delegate instantiated")
  }

  Timer { id: readTimer; interval: 800; repeat: false; onTriggered: if (root.svc && root.roomId) root.svc.markRead(root.roomId) }
  function maybeMarkRead() { if (root.visibleToUser && list.atBottom && root.roomId) readTimer.restart() }
  // Reopening restores the old contentY, then delegates resize: re-pin to newest.
  Timer {
    id: pinBottom
    interval: 60; repeat: true; property int ticks: 0
    onTriggered: {
      ticks++
      list.positionViewAtBeginning()
      list.returnToBounds()
      if (ticks > 8) { stop(); ticks = 0 }
    }
  }
  function pinToLatest() { pinBottom.ticks = 0; pinBottom.restart() }

  onVisibleToUserChanged: {
    // Leaving the chat closes the composer sheets.
    if (!visibleToUser) { root.attachOpen = false; root.unreadDividerAllowed = true; return }
    dividerHold.restart()
    maybeMarkRead()
    root.pinToLatest()
  }
  onRoomIdChanged: { root.entryAnimAllowed = false; root.animatedIds = ({}); entrySettle.restart();
                     root.knownReaders = ({});
                     root.animateMarks = false; markSettle.restart(); root.recomputeLastRead(); root.pinToLatest(); Qt.callLater(root.recomputeLatestOwn); root.attachOpen = false; root.recorderOpen = false;
                     root.restartWarm(); prefetchTimer.restart(); root.threadRoots = ({}); threadsTimer.restart();
                     }

  Connections {
    target: root.svc
    function onTimelineChanged(rid, ops) {
      if (rid !== root.roomId) return
      root.recomputeLastRead()
      for (var i = 0; i < ops.length; i++) if (ops[i].op === "pushBack" && ops[i].item && ops[i].item.isOwn) { Qt.callLater(function() { list.positionViewAtBeginning() }); break }
      root.maybeMarkRead()
    }
    function onTimelineReset(rid) { if (rid === root.roomId) { root.recomputeLastRead(); root.pinToLatest() } }
  }

  // A wheel handler inside the ListView never fires (Qt binds it to the content
  // item, shadowed by the Flickable). NoButton leaves clicks and drags alone.
  MouseArea {
    anchors.fill: parent
    z: 30
    acceptedButtons: Qt.NoButton
    onWheel: function(w) {
      // Let the wheel through when the pointer is over a sheet's own scroller.
      for (var i = 0; i < 2; i++) {
        var sheet = i === 0 ? attachSheet : recorder
        if (sheet.height > 1) {
          var pt = root.mapToItem(sheet, w.x, w.y)
          if (pt.x >= 0 && pt.y >= 0 && pt.x <= sheet.width && pt.y <= sheet.height) { w.accepted = false; return }
        }
      }
      var ang = w.angleDelta.y
      var px = w.pixelDelta ? w.pixelDelta.y : 0
      if (w.inverted) { ang = -ang; px = -px }
      if (ang !== 0) root.listItem.wheelByAngle(ang)
      else if (px !== 0) root.listItem.pixelBy(px)
      w.accepted = true
    }
  }
}
