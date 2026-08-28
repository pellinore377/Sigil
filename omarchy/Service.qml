import QtQuick
import Quickshell
import Quickshell.Io

// Sigil's Omarchy frontend. Talks to sigil-engine over a unix socket speaking
// JSON Lines, holds the live state the UI renders, and relays actions back.
// Spawns the engine if nothing is listening, and respawns it with a backoff.
Item {
  id: root

  // Injected by shell.qml.
  property var shell: null
  property var manifest: null
  property var pluginRegistry: null

  readonly property string pluginId: "pellinore.sigil"
  readonly property string home: Quickshell.env("HOME")
  readonly property string engineBin: home + "/.local/bin/sigil-engine"
  readonly property string pluginDir:
    Qt.resolvedUrl(".").toString().replace(/^file:\/\//, "").replace(/\/$/, "")
  readonly property bool noSpawn: Quickshell.env("SIGIL_NO_SPAWN") === "1"
  // Must mirror paths::socket_path() in engine/src/paths.rs.
  readonly property string socketPath: {
    var o = Quickshell.env("SIGIL_SOCKET")
    if (o && o !== "") return o
    var rt = Quickshell.env("XDG_RUNTIME_DIR")
    if (rt && rt !== "") return rt + "/sigil.sock"
    return home + "/.local/state/sigil/sigil.sock"
  }

  // connection
  property bool connected: false
  property bool engineMissing: false
  property bool engineSpawned: false
  property string engineError: ""
  property string engineVersion: ""
  property int protocol: 0
  property bool engineSetupRunning: false
  property string engineSetupError: ""

  // auth
  property string authState: "unknown"     // loggedOut | loginPending | restoring | loggedIn
  property string homeserver: ""
  property string serverName: ""
  property string userId: ""
  property string deviceId: ""
  property string displayName: ""
  property string avatarPath: ""
  property string ssoUrl: ""
  property string authError: ""
  property bool verified: false
  property string syncState: "offline"
  property string syncError: ""
  property string recoveryState: "unknown"  // unknown | enabled | disabled | incomplete
  property string backupState: "unknown"
  property bool recoverySkipped: false
  property string recoveryError: ""
  readonly property bool loggedIn: authState === "loggedIn"
  readonly property bool needsRecovery: loggedIn && !recoverySkipped && (recoveryState === "incomplete" || (recoveryState === "disabled" && !verified))

  // rooms
  property bool roomsLoaded: false
  property var rooms: []            // room summaries (engine schema), sorted by the engine
  property var roomsById: ({})
  property int roomsRevision: 0
  property var spaces: []           // [{id,name,avatarPath,level,children[]}]
  property string selectedSpaceId: ""
  property string selectedRoomId: ""
  property string pendingOpenRoomId: ""
  property var typingByRoom: ({})
  property var receiptsByRoom: ({})   // roomId -> [{userId, displayName}]
  property var membersByRoom: ({})  // roomId -> [members]
  // userId -> {state, busy, lastActiveAgo}. "busy" is derived by the engine from live call membership.
  property var presenceByUser: ({})
  /// "online" | "away" | "busy" | "offline" | "" when we simply do not know.
  function presenceOf(userId) {
    if (!userId) return ""
    var p = root.presenceByUser[userId]
    if (!p) return ""
    if (p.busy) return "busy"
    return p.state || ""
  }
  property var paginationByRoom: ({}) // roomId -> "idle"|"paginating"|"timelineStart"

  readonly property int unreadTotal: {
    var n = 0
    for (var i = 0; i < rooms.length; i++) n += Math.max(rooms[i].unread || 0, rooms[i].unreadMessages || 0)
    return n
  }
  readonly property int highlightTotal: {
    var n = 0
    for (var i = 0; i < rooms.length; i++) n += (rooms[i].highlights || 0)
    return n
  }

  // Timeline models: roomId -> { model: ListModel (index 0 = newest), revision }
  property var timelines: ({})
  signal timelineReset(string roomId)
  signal timelineChanged(string roomId, var ops)
  signal mediaReady(var info)
  signal notifyEvent(var info)
  signal loginFinished()
  signal loginFailed(string message)

  // calls
  property var call: ({ state: "idle", roomId: "", participants: [], local: {}, incoming: null, error: "" })
  property var devices: ({ mics: [], speakers: [], cameras: [], selected: {} })
  readonly property bool inCall: call.state === "connected" || call.state === "joining" || call.state === "reconnecting"

  // engine
  Loader {
    id: sockLoader
    active: false
    sourceComponent: Socket {
      path: root.socketPath
      parser: SplitParser {
        onRead: function(line) { root.handleLine(line) }
      }
      onConnectionStateChanged: {
        root.connected = connected
        if (connected) {
          root.reconnectDelay = 1500
          root.engineError = ""
        } else {
          root.onDisconnected()
          root.scheduleReconnect()
        }
      }
      onError: function(err) { root.scheduleReconnect() }
    }
    onLoaded: item.connected = true
  }

  function attemptConnect() {
    sockLoader.active = false
    sockLoader.active = true
  }

  property int reconnectDelay: 1500

  function scheduleReconnect() {
    root.connected = false
    if (!root.noSpawn && !engineProc.running && !root.engineMissing) {
      root.engineSpawned = true
      engineProc.running = true
    }
    if (!reconnect.running) {
      reconnect.interval = root.reconnectDelay
      root.reconnectDelay = Math.min(root.reconnectDelay * 2, 30000)
      reconnect.start()
    }
  }

  Timer {
    id: reconnect
    repeat: false
    onTriggered: root.attemptConnect()
  }

  Process {
    id: engineProc
    command: ["sh", "-c",
      'if [ -x "$1" ]; then exec "$1" daemon --socket "$2"; else exec sigil-engine daemon --socket "$2"; fi',
      "sigil-engine-launch", root.engineBin, root.socketPath]
    stderr: StdioCollector {
      onStreamFinished: {
        var lines = String(text || "").trim().split("\n")
        if (lines.length && lines[lines.length - 1] !== "")
          root.engineError = lines[lines.length - 1]
      }
    }
    onExited: function(code) {
      if (code === 127) root.engineMissing = true
      if (!reconnect.running) {
        reconnect.interval = root.reconnectDelay
        root.reconnectDelay = Math.min(root.reconnectDelay * 2, 30000)
        reconnect.start()
      }
    }
  }

  Process {
    id: engineSetup
    command: ["sh", "-c", 'exec "$1/bin/sigil-setup"', "sigil-setup-launch", root.pluginDir]
    stderr: StdioCollector {
      onStreamFinished: {
        var lines = String(text || "").trim().split("\n")
        for (var i = 0; i < lines.length; i++) {
          if (lines[i].trim() !== "") { root.engineSetupError = lines[i]; break }
        }
      }
    }
    onExited: function(code) {
      root.engineSetupRunning = false
      if (code === 0) {
        root.engineMissing = false
        root.engineSetupError = ""
        root.reconnectDelay = 1500
        root.scheduleReconnect()
      }
    }
  }

  function installEngine() {
    if (root.engineSetupRunning) return
    root.engineSetupError = ""
    root.engineSetupRunning = true
    engineSetup.running = true
  }

  function onDisconnected() {
  }

  // requests
  property int _nextId: 0
  property var _pending: ({})

  // request(name, params, cb(result, error)) -> id or -1
  function request(name, params, cb) {
    var s = sockLoader.item
    if (!s || !root.connected) {
      if (cb) cb(null, { code: "disconnected", message: "engine not connected" })
      return -1
    }
    var id = ++root._nextId
    var obj = { req: name, id: id }
    if (params) for (var k in params) obj[k] = params[k]
    root._pending[id] = { cb: cb || null, name: name, sentAt: Date.now() }
    s.write(JSON.stringify(obj) + "\n")
    s.flush()
    return id
  }

  Timer {
    interval: 5000; running: true; repeat: true
    onTriggered: {
      var now = Date.now()
      for (var id in root._pending) {
        var p = root._pending[id]
        if (now - p.sentAt > 30000) {
          delete root._pending[id]
          if (p.cb) p.cb(null, { code: "timeout", message: p.name + " timed out" })
        }
      }
    }
  }

  function handleLine(line) {
    var msg
    try { msg = JSON.parse(line) } catch (e) { return }
    if (msg.reply !== undefined) {
      var p = root._pending[msg.reply]
      if (p) {
        delete root._pending[msg.reply]
        if (p.cb) {
          if (msg.ok) p.cb(msg.result, null)
          else p.cb(null, msg.error || { code: "error", message: "unknown error" })
        }
      }
      return
    }
    if (msg.event) root.handleEvent(msg.event, msg)
  }

  function handleEvent(ev, m) {
    switch (ev) {
    case "hello":
      root.engineVersion = m.engine || ""
      root.protocol = m.protocol || 0
      break
    case "status":
      // A restored session never sends `login.finished`, so contacts load on first running sync.
      if (m.sync === "running" && !root._contactsLoaded) { root._contactsLoaded = true; root.loadContacts() }
      root.authState = m.session || "unknown"
      root.homeserver = m.homeserver || ""
      root.serverName = m.serverName || ""
      root.userId = m.userId || ""
      root.deviceId = m.deviceId || ""
      root.displayName = m.displayName || ""
      root.avatarPath = m.avatarPath || ""
      root.syncState = m.sync || "offline"
      root.syncError = m.syncError || ""
      root.verified = !!m.verified
      root.ssoUrl = (m.login && m.login.url) ? m.login.url : ""
      root.authError = m.lastError || ""
      root.mapStyleUrl = m.mapStyleUrl || ""
      if (root.authState !== "loggedIn") {
        root.rooms = []; root.roomsById = ({}); root.roomsLoaded = false; root.spaces = []
        root.timelines = ({}); root.selectedRoomId = ""
      }
      break
    case "login.finished":
      root.authError = ""
      root.loadContacts()
      root.loginFinished()
      break
    case "login.failed":
      root.authError = (m.error && m.error.message) ? m.error.message : "login failed"
      root.loginFailed(root.authError)
      break
    case "recovery.status":
      root.recoveryState = m.recovery || "unknown"
      root.backupState = m.backup || "unknown"
      if (m.verified !== undefined) root.verified = !!m.verified
      break
    case "rooms.list":
      root.applyRooms(m.rooms || [], !!m.loaded)
      break
    case "spaces.tree":
      root.spaces = m.spaces || []
      break
    case "room.receipts": {
      var rc = Object.assign({}, root.receiptsByRoom)
      rc[m.roomId] = m.users || []
      root.receiptsByRoom = rc
      break
    }
    case "position":
      root.positionKnown = !!m.known
      root.positionLat = m.lat || 0
      root.positionLon = m.lon || 0
      root.positionAccuracy = m.accuracy || 0
      root.positionError = m.error || ""
      break
    case "location.live":
      root.liveSharing = !!m.sharing
      root.liveRoomId = m.roomId || ""
      root.liveUntil = m.until || 0
      break
    case "presence.list":
      root.presenceByUser = m.users || ({})
      break
    case "call.reaction": {
      root.callReaction(m.emoji || "", m.displayName || "", !!m.own)
      return
    }
    case "room.pinned": {
      // Pushed on every m.room.pinned_events change, here or elsewhere, so the badge stays fresh.
      var pm = Object.assign({}, root.pinnedByRoom)
      pm[m.roomId] = m.events || []
      root.pinnedByRoom = pm
      return
    }
    case "room.typing": {
      var t = Object.assign({}, root.typingByRoom)
      t[m.roomId] = m.users || []
      root.typingByRoom = t
      break
    }
    case "timeline.reset":
      root.resetTimeline(m.roomId, m.items || [])
      break
    case "timeline.diff":
      root.applyTimelineDiff(m.roomId, m.ops || [], m.len)
      root.noteThreadActivity(m.roomId, m.ops || [])
      break
    case "timeline.paginationState": {
      var ps = Object.assign({}, root.paginationByRoom)
      ps[m.roomId] = m.state
      root.paginationByRoom = ps
      break
    }
    case "media.ready":
      root.mediaReady(m)
      root.applyMediaReady(m)
      break
    case "notify":
      root.notifyEvent(m)
      break
    case "call.state":
      root.call = m
      break
    case "call.incoming":
      root.call = Object.assign({}, root.call, { incoming: m })
      break
    case "voice.level":
      root.voiceLevel = m.level || 0
      return
    case "call.devices":
      root.devices = m
      break
    default:
      break
    }
  }

  function applyRooms(list, loaded) {
    var byId = {}
    for (var i = 0; i < list.length; i++) byId[list[i].id] = list[i]
    root.roomsById = byId
    root.rooms = list
    root.roomsLoaded = loaded
    root.roomsRevision++
    if (root.pendingOpenRoomId !== "" && byId[root.pendingOpenRoomId]) {
      root.selectRoom(root.pendingOpenRoomId)
      root.pendingOpenRoomId = ""
    }
  }

  function room(id) { return root.roomsById[id] || null }

  function selectRoom(id) {
    if (!id) return
    root.selectedRoomId = id
    root.openRoom(id)
  }

  /// Make sure a model exists for a view key before anything streams into it.
  function ensureTimeline(key) {
    if (root.timelines[key]) return
    var t = Object.assign({}, root.timelines)
    t[key] = { model: Qt.createQmlObject("import QtQuick; ListModel { dynamicRoles: true }", root), revision: 0, ready: false }
    root.timelines = t
  }
  function openRoom(id) {
    root.ensureTimeline(id)
    root.loadPinned(id)
    root.request("room.open", { roomId: id, initialItems: 80 }, function(r, e) {
      if (e) console.warn("room.open failed:", e.message)
    })
  }

  function timelineFor(id) {
    if (!id) return null
    return root.timelines[id] || null
  }

  // Engine items are oldest-first; the ListModel is newest-first (index 0 =
  // newest) so a BottomToTop ListView stays stable while paginating.
  function resetTimeline(roomId, items) {
    var t = root.timelineFor(roomId)
    if (!t) {
      var tl = Object.assign({}, root.timelines)
      tl[roomId] = { model: Qt.createQmlObject("import QtQuick; ListModel { dynamicRoles: true }", root), revision: 0, ready: false }
      root.timelines = tl
      t = tl[roomId]
    }
    var m = t.model
    m.clear()
    for (var i = items.length - 1; i >= 0; i--) m.append(root.decorate(items[i]))
    root.recomputeGrouping(m, 0, m.count - 1)
    t.ready = true
    t.revision++
    root.timelineReset(roomId)
  }

  property var _lastDiffLen: undefined
  function applyTimelineDiff(roomId, ops, len) {
    root._lastDiffLen = len
    var t = root.timelineFor(roomId)
    if (!t) return
    var m = t.model
    var touched = []
    for (var i = 0; i < ops.length; i++) {
      var op = ops[i]
      var n = m.count
      switch (op.op) {
      case "append": {
        for (var j = 0; j < op.items.length; j++) m.insert(0, root.decorate(op.items[j]))
        touched.push(0, op.items.length)
        break
      }
      case "clear": m.clear(); break
      case "pushFront": m.append(root.decorate(op.item)); touched.push(m.count - 1); break
      case "pushBack": m.insert(0, root.decorate(op.item)); touched.push(0, 1); break
      case "popFront": if (n > 0) m.remove(n - 1); break
      case "popBack": if (n > 0) m.remove(0); break
      case "insert": { var idx = n - op.index; m.insert(idx, root.decorate(op.item)); touched.push(idx); break }
      case "set": { var si = n - 1 - op.index; if (si >= 0 && si < n) { m.set(si, root.decorate(op.item)); touched.push(si) } break }
      case "remove": { var ri = n - 1 - op.index; if (ri >= 0 && ri < n) m.remove(ri); touched.push(ri); break }
      case "truncate": { var keep = op.len; if (n > keep) m.remove(0, n - keep); touched.push(0); break }
      case "reset": {
        m.clear()
        for (var k = op.items.length - 1; k >= 0; k--) m.append(root.decorate(op.items[k]))
        touched.push(0, m.count - 1)
        break
      }
      }
    }
    // The engine sends its vector length per batch; if ours drifted, index-based ops land wrong.
    if (ops.length && m.count !== undefined) {
      var expected = root._lastDiffLen
      if (expected !== undefined && expected >= 0 && expected !== m.count) {
        var t2 = []
        for (var q = 0; q < ops.length; q++) t2.push(ops[q].op + (ops[q].index !== undefined ? "@" + ops[q].index : ""))
        console.log("sigil drift: model", m.count, "engine", expected, "| ops:", t2.join(","))
        // Index-based ops on a drifted model corrupt more rows each batch; reset instead.
        root.request("room.open", { roomId: roomId, initialItems: Math.max(60, m.count) })
      }
    }

    // Catch duplicated event ids right after the ops that caused them.
    {
      var seen = ({}), dupes = []
      for (var d = 0; d < m.count; d++) {
        var eid = m.get(d).eventId
        if (!eid) continue
        if (seen[eid] !== undefined) dupes.push(eid.slice(0, 10) + " at " + seen[eid] + "+" + d)
        else seen[eid] = d
      }
      if (dupes.length) {
        var trace = []
        for (var o = 0; o < ops.length; o++) trace.push(ops[o].op + (ops[o].index !== undefined ? "@" + ops[o].index : ""))
        console.log("sigil dup:", dupes.join(", "), "| ops:", trace.join(","), "| count:", m.count)
      }
    }

    if (touched.length) {
      var lo = Math.max(0, Math.min.apply(null, touched) - 2)
      var hi = Math.min(m.count - 1, Math.max.apply(null, touched) + 2)
      root.recomputeGrouping(m, lo, hi)
    }
    t.revision++
    root.timelineChanged(roomId, ops)
  }

  // Normalise an engine item for ListModel use (all roles present).
  function decorate(it) {
    var o = {
      id: it.id || "", kind: it.kind || "unsupported", eventId: it.eventId || "", txnId: it.txnId || "",
      sender: it.sender || "", senderName: it.senderName || "", senderAvatarPath: it.senderAvatarPath || "",
      senderAvatarUrl: it.senderAvatarUrl || "",
      ts: it.ts || 0, isOwn: !!it.isOwn, isHighlighted: !!it.isHighlighted,
      body: it.body || "", html: it.html || "", isEdited: !!it.isEdited,
      // Carried as JSON rather than as an array: a ListModel turns a nested array of
      // objects into something that is no longer a JS array, leaving the Repeater nothing.
      partsJson: it.parts ? JSON.stringify(it.parts) : "",
      contactJson: it.contact ? JSON.stringify(it.contact) : "",
      // Same JSON-string treatment, for the same ListModel reason.
      effectsJson: it.effects ? JSON.stringify(it.effects) : "",
      replyTo: it.replyTo || null, threadRoot: it.threadRoot || "",
      reactions: root.decorateReactions(it.reactions || []), media: it.media || null,
      // decorate() is a whitelist: a field it does not copy never reaches the delegate.
      poll: it.poll || null, location: it.location || null,
      liveShare: it.liveShare || null,
      sendState: it.sendState || "sent", sendError: it.sendError || "",
      readCount: (it.readBy || []).filter(function(r) { return r.userId !== root.userId }).length, utdReason: it.utdReason || "", stateText: it.stateText || "",
      can: it.can || { edit: false, reply: false, redact: false, react: false },
      showHeader: true, groupEnd: true, dayLabel: ""
    }
    return o
  }

  // Nested arrays of strings do not survive ListModel dynamic roles; flatten.
  function decorateReactions(list) {
    var out = []
    for (var i = 0; i < list.length; i++) {
      var r = list[i]
      var senders = r.senders || []
      out.push({ key: r.key, count: r.count || senders.length, mine: senders.indexOf(root.userId) >= 0, sendersText: senders.join(", ") })
    }
    return out
  }

  // Header/day grouping. Index 0 is newest; the older neighbour is index+1.
  function recomputeGrouping(m, lo, hi) {
    for (var i = Math.max(0, lo); i <= Math.min(m.count - 1, hi); i++) {
      var it = m.get(i)
      var older = (i + 1 < m.count) ? m.get(i + 1) : null
      var newer = (i - 1 >= 0) ? m.get(i - 1) : null
      var isMsg = root.isMessageKind(it.kind)
      var showHeader = true
      if (isMsg && older && root.isMessageKind(older.kind) && older.sender === it.sender && (it.ts - older.ts) < 5 * 60 * 1000 && root.sameDay(it.ts, older.ts))
        showHeader = false
      var groupEnd = true
      if (isMsg && newer && root.isMessageKind(newer.kind) && newer.sender === it.sender && (newer.ts - it.ts) < 5 * 60 * 1000 && root.sameDay(newer.ts, it.ts))
        groupEnd = false
      if (it.groupEnd !== groupEnd) m.setProperty(i, "groupEnd", groupEnd)
      var dayLabel = (it.ts > 0 && it.kind !== "dayDivider" && (!older || !root.sameDay(it.ts, older.ts) || (it.ts - older.ts) > 60 * 60 * 1000)) ? root.sessionLabelFor(it.ts) : ""
      if (it.showHeader !== showHeader) m.setProperty(i, "showHeader", showHeader)
      if (it.dayLabel !== dayLabel) m.setProperty(i, "dayLabel", dayLabel)
    }
  }

  function isMessageKind(k) {
    return k === "text" || k === "notice" || k === "emote" || k === "image" || k === "video" || k === "audio" || k === "voice" || k === "file" || k === "sticker" || k === "poll" || k === "utd" || k === "redacted" || k === "location" || k === "liveLocation"
  }

  function sameDay(a, b) {
    var da = new Date(a), db = new Date(b)
    return da.getFullYear() === db.getFullYear() && da.getMonth() === db.getMonth() && da.getDate() === db.getDate()
  }

  // Google-Messages-style session stamps: "1:58 PM", "Yesterday · 1:58 PM", …
  function sessionLabelFor(ts) {
    var d = new Date(ts), now = new Date()
    var start = new Date(now.getFullYear(), now.getMonth(), now.getDate())
    var diff = Math.floor((start - new Date(d.getFullYear(), d.getMonth(), d.getDate())) / 86400000)
    var t = Qt.formatTime(d, "h:mm AP")
    if (diff === 0) return t
    if (diff === 1) return "Yesterday · " + t
    if (diff < 7) return Qt.formatDate(d, "dddd") + " · " + t
    return Qt.formatDate(d, d.getFullYear() === now.getFullYear() ? "d MMM" : "d MMM yyyy") + " · " + t
  }

  function dayLabelFor(ts) {
    var d = new Date(ts), now = new Date()
    var start = new Date(now.getFullYear(), now.getMonth(), now.getDate())
    var diff = Math.floor((start - new Date(d.getFullYear(), d.getMonth(), d.getDate())) / 86400000)
    if (diff === 0) return "Today"
    if (diff === 1) return "Yesterday"
    if (diff < 7) return Qt.formatDate(d, "dddd")
    return Qt.formatDate(d, d.getFullYear() === now.getFullYear() ? "d MMMM" : "d MMMM yyyy")
  }

  function applyMediaReady(m) {
    if (m.kind === "avatar") {
      // Ask for a fresh room list so avatarPath fields fill in.
      avatarRefresh.restart()
      return
    }
    if (!m.roomId) return
    var t = root.timelineFor(m.roomId)
    if (!t) return
    var model = t.model
    for (var i = 0; i < model.count; i++) {
      var it = model.get(i)
      if (it.eventId === m.eventId && it.media) {
        var media = JSON.parse(JSON.stringify(it.media))
        if (m.thumbnail) media.thumbnailPath = m.path; else media.path = m.path
        model.setProperty(i, "media", media)
        break
      }
    }
  }

  Timer {
    id: avatarRefresh
    interval: 400; repeat: false
    onTriggered: root.request("rooms.list", {}, function(r, e) { if (r && r.rooms) root.applyRooms(r.rooms, !!r.loaded) })
  }

  // Polls, locations and stickers (engine: timeline/extras.rs).
  function createPoll(roomId, question, options, closed, cb) {
    root.request("poll.create", { roomId: roomId, question: question, options: options, closed: !!closed }, cb)
  }
  property string mapStyleUrl: ""
  property int mapProbe: -1     // -1 unknown, 1 usable, 0 module missing
  /// Whether the QtLocation MapLibre plugin is actually installed. A plain
  /// property, probed once: as a function that cached into `mapProbe`, every
  /// `mapReady` binding *wrote* the property it read — a binding loop per map.
  readonly property bool mapsAvailable: root.mapProbe === 1
  /// Test switch: draw location bubbles without a MapLibre renderer per bubble.
  property bool debugNoBubbleMaps: false
  function mapsRenderable() { return root.mapsAvailable }
  function probeMaps() {
    if (root.mapProbe >= 0) return
    var c = Qt.createComponent(Qt.resolvedUrl("mobile/MapView.qml"))
    root.mapProbe = (c.status === Component.Ready) ? 1 : 0
    if (root.mapProbe === 0)
      console.log("sigil: maps unavailable (install qt6-location and maplibre-native-qt):", c.errorString())
  }
  function setMapStyle(url, cb) { root.request("map.setStyle", { url: url || "" }, cb) }
  /// Spoilers already uncovered, by event id. Session-only, not persisted.
  property var revealedSpoilers: ({})
  function rememberSpoiler(eventId) {
    if (!eventId || root.revealedSpoilers[eventId]) return
    var m = Object.assign({}, root.revealedSpoilers)
    m[eventId] = true
    root.revealedSpoilers = m
  }

  /// Sigil-level "stop animating text", on top of whatever the system reports.
  property bool reduceMotion: false

  property bool liveSharing: false
  property string liveRoomId: ""
  property real liveUntil: 0
  function startLiveLocation(roomId, durationMs, cb) {
    root.request("location.startLive", { roomId: roomId, durationMs: durationMs }, cb)
  }
  function stopLiveLocation(cb) { root.request("location.stopLive", {}, cb) }
  function refreshPosition(cb) { root.request("position.refresh", {}, cb) }

  property string positionError: ""
  property bool positionKnown: false
  property real positionLat: 0
  property real positionLon: 0
  property real positionAccuracy: 0

  /// Read a document into the structured preview; the engine downloads it first.
  function docPreview(roomId, eventId, cb) {
    root.request("doc.preview", { roomId: roomId, eventId: eventId }, cb)
  }

  function pollVote(roomId, eventId, answers, cb) {
    root.request("poll.vote", { roomId: roomId, eventId: eventId, answers: answers || [] }, cb)
  }
  function endPoll(roomId, eventId, cb) {
    root.request("poll.end", { roomId: roomId, eventId: eventId }, cb)
  }
  /// `self` marks it MSC3488 m.self, which draws your face instead of a pin.
  function sendLocation(roomId, lat, lon, description, self, cb) {
    root.request("location.send",
                 { roomId: roomId, lat: lat, lon: lon, description: description || "", selfLocation: !!self },
                 cb)
  }
  function sendSticker(roomId, st, cb) {
    root.request("sticker.send", { roomId: roomId, url: st.url, body: st.body || "Sticker", width: st.width || 0, height: st.height || 0 }, cb)
  }
  function listStickers(cb) { root.request("stickers.list", {}, cb) }

  function ssoStart(hs) {
    root.authError = ""
    root.request("login.start", { homeserver: hs, openBrowser: true }, function(r, e) {
      if (e) root.authError = e.message
    })
  }
  function ssoCancel() { root.request("login.cancel", {}) }
  function loginFinishManual(q) { root.request("login.finish", { query: q }, function(r, e) { if (e) root.authError = e.message }) }
  function logout() { root.request("logout", { wipe: false }) }
  function submitRecoveryKey(key, cb) {
    root.recoveryError = ""
    root.request("recovery.recover", { key: key }, function(r, e) {
      if (e) root.recoveryError = e.message
      if (cb) cb(r, e)
    })
  }
  function skipRecovery() { root.recoverySkipped = true }

  function paginate(roomId) {
    if (root.paginationByRoom[roomId] === "paginating" || root.paginationByRoom[roomId] === "timelineStart") return
    root.request("timeline.paginate", { roomId: roomId, count: 50 })
  }
  function sendText(roomId, text, opts, cb) {
    var o = opts || {}
    if (o.editOf) root.request("message.edit", { roomId: roomId, eventId: o.editOf, body: text, markdown: true }, cb)
    else if (o.replyTo) root.request("message.reply", { roomId: roomId, eventId: o.replyTo, body: text, markdown: true }, cb)
    else root.request("message.send", { roomId: roomId, body: text, markdown: true }, cb)
  }
  function sendFiles(roomId, paths, caption) {
    for (var i = 0; i < paths.length; i++) {
      var p = { roomId: roomId, path: paths[i] }
      // One caption per batch, on the first file — the rest go bare.
      if (i === 0 && caption && String(caption).trim() !== "") p.caption = String(caption).trim()
      root.request("attachment.send", p)
    }
  }
  function react(roomId, eventId, key) { root.request("message.react", { roomId: roomId, eventId: eventId, key: key }) }
  function redact(roomId, eventId) { root.request("message.redact", { roomId: roomId, eventId: eventId }) }
  // The first few lines of a document, for its bubble preview. Keyed roomId|eventId.
  // `null` = asked and waiting, `false` = none to be had; asking again re-downloads.
  property var docThumbs: ({})
  signal docThumbReady(string key)
  function docThumbKey(roomId, eventId) { return roomId + "|" + eventId }

  // contacts
  /// The homeserver's user directory. Only knows local users and remote users who
  /// share a room with someone here, so an empty result is a normal answer.
  function searchDirectory(query, cb) {
    root.request("users.search", { query: query || "", limit: 12 }, cb)
  }
  function sendContact(roomId, userId, displayName, avatarUrl, cb) {
    root.request("contact.send", {
      roomId: roomId, userId: userId,
      displayName: displayName || "", avatarUrl: avatarUrl || ""
    }, cb)
  }
  /// The saved address book, from account data. Matrix syncs it across devices.
  property var savedContacts: []
  property bool _contactsLoaded: false
  function loadContacts() {
    root.request("contacts.list", {}, function (r, e) {
      if (r && r.contacts) root.savedContacts = r.contacts
    })
  }
  function saveContact(userId, nickname, favorite, cb) {
    var p = { userId: userId }
    if (nickname !== undefined && nickname !== null) p.nickname = nickname
    if (favorite !== undefined && favorite !== null) p.favorite = favorite
    root.request("contacts.save", p, function (r, e) {
      if (r && r.contacts) root.savedContacts = r.contacts
      if (cb) cb(r, e)
    })
  }
  function removeContact(userId, cb) {
    root.request("contacts.remove", { userId: userId }, function (r, e) {
      if (r && r.contacts) root.savedContacts = r.contacts
      if (cb) cb(r, e)
    })
  }
  function isSavedContact(userId) {
    for (var i = 0; i < root.savedContacts.length; i++)
      if (root.savedContacts[i].user_id === userId) return true
    return false
  }

  function contactVcf(userId, displayName, cb) {
    root.request("contact.vcf", { userId: userId, displayName: displayName || "" }, cb)
  }
  /// The same vCard, written to the user's downloads rather than the swept media cache.
  function downloadContactVcf(userId, displayName, cb) {
    root.request("contact.vcf", { userId: userId, displayName: displayName || "", download: true }, cb)
  }

  /// Parsed `.vcf` contents, keyed roomId|eventId. `false` = unreadable, so it falls back to a file chip.
  property var vcards: ({})
  signal vcardReady(string key)
  function readVcard(roomId, eventId) {
    if (!roomId || !eventId) return
    var k = root.docThumbKey(roomId, eventId)
    if (root.vcards[k] !== undefined) return
    var m = Object.assign({}, root.vcards); m[k] = null; root.vcards = m
    root.request("vcard.read", { roomId: roomId, eventId: eventId }, function (r, e) {
      var mm = Object.assign({}, root.vcards)
      mm[k] = (r && r.cards && r.cards.length > 0) ? r : false
      root.vcards = mm
      root.vcardReady(k)
    })
  }

  // Cover art, its colour and a waveform, none of which the event carries. Keyed
  // roomId|eventId; `null` = asked and waiting, `false` = none; asking again re-downloads.
  property var audioInfos: ({})
  signal audioInfoReady(string key)
  function audioInfo(roomId, eventId, size) {
    if (!roomId || !eventId) return
    var k = root.docThumbKey(roomId, eventId)
    if (root.audioInfos[k] !== undefined) return
    var m = Object.assign({}, root.audioInfos); m[k] = null; root.audioInfos = m
    root.request("audio.info", { roomId: roomId, eventId: eventId, size: size || 0 }, function (r, e) {
      var mm = Object.assign({}, root.audioInfos)
      mm[k] = r ? r : false
      root.audioInfos = mm
      root.audioInfoReady(k)
    })
  }
  function docPage(roomId, eventId, index, width, cb) {
    root.request("doc.page", { roomId: roomId, eventId: eventId, index: index, width: width }, cb)
  }
  function docThumb(roomId, eventId, size) {
    if (!roomId || !eventId) return
    var k = root.docThumbKey(roomId, eventId)
    if (root.docThumbs[k] !== undefined) return
    var m = Object.assign({}, root.docThumbs); m[k] = null; root.docThumbs = m
    root.request("doc.thumb", { roomId: roomId, eventId: eventId, size: size || 0 }, function(r, e) {
      var mm = Object.assign({}, root.docThumbs)
      // A PDF comes back drawn, not quoted, so empty `lines` plus an image is a hit.
      mm[k] = (r && ((r.lines && r.lines.length > 0) || r.imagePath)) ? r : false
      root.docThumbs = mm
      root.docThumbReady(k)
    })
  }

  property var linkPreviews: ({})
  signal linkPreviewReady(string url)
  // Failures are usually transient: the homeserver fetches the page itself and a cold
  // fetch of a heavy site can exceed our timeout. Retry twice, then remember briefly.
  property var linkPreviewFails: ({})
  function linkPreview(url, attempt) {
    if (!url) return
    attempt = attempt || 0
    var cur = root.linkPreviews[url]
    if (attempt === 0) {
      if (cur !== undefined && cur !== false) return
      if (cur === false) {
        var failedAt = root.linkPreviewFails[url] || 0
        if (Date.now() - failedAt < 300000) return       // cooldown
      }
      var m = Object.assign({}, root.linkPreviews); m[url] = null; root.linkPreviews = m
    }
    root.request("link.preview", { url: url }, function(r, e) {
      if (r && (r.title || r.description || r.imagePath)) {
        var mm = Object.assign({}, root.linkPreviews)
        mm[url] = r
        root.linkPreviews = mm
        root.linkPreviewReady(url)
        return
      }
      if (attempt < 2) {
        // give the homeserver time to finish and cache its own fetch
        var t = Qt.createQmlObject('import QtQuick; Timer { interval: 12000; running: true; repeat: false }', root)
        t.triggered.connect(function() { t.destroy(); root.linkPreview(url, attempt + 1) })
        return
      }
      var f = Object.assign({}, root.linkPreviewFails); f[url] = Date.now(); root.linkPreviewFails = f
      var mm2 = Object.assign({}, root.linkPreviews); mm2[url] = false; root.linkPreviews = mm2
      root.linkPreviewReady(url)
    })
  }

  // Voice messages
  property real voiceLevel: 0
  signal voiceLevelChanged2()
  function voiceStart(cb) { root.request("voice.start", {}, cb) }
  function voiceStop(cb) { root.request("voice.stop", {}, cb) }
  function voiceCancel() { root.request("voice.cancel", {}) }
  function voiceSend(roomId, path, duration, waveform, caption, cb) {
    root.request("voice.send", { roomId: roomId, path: path, duration: duration, waveform: waveform, caption: caption || "" }, cb)
  }
  function playAudio(roomId, eventId, seek, cb) { root.request("audio.play", { roomId: roomId, eventId: eventId, seek: seek || 0 }, cb) }
  function stopAudio() { root.request("audio.stop", {}) }
  function playAudioFile(path, cb) { root.request("audio.playFile", { path: path }, cb) }
  function playAudioFileAt(path, seek, cb) { root.request("audio.playFile", { path: path, seek: seek }, cb) }

  function playVideo(roomId, eventId, cb) { root.request("video.play", { roomId: roomId, eventId: eventId, audio: true }, cb) }
  function stopVideo() { root.request("video.stop", {}) }
  function seekVideo(seconds, cb) { root.request("video.seek", { seconds: seconds }, cb) }

  function editCaption(roomId, eventId, body) { root.request("message.editCaption", { roomId: roomId, eventId: eventId, body: body }) }
  function retrySend(roomId, item) { root.request("message.retry", { roomId: roomId, id: item.id || "", txnId: item.txnId || "" }) }
  function cancelSend(roomId, item) { root.request("message.cancelSend", { roomId: roomId, id: item.id || "", txnId: item.txnId || "" }) }
  function markRead(roomId) { root.request("room.markRead", { roomId: roomId }) }
  function readReceipt(roomId, eventId) { root.request("readReceipt", { roomId: roomId, eventId: eventId }) }
  property var _typingLast: ({})
  function setTyping(roomId, on) {
    var now = Date.now()
    if (on && root._typingLast[roomId] && now - root._typingLast[roomId] < 4000) return
    root._typingLast[roomId] = on ? now : 0
    root.request("typing", { roomId: roomId, typing: on })
  }
  function fetchMembers(roomId, cb) {
    root.request("room.members", { roomId: roomId }, function(r, e) {
      if (r) { var mb = Object.assign({}, root.membersByRoom); mb[roomId] = r.members; root.membersByRoom = mb }
      if (cb) cb(r, e)
    })
  }
  function fetchMedia(roomId, eventId, thumb, cb) {
    var p = { roomId: roomId, eventId: eventId }
    if (thumb) p.thumbnail = thumb
    root.request("media.get", p, cb)
  }
  function saveMedia(roomId, eventId, dest, cb) { root.request("media.saveAs", { roomId: roomId, eventId: eventId, dest: dest }, cb) }
  function searchUsers(q, cb) { root.request("users.search", { query: q, limit: 10 }, cb) }
  function createDm(userId, cb) { root.request("dm.create", { userId: userId }, cb) }
  // A thread and the pinned list are timelines with a different focus, filed under a key
  // beginning with the room id. That key is passed back as `roomId` afterwards, so the
  // whole timeline stack works on them unchanged.
  /// roomId -> [eventId] currently pinned.
  property var pinnedByRoom: ({})
  function pinnedIds(roomId) { return root.pinnedByRoom[roomId] || [] }
  function isPinned(roomId, eventId) {
    if (!eventId) return false
    return root.pinnedIds(roomId).indexOf(eventId) >= 0
  }
  function loadPinned(roomId) {
    if (!roomId) return
    root.request("pins.list", { roomId: roomId }, function (r, e) {
      if (!r || !r.events) return
      var m = Object.assign({}, root.pinnedByRoom); m[roomId] = r.events; root.pinnedByRoom = m
    })
  }
  function pinMessage(roomId, eventId, cb) { root.request("message.pin", { roomId: roomId, eventId: eventId }, cb) }
  function unpinMessage(roomId, eventId, cb) { root.request("message.unpin", { roomId: roomId, eventId: eventId }, cb) }
  function togglePin(roomId, eventId, cb) {
    if (root.isPinned(roomId, eventId)) root.unpinMessage(roomId, eventId, cb)
    else root.pinMessage(roomId, eventId, cb)
  }
  /// The pinned events themselves, fetched by id. Not a pinned-focus timeline:
  /// matrix-sdk-ui's `TimelineFocus::PinnedEvents` loads once at build time via
  /// `load_pinned_events()`, which Synapse answers 404 (ruma asks for
  /// `state/m.room.pinned_events/`, with a trailing slash), and it will not paginate.
  function pinnedItems(roomId, cb) { root.request("pins.items", { roomId: roomId }, cb) }
  /// Emitted on activity in a thread of this room, so an open view can refresh counts.
  signal threadsChanged(string roomId)
  /// A thread reply arrives on the THREAD's key while the root updates in the room's own timeline — watch both.
  function noteThreadActivity(key, ops) {
    if (!key) return
    var room = root.roomOfKey(key)
    if (key !== room) { root.threadsChanged(room); return }   // any thread view
    for (var i = 0; i < ops.length; i++) {
      var o = ops[i]
      var items = o.items ? o.items : (o.item ? [o.item] : [])
      for (var j = 0; j < items.length; j++) {
        if (items[j] && (items[j].threadRoot || items[j].threadSummary)) { root.threadsChanged(room); return }
      }
    }
  }

  function listThreads(roomId, cb) { root.request("threads.list", { roomId: roomId }, cb) }
  /// Open one thread. `cb(key)` gets the id to drive a ChatPage with.
  function openThread(roomId, rootId, cb) {
    root.ensureTimeline(roomId + "|thread:" + rootId)
    root.request("thread.open", { roomId: roomId, rootId: rootId, initialItems: 60 }, function (r, e) {
      if (cb) cb(r && r.key ? r.key : "")
    })
  }
  /// The room a view key belongs to; a plain room id is its own room.
  function roomOfKey(key) { var i = String(key || "").indexOf("|"); return i < 0 ? key : key.substring(0, i) }
  function isThreadKey(key) { return String(key || "").indexOf("|thread:") >= 0 }
  function threadRootOfKey(key) {
    var i = String(key || "").indexOf("|thread:")
    return i < 0 ? "" : key.substring(i + 8)
  }

  /// Someone reacted during a call. Ephemeral — nothing is stored.
  signal callReaction(string emoji, string who, bool own)
  function callReact(emoji) { root.request("call.react", { emoji: emoji }) }

  function joinRoom(idOrAlias, cb) { root.request("room.join", { roomIdOrAlias: idOrAlias }, cb) }
  function createRoom(opts, cb) { root.request("room.create", opts, cb) }
  /// A space is a room with `type: m.space`. Never encrypted: it carries no messages, and
  /// encrypting it only stops other clients reading the tree.
  function createSpace(nameOrOpts, cb) {
    var o = (typeof nameOrOpts === "string") ? { name: nameOrOpts } : (nameOrOpts || {})
    root.request("room.create", Object.assign({ topic: "", private: true, encrypted: false, space: true }, o), cb)
  }
  /// A space's contents are `m.space.child` state on the space itself.
  function addRoomToSpace(spaceId, roomId, cb) { root.request("space.addRoom", { spaceId: spaceId, roomId: roomId }, cb) }
  function removeRoomFromSpace(spaceId, roomId, cb) { root.request("space.removeRoom", { spaceId: spaceId, roomId: roomId }, cb) }
  function spacesContaining(roomId) {
    var out = []
    for (var i = 0; i < root.spaces.length; i++) {
      var sp = root.spaces[i]
      if ((sp.children || []).indexOf(roomId) >= 0) out.push(sp)
    }
    return out
  }
  function leaveRoom(roomId, cb) { root.request("room.leave", { roomId: roomId }, cb) }
  function inviteUser(roomId, userId, cb) { root.request("room.invite", { roomId: roomId, userId: userId }, cb) }
  function setFavourite(roomId, on) { root.request("room.setFavourite", { roomId: roomId, favourite: on }) }

  // A space IS a room, so every settings call below takes a plain `roomId` and the
  // same pages serve both. Only the hierarchy and child add/remove are space-shaped.

  /// Every child of a space, joined or not: `spaces.tree` carries child ids only.
  function spaceHierarchy(spaceId, cb) { root.request("space.hierarchy", { spaceId: spaceId }, cb) }

  /// Everything the settings pages read, in one round trip, so they cannot disagree mid-edit.
  function roomSettings(roomId, cb) { root.request("room.settings", { roomId: roomId }, cb) }
  /// Writes only the fields present; absent means "leave alone", so pages do not overwrite each other.
  function setRoomSettings(roomId, fields, cb) { root.request("room.setSettings", Object.assign({ roomId: roomId }, fields), cb) }
  function setRoomAvatar(roomId, path, cb) { root.request("room.setAvatar", { roomId: roomId, path: path || "" }, cb) }
  /// `userId` moves a person between roles; `key` moves the bar for one capability. Same state event.
  function setPowerLevel(roomId, fields, cb) { root.request("room.setPowerLevel", Object.assign({ roomId: roomId }, fields), cb) }
  function setLowPriority(roomId, on) { root.request("room.setLowPriority", { roomId: roomId, lowPriority: on }) }
  function setFocus(roomId, visible) { root.request("ui.focus", { roomId: roomId || "", visible: !!visible }) }

  function callStart(roomId, video) { root.request("call.start", { roomId: roomId, video: !!video }) }
  function callJoin(roomId, video) { root.request("call.join", { roomId: roomId, video: !!video }) }
  function callAccept() { if (root.call.incoming) root.callJoin(root.call.incoming.roomId, root.call.incoming.intent === "video") }
  function callDecline() { if (root.call.incoming) root.request("call.decline", { roomId: root.call.incoming.roomId }) }
  function callHangup() { root.request("call.leave", {}) }
  function callSetMic(on) { root.request("call.mute", { muted: !on }) }
  function callSetCamera(on) { root.request("call.camera", { enabled: !!on }) }
  function callScreenshare(on) { root.request("call.screenshare", { enabled: !!on }) }
  function callSelectDevice(kind, id) { root.request("call.setDevice", { kind: kind, id_: id }) }
  function openRoomAfterAccept(roomId) {
    root.pendingOpenRoomId = roomId
    if (root.roomsById[roomId]) { root.selectRoom(roomId); root.pendingOpenRoomId = "" }
    if (root.shell && typeof root.shell.summon === "function") root.shell.summon(root.pluginId, JSON.stringify({ roomId: roomId }))
  }

  // Always-on call surfaces (survive the main window being closed).
  Loader { active: root.call && root.call.incoming !== null && root.call.incoming !== undefined; source: "calls/CallBanner.qml"; onLoaded: item.svc = root }
  Loader { active: root.inCall && !(root.panelOpen() && root.selectedRoomId === root.call.roomId); source: "calls/CallPill.qml"; onLoaded: item.svc = root }
  function refreshDevices() { root.request("call.devices", {}, function(r) { if (r) root.devices = r }) }

  // notifications
  function panelOpen() {
    return root.shell && root.shell.openPanelIds && root.shell.openPanelIds[root.pluginId] === true
  }

  onNotifyEvent: function(info) {
    // The engine already ran notify-send unless the room was focused; keep the badge fresh.
  }

  // IPC
  IpcHandler {
    target: "sigil"

    function ping(): string { return root.connected ? "ok" : "engine not connected" }
    function status(): string {
      return JSON.stringify({ connected: root.connected, engineMissing: root.engineMissing, auth: root.authState, sync: root.syncState,
        userId: root.userId, rooms: root.rooms.length, unread: root.unreadTotal, highlights: root.highlightTotal, call: root.call.state })
    }
    function toggle(): string { if (root.shell) root.shell.toggle(root.pluginId, "{}"); return "ok" }
    function open(): string { if (root.shell) root.shell.summon(root.pluginId, "{}"); return "ok" }
    function openRoom(roomId: string): string {
      root.pendingOpenRoomId = roomId
      if (root.roomsById[roomId]) { root.selectRoom(roomId); root.pendingOpenRoomId = "" }
      if (root.shell) root.shell.summon(root.pluginId, JSON.stringify({ roomId: roomId }))
      return "ok"
    }
    function callAccept(): string { root.callAccept(); return "ok" }
    function callDecline(): string { root.callDecline(); return "ok" }
    function callHangup(): string { root.callHangup(); return "ok" }
    function callToggle(): string {
      if (root.call.incoming) root.callAccept()
      else if (root.inCall) root.callHangup()
      return "ok"
    }
    function callToggleMic(): string { root.callSetMic(!!(root.call.local && root.call.local.micMuted)); return "ok" }
    function callToggleCamera(): string { root.callSetCamera(!(root.call.local && root.call.local.cameraOn)); return "ok" }
    function markAllRead(): string {
      for (var i = 0; i < root.rooms.length; i++) if ((root.rooms[i].unread > 0) || (root.rooms[i].unreadMessages > 0)) root.markRead(root.rooms[i].id)
      return "ok"
    }
    function logout(): string { root.logout(); return "ok" }
    function callDump(): string { return JSON.stringify(root.call) }
    function debug(what: string): string {
      var t = root.timelineFor(root.selectedRoomId)
      return JSON.stringify({ selectedRoomId: root.selectedRoomId, timelineCount: t ? t.model.count : -1, pending: Object.keys(root._pending).length, protocol: root.protocol, engine: root.engineVersion })
    }
  }

  Component.onCompleted: { root.probeMaps(); root.attemptConnect() }
}
