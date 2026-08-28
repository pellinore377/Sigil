import QtQuick
import QtQuick.Effects
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import qs.Commons
import qs.Ui
import "components"
import "mobile"
import "calls"

// Sigil panel host for omarchy-shell: a compact frosted panel next to the bar.
Item {
  id: root

  readonly property string selfId: "pellinore.sigil"
  property bool opened: false

  property var shell: null
  property var service: null
  property var manifest: null
  onShellChanged: {
    if (!root.opened && root.shell && root.shell.openPanelIds && root.shell.openPanelIds[root.selfId] === true)
      root.open("{}")
  }
  readonly property var svc: root.service ? root.service
    : ((root.shell && typeof root.shell.serviceFor === "function") ? root.shell.serviceFor(root.selfId) : null)
  readonly property bool hasService: root.svc !== null && root.svc !== undefined

  // The card is the chrome surface, tinted by a chat theme. Gated on the chat page
  // being settled at x = 0 — keying off the room alone repaints the card mid-slide.
  readonly property bool chatThemed: root.nav === "chat" && chatPage.themed && chatHolder.x < 0.5
  property color background: root.chatThemed ? chatPage.chromeC : Util.alpha(Qt.lighter(Color.menu.background, 1.35), 0.96)
  // The card tint is translucent; a page stacked over another must be opaque.
  readonly property color pageGround: Qt.rgba(root.background.r, root.background.g, root.background.b, 1)
  /// Ground for pages belonging to a themed ROOM. Their chrome is the room's, so
  /// the neutral ground would show through the inverted corner cut-outs at the top.
  readonly property color roomPageGround: {
    var t = root.chatThemes[root.selectedRoomId] || ({})
    if ((t.accent || "") === "") return root.pageGround
    var base = Qt.lighter(Color.menu.background, 1.35)
    var a = Qt.color(t.accent)
    // The same mix ChatPage uses for `surfaceC`, which is what its header is.
    return Qt.rgba(base.r * 0.65 + a.r * 0.35, base.g * 0.65 + a.g * 0.35, base.b * 0.65 + a.b * 0.35, 1)
  }
  property color foreground: Color.menu.text
  property var borderSpec: Border.surfaceSpec("menu", "border", Color.menu.border, Math.max(1, Style.space(2)))
  readonly property string barPos: (root.shell && root.shell.bar && root.shell.bar.position) ? String(root.shell.bar.position) : "left"
  readonly property bool barVertical: barPos === "left" || barPos === "right"
  readonly property real barEdge: (root.shell && root.shell.bar && root.shell.bar.barSize > 0) ? root.shell.bar.barSize : (barVertical ? Style.bar.sizeVertical : Style.bar.sizeHorizontal)

  // state
  readonly property string page: !root.hasService ? "login"
    : (root.svc.authState !== "loggedIn" ? "login" : (root.svc.needsRecovery ? "recovery" : "main"))
  property string nav: "home"            // main-page navigation: home | chat | start | forward
  // spaces
  property string spaceId: ""            // the space whose page is open
  property string spaceRoomsMode: "manage"   // manage | add, for SpaceRoomsPage
  /// A space picked from its own page, so a room created from there lands in it.
  property string spaceForNewRoom: ""
  // Serve a room and a space alike, so they must be told where back goes.
  property string settingsRoomId: ""
  property string settingsReturn: "roomsettings"
  property string membersReturn: "space"
  property int membersFilter: -1
  /// Which navs keep the chat page stacked underneath. Anything not listed slides
  /// the chat out too, flashing the room's exit animation behind the new page.
  readonly property bool chatUnder: ["chat", "search", "roomsettings", "chattheme", "forward", "map", "doc",
                                     "threads", "thread", "pins"].indexOf(root.nav) >= 0
    || (root.nav === "addpeople" && root.addReturn === "chat")
  property var forwardItem: null
  /// "forward" | "attach" — see ForwardPage.mode.
  property string forwardMode: "forward"
  /// Files waiting for a room to be chosen.
  property var pendingShare: []

  /// Write a contact out as a `.vcf` and ask where it should go.
  function shareContactVcf(userId, displayName) {
    if (!root.hasService || !userId) return
    root.svc.contactVcf(userId, displayName, function (r, e) {
      if (!r || !r.path) return
      root.pendingShare = [r.path]
      root.forwardMode = "attach"
      forwardPage.reset()
      root.nav = "forward"
      Qt.callLater(forwardPage.focusSearch)
    })
  }
  property string addReturn: "chat"
  // Threads and pins are engine timelines filed under a key beginning with the
  // room id. Holding the key is all the UI needs; the chat page renders either.
  property string threadKey: ""
  /// Open one thread by its root event.
  function openThread(rootId) {
    if (!root.hasService || !root.selectedRoomId || !rootId) return
    root.svc.openThread(root.selectedRoomId, rootId, function (key) {
      if (!key) return
      root.threadKey = key
      root.nav = "thread"
    })
  }
  /// Open the room's pinned messages.
  function openPins() {
    if (!root.hasService || !root.selectedRoomId) return
    pinsPage.reset()
    root.nav = "pins"
  }
  readonly property string selectedRoomId: root.hasService ? root.svc.selectedRoomId : ""
  readonly property bool callHere: root.hasService && root.svc.call && (root.svc.call.state === "joining" || root.svc.call.state === "connected" || root.svc.call.state === "reconnecting" || root.svc.call.state === "leaving")
  property bool callMinimized: false   // call running as an in-app PiP
  property bool callPageOpen: false

  // Minimising shrinks the call page into the PiP. The origin is resolved once,
  // here, at the moment the gesture starts — a mapFromItem binding never updates.
  function minimizeCall() {
    if (!root.callPageOpen) return
    var c = callHolder.mapFromItem(callPip.parent, callPip.x + callPip.width / 2,
                                                   callPip.y + callPip.height / 2)
    callHolder.shrinkOx = c.x
    callHolder.shrinkOy = c.y
    callHolder.shrinkScale = callPip.width / Math.max(1, callHolder.width)
    callHolder.shrinking = true
    root.callMinimized = true
    shrinkDone.restart()
  }
  // Expanding runs the shrink backwards: snap to the tile with animations off, then grow.
  function maximizeCall() {
    if (root.callPageOpen) return "already open"
    if (!root.callMinimized) { root.callPageOpen = true; return "no pip" }
    // Park the holder first, *then* resolve the origin: while minimised it sits
    // off-screen at x = width, so an origin measured earlier is a panel width off.
    callHolder.instant = true
    callHolderSlide.running = false
    callHolder.x = 0
    var c = callHolder.mapFromItem(callPip.parent, callPip.x + callPip.width / 2,
                                                   callPip.y + callPip.height / 2)
    callHolder.shrinkOx = c.x
    callHolder.shrinkOy = c.y
    callHolder.shrinkScale = callPip.width / Math.max(1, callHolder.width)
    callHolder.shrinking = true
    root.callMinimized = false
    root.callPageOpen = true
    growStart.restart()
    return "pip=" + Math.round(callPip.x) + "," + Math.round(callPip.y)
         + " origin=" + Math.round(c.x) + "," + Math.round(c.y)
         + " scale=" + callHolder.shrinkScale.toFixed(3)
         + " holder=" + Math.round(callHolder.x) + "," + Math.round(callHolder.y)
         + " " + Math.round(callHolder.width) + "x" + Math.round(callHolder.height)
  }
  Timer { id: growStart; interval: 16; onTriggered: { callHolder.instant = false; callHolder.shrinking = false } }

  // Already shrunk into the tile, so park the holder rather than sliding it out too.
  Timer {
    id: shrinkDone
    interval: 230
    onTriggered: {
      root.callPageOpen = false
      callHolderSlide.running = false
      callHolder.x = callHolder.width
      callHolder.shrinking = false
    }
  }
  property var drafts: ({})

  // Register this plugin in shell.json so its bar entry exists.
  property bool selfRefEnsured: false
  readonly property string ensureSelfRefScript: [
    'id="$1"',
    'f="$HOME/.config/omarchy/shell.json"',
    '[ -f "$f" ] || exit 0',
    'jq -e --arg id "$id" \'any(.plugins[]?; (.id // empty) == $id)\' "$f" >/dev/null && exit 0',
    'tmp="$f.selfref.$$"',
    'jq --arg id "$id" \'.plugins = ((.plugins // []) + [{id: $id}])\' "$f" > "$tmp" || { rm -f "$tmp"; exit 1; }',
    '[ -s "$tmp" ] || { rm -f "$tmp"; exit 1; }',
    'mv "$tmp" "$f"'
  ].join("\n")
  function ensureSelfReference() {
    if (root.selfRefEnsured) return
    root.selfRefEnsured = true
    Quickshell.execDetached(["sh", "-c", root.ensureSelfRefScript, "plugin-selfref", root.selfId])
  }

  // open/close
  function open(payloadJson) {
    root.opened = true
    root.ensureSelfReference()
    try {
      var p = JSON.parse(String(payloadJson || "{}"))
      if (p && p.roomId && root.hasService) { root.svc.selectRoom(p.roomId); root.nav = "chat" }
      if (p && p.window) { root.maximize(); return }
    } catch (e) {}
    if (root.callHere) root.callPageOpen = true
    root.reportFocus()
    Qt.callLater(root.focusDefault)
  }
  function close() {
    if (!root.opened) return
    root.opened = false
    imageViewer.close()
    dialogs.close()
    root.reportFocus()
    if (root.shell && typeof root.shell.hide === "function") root.shell.hide(root.selfId)
  }
  function toggle() { if (root.opened) root.close(); else root.open("{}") }

  // Windowed view: launches the standalone desktop app once it exists.
  function maximize() {}

  function focusDefault() {
    if (!root.opened) return
    if (root.page === "login") loginPage.focusInput()
    else if (root.page === "recovery") recoveryPage.focusInput()
    else if (root.nav === "start") startPage.focusSearch()
    else if (root.nav === "forward") forwardPage.focusSearch()
    else if (root.nav === "search") searchPage.focusSearch()
    else if (root.nav === "addpeople") addPeoplePage.focusSearch()
    else if (root.nav === "chat") chatPage.focusInput()
    else homePage.focusSearch()
  }
  function reportFocus() { if (root.hasService) root.svc.setFocus(root.nav === "chat" ? root.selectedRoomId : "", root.opened && root.page === "main") }
  /// Anything that opens over the conversation must dismiss its message sheet first.
  function dismissSheet() {
    if (!messageSheet.item) return
    messageSheet.drawerOpen = false
    messageSheet.close()
  }

  onNavChanged: {
    root.reportFocus()
    // Leaving the conversation for any expanded view has to take its sheet along.
    if (root.nav !== "chat") root.dismissSheet()
  }
  onOpenedChanged: root.reportFocus()

  // One place for "go back": media viewer, then modals, then the page stack, then the
  // panel. A window Shortcut, because a focused composer or search field swallows
  // Escape first — and it arrives twice, so guard against skipping a level.
  property real lastBack: 0
  function goBack() {
    var now = Date.now()
    if (now - root.lastBack < 90) return
    root.lastBack = now
    root.goBackNow()
  }
  function goBackNow() {
    if (imageViewer.item) { imageViewer.close(); return }
    if (dialogs.mode !== "") { dialogs.close(); return }
    if (messageSheet.confirmItem) { messageSheet.confirmItem = null; return }
    if (messageSheet.item) { if (messageSheet.drawerOpen) messageSheet.drawerOpen = false; else messageSheet.close(); return }
    if (root.nav === "home" && homePage.accountOpen) { homePage.accountOpen = false; return }
    // Overlays unwind innermost-first; the emoji tray is one of them.
    if (root.callPageOpen && callPage.reactOpen) { callPage.reactOpen = false; return }
    if (root.callPageOpen && callPage.settingsOpen) { callPage.settingsOpen = false; return }
    if (root.callPageOpen && root.callHere) { root.minimizeCall(); return }
    if (root.callPageOpen) {
      // Escape out of a call minimises rather than abandons it: the PiP keeps the feed.
      root.callPageOpen = false
      root.callMinimized = true
      return
    }
    if (root.nav === "chat" && chatPage.menuOpen) { chatPage.menuOpen = false; return }
    if (root.nav === "start") { root.goHome(); return }
    if (root.nav === "thread") {
      // You almost certainly just replied, so the counts behind you are stale.
      root.nav = "threads"
      threadsPage.load()
      chatPage.loadThreads()
      return
    }
    if (root.nav === "threads" || root.nav === "pins") { root.nav = "chat"; return }
    if (root.nav === "forward" || root.nav === "search" || root.nav === "chattheme" || root.nav === "roomsettings" || root.nav === "doc" || root.nav === "audio") { root.nav = "chat"; return }
    if (root.nav === "map") { root.closeLocationView(); return }
    if (root.nav === "addpeople") { root.nav = root.addReturn; return }
    // Spaces unwind as entered: the settings children know which page opened them.
    if (root.nav === "permissions") { root.nav = "roles"; return }
    if (root.nav === "notifications" || root.nav === "security" || root.nav === "roles") { root.nav = root.settingsReturn; return }
    if (root.nav === "members") { root.nav = root.membersReturn; return }
    if (root.nav === "spacerooms" || root.nav === "spacesettings") { root.nav = "space"; return }
    if (root.nav === "space" || root.nav === "newspace") { root.goHome(); return }
    if (root.nav === "chat") { root.goHome(); return }
    root.close()
  }

  function openRoom(id) {
    if (!root.hasService) return
    if (root.nav === "chat" && root.selectedRoomId) { var d = Object.assign({}, root.drafts); d[root.selectedRoomId] = chatPage.textValue(); root.drafts = d }
    root.svc.selectRoom(id)
    root.nav = "chat"
    chatPage.clearComposer()
    chatPage.setText(root.drafts[id] || "")
    Qt.callLater(chatPage.focusInput)
  }
  // Test hook: open the viewer on the first image in the room.
  function debugOpenImage() {
    if (!root.hasService) return
    var t = root.svc.timelineFor(root.selectedRoomId)
    if (!t) return
    var m = t.model
    for (var i = 0; i < m.count; i++) { var it = m.get(i); if (it.kind === "image") { imageViewer.show(it); return } }
  }

  // Test hook: open the message sheet on the first text message.
  function debugOpenSheet(drawer, y) {
    if (!root.hasService) return
    var t = root.svc.timelineFor(root.selectedRoomId)
    if (!t) return
    var m = t.model
    for (var i = 0; i < m.count; i++) { var it = m.get(i); if (it.kind === "text") { messageSheet.openFor(it, 110, y === undefined ? 400 : y, Style.space(240), Style.space(64)); if (drawer) messageSheet.drawerOpen = true; return } }
  }

  /// Test hook: open the action sheet on the newest message of a given kind.
  function debugSheetKind(kind) {
    if (!root.hasService) return "no service"
    var t = root.svc.timelineFor(root.selectedRoomId)
    if (!t) return "no timeline"
    var m = t.model
    for (var i = 0; i < m.count; i++) {
      var it = m.get(i)
      if (it.kind !== kind) continue
      messageSheet.openFor(it, 110, 400, Style.space(240), Style.space(64))
      return "ok " + i
    }
    return "no " + kind
  }

  function debugPressImage(mode) { chatPage.debugPressImage(mode) }
  function debugPressFailed() { chatPage.debugPressFailed() }
  function debugPressSending() { chatPage.debugPressSending() }

  function debugCloseSheet() { messageSheet.close() }

  function debugChatMenu() { chatPage.menuOpen = true }

  function debugViewerPlay() {
    chatPage.debugPressImage("image")
    Qt.callLater(function() {
      messageSheet.close()
      var t = root.svc.timelineFor(root.selectedRoomId)
      var m = t.model
      for (var i = 0; i < m.count; i++) { var it = m.get(i); if (it.kind === "image") { imageViewer.show(it); break } }
      imageViewer.playingEvent = imageViewer.curItem ? imageViewer.curItem.eventId : "x"
      imageViewer.playShm = Quickshell.env("XDG_RUNTIME_DIR") + "/sigil/video-test.shm"
      imageViewer.playDuration = 42
      imageViewer.playOffset = 8
    })
  }

  function debugJump() { chatPage.jumpToLatest() }
  function debugConfirmDelete() { debugOpenSheet(false); messageSheet.confirmItem = messageSheet.item }
  function debugJumpReport() { chatPage.debugJumpReport() }

  function debugPicker() { themePage.reset(); root.nav = "chattheme"; themePage.pickingColor = true }

  function debugThemeScroll() { themePage.scrollToEnd() }

  function openSpace(id) {
    root.spaceId = id
    root.settingsRoomId = id
    root.membersReturn = "space"
    root.nav = "space"
  }
  /// The four shared settings pages, reached from a room or from a space. `ret` is
  /// where their back button goes — the only thing that differs between the two.
  function openSettingsChild(which, ret) {
    root.settingsReturn = ret
    root.nav = which
  }

  function goHome() {
    if (root.selectedRoomId) { var d = Object.assign({}, root.drafts); d[root.selectedRoomId] = chatPage.textValue(); root.drafts = d }
    root.nav = "home"
    Qt.callLater(homePage.focusSearch)
  }

  // Hide the overlay around external dialogs (file chooser, share picker).
  property bool returnAfterDialog: false
  function hideForExternalDialog() { root.returnAfterDialog = true; root.close() }
  function returnFromDialog() {
    if (!root.returnAfterDialog) return
    root.returnAfterDialog = false
    if (root.shell && typeof root.shell.summon === "function") root.shell.summon(root.selfId, JSON.stringify({ roomId: root.selectedRoomId }))
  }

  // per-chat themes (local)
  property var chatThemes: ({})
  readonly property string themesPath: Quickshell.env("HOME") + "/.local/state/sigil/chat-themes.json"
  FileView { id: themesFile; path: root.themesPath; onLoaded: { try { root.chatThemes = JSON.parse(themesFile.text()) } catch (e) {} } }
  function setChatTheme(rid, t) {
    var m = JSON.parse(JSON.stringify(root.chatThemes))
    if (t && ((t.accent || "") !== "" || (t.wallpaper || "") !== "")) m[rid] = t; else delete m[rid]
    root.chatThemes = m
    Quickshell.execDetached(["sh", "-c", 'mkdir -p "$(dirname "$2")"; printf %s "$1" > "$2"', "themes", JSON.stringify(m), root.themesPath])
  }

  // An empty `roomId` means the New space page: it holds the path and uploads later.
  Process {
    id: avatarPicker
    property string roomId: ""
    command: ["omarchy-file-select", "--title", "Choose picture"]
    stdout: StdioCollector {
      onStreamFinished: {
        var f = String(text || "").trim().split("\n")[0]
        if (f === "") return
        if (avatarPicker.roomId === "") newSpacePage.avatarPath = f
        else spaceSettingsPage.newAvatarPath = f
      }
    }
    onExited: root.returnFromDialog()
  }
  function pickRoomAvatar(roomId) {
    if (avatarPicker.running) return
    avatarPicker.roomId = roomId || ""
    root.hideForExternalDialog()
    avatarPicker.running = true
  }

  Process {
    id: wallPicker
    property string roomId: ""
    command: ["omarchy-file-select", "--title", "Choose wallpaper"]
    stdout: StdioCollector {
      onStreamFinished: {
        var p = String(text || "").trim().split("\n")[0]
        if (p !== "" && wallPicker.roomId !== "") {
          var t = JSON.parse(JSON.stringify(root.chatThemes[wallPicker.roomId] || {}))
          t.wallpaper = p
          root.setChatTheme(wallPicker.roomId, t)
        }
      }
    }
    onExited: root.returnFromDialog()
  }
  function pickWallpaper() {
    if (wallPicker.running || !root.selectedRoomId) return
    wallPicker.roomId = root.selectedRoomId
    root.hideForExternalDialog()
    wallPicker.running = true
  }

  Process {
    id: filePicker
    property string roomId: ""
    command: ["omarchy-file-select", "--multiple", "--title", "Send with Sigil"]
    stdout: StdioCollector {
      onStreamFinished: {
        var paths = String(text || "").trim().split("\n").filter(function(x) { return x.trim() !== "" })
          // Stage them in the composer rather than firing them off at chooser close.
        if (paths.length && filePicker.roomId) chatPage.addAttachments(paths)
      }
    }
    onExited: root.returnFromDialog()
  }
  function pickFiles() {
    if (!root.selectedRoomId || filePicker.running) return
    filePicker.roomId = root.selectedRoomId
    root.hideForExternalDialog()
    filePicker.running = true
  }

  PanelWindow {
    id: panel
    visible: root.opened
    anchors { top: true; bottom: true; left: true; right: true }
    color: "transparent"
    WlrLayershell.namespace: "omarchy-sigil"
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: root.opened ? WlrKeyboardFocus.Exclusive : WlrKeyboardFocus.None
    exclusionMode: ExclusionMode.Ignore

    MouseArea { anchors.fill: parent; onClicked: root.close() }

    BorderSurface {
      id: card
      objectName: "sigilCard"
      focus: true
      Keys.onEscapePressed: function(e) { root.goBack(); e.accepted = true }
      readonly property real availH: panel.height - Style.gapsOut * 2
      width: Style.space(400)
      height: Math.min(Style.space(820), availH)
      x: (root.barPos === "left" ? root.barEdge + Style.gapsOut : Style.gapsOut)
      y: Style.gapsOut + Math.round((availH - height) / 2)
      radius: Style.space(22)
      color: root.background
      borderSpec: root.borderSpec
      clip: true

      MouseArea { anchors.fill: parent; onClicked: {} }


      // Engine / sync status strip
      Rectangle {
        id: statusStrip
        anchors.top: parent.top; anchors.left: parent.left; anchors.right: parent.right
        readonly property string msg: !root.hasService ? "" : (!root.svc.connected ? "Connecting to the Sigil engine…" : (root.svc.authState === "loggedIn" && root.svc.syncState === "error" ? "Sync error" : (root.svc.authState === "loggedIn" && root.svc.syncState === "offline" ? "Offline — reconnecting…" : "")))
        visible: msg !== "" && root.page === "main"
        height: visible ? Style.space(22) : 0
        topLeftRadius: Style.space(22); topRightRadius: Style.space(22)
        antialiasing: true
        color: Util.alpha(Color.urgent, 0.18)
        Text { anchors.centerIn: parent; text: statusStrip.msg; color: root.foreground; font.family: Fonts.ui; font.pixelSize: Style.font.caption }
      }

      LoginPage { id: loginPage; anchors.fill: parent; visible: root.page === "login"; svc: root.svc; fg: root.foreground }
      RecoveryPage { id: recoveryPage; anchors.fill: parent; visible: root.page === "recovery"; svc: root.svc; fg: root.foreground }

      Item {
        id: mainPages
        // While a page is mid-slide the pages beneath stay visible and paint an
        // opaque ground. Home hides only once a page above it sits flush at x = 0.
        readonly property bool homeCovered: (chatHolder.visible && chatHolder.x < 0.5)
          || (startHolder.visible && startHolder.x < 0.5)
        readonly property bool sliding: chatHolder.sliding || startHolder.sliding
          || searchHolder.sliding || settingsHolder.sliding || themeHolder.sliding
          || forwardHolder.sliding || addHolder.sliding || callHolder.sliding
        anchors.fill: parent
        anchors.topMargin: statusStrip.height
        visible: root.page === "main"
        // Round-clip every page to the card shape, the map page included: without
        // it, content spills past the card's rounded corners.
        layer.enabled: true
        layer.smooth: true
        layer.effect: MultiEffect {
          maskEnabled: true
          maskSource: pagesMask
          // A hard mask cutoff aliases the card corners; these soften the edge.
          maskThresholdMin: 0.5
          maskSpreadAtMin: 1.0
        }

        Item {
          id: homeHolder
          readonly property bool active: root.nav === "home"
          width: parent.width; height: parent.height; z: 0
          readonly property bool sliding: false
          visible: active || !mainPages.homeCovered
          HomePage {
            id: homePage
          tipLayer: tipLayer
            visible: true
            anchors.fill: parent
            svc: root.svc; fg: root.foreground
            drafts: root.drafts
            onRoomSelected: function(id) { root.openRoom(id) }
            onNewChat: { startPage.reset(); root.spaceForNewRoom = ""; root.nav = "start"; Qt.callLater(startPage.focusSearch) }
            onNewSpace: { newSpacePage.reset(); root.nav = "newspace"; Qt.callLater(newSpacePage.focusInput) }
            onSpaceOpened: function (id) { root.openSpace(id) }
            onMaximizeRequested: root.maximize()
          }
        }
        Item {
          id: docHolder
          readonly property bool active: root.nav === "doc"
          width: parent.width; height: parent.height; z: 2
          readonly property bool sliding: x > 0.5 && x < width - 0.5
          readonly property real targetX: active ? 0 : width
          // Pages slide in from the right, frame-driven rather than a timed Behavior:
          // opening a room stalls the UI thread, and a timed animation spends that stall
          // and arrives already finished. Every other page holder below repeats this shape.
          visible: x < width - 0.5
          onTargetXChanged: docHolderSlide.running = true
          Component.onCompleted: x = targetX
          onWidthChanged: if (!active && !docHolderSlide.running) x = width
          FrameAnimation {
            id: docHolderSlide
            running: false
            onTriggered: {
              var d = docHolder.targetX - docHolder.x
              if (Math.abs(d) < 0.5) { docHolder.x = docHolder.targetX; running = false; return }
              var step = d * 0.26
              var floor = Math.max(2, docHolder.width * 0.03)
              if (Math.abs(step) < floor) step = d > 0 ? floor : -floor
              docHolder.x += step
            }
          // Own ground so stacked pages never show through. The sinks sit *behind*
          // the page, so its own lists get the wheel first; only the rest is swallowed.
          }
          Rectangle {
            anchors.fill: parent
            color: root.pageGround
            MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; hoverEnabled: true }
            WheelHandler { onWheel: function(e) { e.accepted = true } }
          }
          DocumentPage {
            id: docPage
            anchors.fill: parent
            svc: root.svc; fg: root.foreground
            roomId: root.selectedRoomId
            accent: chatPage.themed ? chatPage.accC : Color.accent
            surface: Util.alpha(root.foreground, 0.08)
            onBackRequested: root.nav = "chat"
          }
        }
        Item {
          id: mapHolder
          readonly property bool active: root.nav === "map"
          width: parent.width; height: parent.height; z: 2
          readonly property bool sliding: x > 0.5 && x < width - 0.5
          readonly property real targetX: active ? 0 : width
          visible: x < width - 0.5

          // Opening grows out of the tapped bubble; closing fades. MapLibre draws through
          // its own scene graph node, so this scales the holder, not anything inside it.
          property bool shrinking: false
          property bool instant: false
          property real shrinkOx: 0
          property real shrinkOy: 0
          property real shrinkScale: 0.25
          transform: Scale {
            origin.x: mapHolder.shrinkOx
            origin.y: mapHolder.shrinkOy
            xScale: mapHolder.shrinking ? mapHolder.shrinkScale : 1
            yScale: mapHolder.shrinking ? mapHolder.shrinkScale : 1
            Behavior on xScale { enabled: !mapHolder.instant; NumberAnimation { duration: 260; easing.type: Easing.OutCubic } }
            Behavior on yScale { enabled: !mapHolder.instant; NumberAnimation { duration: 260; easing.type: Easing.OutCubic } }
          }
          opacity: mapHolder.shrinking ? 0 : 1
          Behavior on opacity {
            enabled: !mapHolder.instant
            NumberAnimation { duration: 200; easing.type: mapHolder.shrinking ? Easing.InCubic : Easing.OutCubic }
          }
          onTargetXChanged: mapHolderSlide.running = true
          Component.onCompleted: x = targetX
          onWidthChanged: if (!active && !mapHolderSlide.running) x = width
          FrameAnimation {
            id: mapHolderSlide
            running: false
            onTriggered: {
              var d = mapHolder.targetX - mapHolder.x
              if (Math.abs(d) < 0.5) { mapHolder.x = mapHolder.targetX; running = false; return }
              var step = d * 0.26
              var floor = Math.max(2, mapHolder.width * 0.03)
              if (Math.abs(step) < floor) step = d > 0 ? floor : -floor
              mapHolder.x += step
            }
          }
          Rectangle {
            anchors.fill: parent
            color: root.pageGround
            MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; hoverEnabled: true }
            WheelHandler { onWheel: function(e) { e.accepted = true } }
          }
          MapPage {
            id: mapPage
            anchors.fill: parent
            svc: root.svc; fg: root.foreground
            accent: chatPage.themed ? chatPage.accC : Color.accent
            onBackRequested: root.closeLocationView()
          }
        }
        Item {
          id: audioHolder
          readonly property bool active: root.nav === "audio"
          width: parent.width; height: parent.height; z: 2
          readonly property bool sliding: x > 0.5 && x < width - 0.5
          readonly property real targetX: active ? 0 : width
          visible: x < width - 0.5
          onTargetXChanged: audioHolderSlide.running = true
          Component.onCompleted: x = targetX
          onWidthChanged: if (!active && !audioHolderSlide.running) x = width
          FrameAnimation {
            id: audioHolderSlide
            running: false
            onTriggered: {
              var d = audioHolder.targetX - audioHolder.x
              if (Math.abs(d) < 0.5) { audioHolder.x = audioHolder.targetX; running = false; return }
              var step = d * 0.26
              var floor = Math.max(2, audioHolder.width * 0.03)
              if (Math.abs(step) < floor) step = d > 0 ? floor : -floor
              audioHolder.x += step
            }
          }
          Rectangle {
            anchors.fill: parent
            color: root.pageGround
            MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; hoverEnabled: true }
            WheelHandler { onWheel: function(e) { e.accepted = true } }
          }
          AudioPage {
            id: audioPage
            anchors.fill: parent
            svc: root.svc; fg: root.foreground
            roomId: root.selectedRoomId
            accent: chatPage.themed ? chatPage.accC : Color.accent
            // One track plays at a time; the bubble and the page must agree which.
            playing: chatPage.playingVoice === audioPage.eventId && audioPage.eventId !== ""
            position: chatPage.playedVoice === audioPage.eventId ? chatPage.voicePos : 0
            onBackRequested: root.nav = "chat"
            onToggleRequested: chatPage.toggleVoice({ eventId: audioPage.eventId })
            onSeekRequested: function (secs) { chatPage.playVoiceAt({ eventId: audioPage.eventId }, secs) }
          }
        }
        Item {
          id: searchHolder
          readonly property bool active: root.nav === "search"
          width: parent.width; height: parent.height; z: 2
          readonly property bool sliding: x > 0.5 && x < width - 0.5
          readonly property real targetX: active ? 0 : width
          visible: x < width - 0.5
          onTargetXChanged: searchHolderSlide.running = true
          Component.onCompleted: x = targetX
          onWidthChanged: if (!active && !searchHolderSlide.running) x = width
          FrameAnimation {
            id: searchHolderSlide
            running: false
            onTriggered: {
              var d = searchHolder.targetX - searchHolder.x
              if (Math.abs(d) < 0.5) { searchHolder.x = searchHolder.targetX; running = false; return }
              var step = d * 0.26
              var floor = Math.max(2, searchHolder.width * 0.03)
              if (Math.abs(step) < floor) step = d > 0 ? floor : -floor
              searchHolder.x += step
            }
          }
          Rectangle {
            anchors.fill: parent
            color: root.pageGround
            MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; hoverEnabled: true }
            WheelHandler { onWheel: function(e) { e.accepted = true } }
          }
          SearchPage {
            id: searchPage
            visible: true
            anchors.fill: parent
            svc: root.svc; fg: root.foreground
            roomId: root.selectedRoomId
            onClosed: root.nav = "chat"
            onJumpTo: function(eid) { root.nav = "chat"; Qt.callLater(function() { chatPage.scrollToEvent(eid) }) }
            onOpenImage: function(it) { imageViewer.show(it) }
          }
        }
        Item {
          id: settingsHolder
          readonly property bool active: root.nav === "roomsettings" || (root.nav === "addpeople" && root.addReturn === "roomsettings")
          width: parent.width; height: parent.height; z: 2
          readonly property bool sliding: x > 0.5 && x < width - 0.5
          readonly property real targetX: active ? 0 : width
          visible: x < width - 0.5
          onTargetXChanged: settingsHolderSlide.running = true
          Component.onCompleted: x = targetX
          onWidthChanged: if (!active && !settingsHolderSlide.running) x = width
          FrameAnimation {
            id: settingsHolderSlide
            running: false
            onTriggered: {
              var d = settingsHolder.targetX - settingsHolder.x
              if (Math.abs(d) < 0.5) { settingsHolder.x = settingsHolder.targetX; running = false; return }
              var step = d * 0.26
              var floor = Math.max(2, settingsHolder.width * 0.03)
              if (Math.abs(step) < floor) step = d > 0 ? floor : -floor
              settingsHolder.x += step
            }
          }
          Rectangle {
            anchors.fill: parent
            color: root.pageGround
            MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; hoverEnabled: true }
            WheelHandler { onWheel: function(e) { e.accepted = true } }
          }
          RoomSettingsPage {
            id: settingsPage
            visible: true
            anchors.fill: parent
            svc: root.svc; fg: root.foreground
            roomId: root.selectedRoomId
            onClosed: root.nav = "chat"
            onAddPeople: { addPeoplePage.reset(); root.addReturn = "roomsettings"; root.nav = "addpeople"; Qt.callLater(addPeoplePage.focusSearch) }
            onLeftRoom: root.goHome()
            onOpenNotifications: { root.settingsRoomId = root.selectedRoomId; root.openSettingsChild("notifications", "roomsettings") }
            onOpenSecurity: { root.settingsRoomId = root.selectedRoomId; root.openSettingsChild("security", "roomsettings") }
            onOpenRoles: { root.settingsRoomId = root.selectedRoomId; root.membersReturn = "roles"; root.openSettingsChild("roles", "roomsettings") }
          }
        }
        Item {
          id: addHolder
          readonly property bool active: root.nav === "addpeople"
          width: parent.width; height: parent.height; z: 3
          readonly property bool sliding: x > 0.5 && x < width - 0.5
          readonly property real targetX: active ? 0 : width
          visible: x < width - 0.5
          onTargetXChanged: addHolderSlide.running = true
          Component.onCompleted: x = targetX
          onWidthChanged: if (!active && !addHolderSlide.running) x = width
          FrameAnimation {
            id: addHolderSlide
            running: false
            onTriggered: {
              var d = addHolder.targetX - addHolder.x
              if (Math.abs(d) < 0.5) { addHolder.x = addHolder.targetX; running = false; return }
              var step = d * 0.26
              var floor = Math.max(2, addHolder.width * 0.03)
              if (Math.abs(step) < floor) step = d > 0 ? floor : -floor
              addHolder.x += step
            }
          }
          Rectangle {
            anchors.fill: parent
            color: root.pageGround
            MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; hoverEnabled: true }
            WheelHandler { onWheel: function(e) { e.accepted = true } }
          }
          AddPeoplePage {
            id: addPeoplePage
            visible: true
            anchors.fill: parent
            svc: root.svc; fg: root.foreground
            roomId: root.selectedRoomId
            onClosed: root.nav = root.addReturn
          }
        }
        Item {
          id: themeHolder
          readonly property bool active: root.nav === "chattheme"
          width: parent.width; height: parent.height; z: 2
          readonly property bool sliding: x > 0.5 && x < width - 0.5
          readonly property real targetX: active ? 0 : width
          visible: x < width - 0.5
          onTargetXChanged: themeHolderSlide.running = true
          Component.onCompleted: x = targetX
          onWidthChanged: if (!active && !themeHolderSlide.running) x = width
          FrameAnimation {
            id: themeHolderSlide
            running: false
            onTriggered: {
              var d = themeHolder.targetX - themeHolder.x
              if (Math.abs(d) < 0.5) { themeHolder.x = themeHolder.targetX; running = false; return }
              var step = d * 0.26
              var floor = Math.max(2, themeHolder.width * 0.03)
              if (Math.abs(step) < floor) step = d > 0 ? floor : -floor
              themeHolder.x += step
            }
          }
          Rectangle {
            anchors.fill: parent
            color: root.pageGround
            MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; hoverEnabled: true }
            WheelHandler { onWheel: function(e) { e.accepted = true } }
          }
          ChatThemePage {
            id: themePage
            visible: true
            anchors.fill: parent
            svc: root.svc; fg: root.foreground
            roomId: root.selectedRoomId
            theme: root.chatThemes[root.selectedRoomId] || ({})
            onClosed: root.nav = "chat"
            onApplied: function(t) { root.setChatTheme(root.selectedRoomId, t); root.nav = "chat" }
            onChoosePhoto: root.pickWallpaper()
          }
        }
        Item {
          id: forwardHolder
          readonly property bool active: root.nav === "forward"
          width: parent.width; height: parent.height; z: 2
          readonly property bool sliding: x > 0.5 && x < width - 0.5
          readonly property real targetX: active ? 0 : width
          visible: x < width - 0.5
          onTargetXChanged: forwardHolderSlide.running = true
          Component.onCompleted: x = targetX
          onWidthChanged: if (!active && !forwardHolderSlide.running) x = width
          FrameAnimation {
            id: forwardHolderSlide
            running: false
            onTriggered: {
              var d = forwardHolder.targetX - forwardHolder.x
              if (Math.abs(d) < 0.5) { forwardHolder.x = forwardHolder.targetX; running = false; return }
              var step = d * 0.26
              var floor = Math.max(2, forwardHolder.width * 0.03)
              if (Math.abs(step) < floor) step = d > 0 ? floor : -floor
              forwardHolder.x += step
            }
          }
          Rectangle {
            anchors.fill: parent
            color: root.pageGround
            MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; hoverEnabled: true }
            WheelHandler { onWheel: function(e) { e.accepted = true } }
          }
          ForwardPage {
            id: forwardPage
            visible: true
            anchors.fill: parent
            svc: root.svc; fg: root.foreground
            payload: root.forwardItem
            mode: root.forwardMode
            onClosed: { root.forwardMode = "forward"; root.nav = "chat" }
            onForwarded: function(id) { root.openRoom(id) }
            onPicked: function(id) {
              var paths = root.pendingShare.slice()
              root.forwardMode = "forward"
              root.pendingShare = []
              root.openRoom(id)
              Qt.callLater(function () { chatPage.addAttachments(paths) })
            }
          }
        }
        Item {
          id: startHolder
          readonly property bool active: root.nav === "start"
          width: parent.width; height: parent.height; z: 1
          readonly property bool sliding: x > 0.5 && x < width - 0.5
          readonly property real targetX: active ? 0 : width
          visible: x < width - 0.5
          onTargetXChanged: startHolderSlide.running = true
          Component.onCompleted: x = targetX
          onWidthChanged: if (!active && !startHolderSlide.running) x = width
          FrameAnimation {
            id: startHolderSlide
            running: false
            onTriggered: {
              var d = startHolder.targetX - startHolder.x
              if (Math.abs(d) < 0.5) { startHolder.x = startHolder.targetX; running = false; return }
              var step = d * 0.26
              var floor = Math.max(2, startHolder.width * 0.03)
              if (Math.abs(step) < floor) step = d > 0 ? floor : -floor
              startHolder.x += step
            }
          }
          Rectangle {
            anchors.fill: parent
            color: root.pageGround
            MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; hoverEnabled: true }
            WheelHandler { onWheel: function(e) { e.accepted = true } }
          }
          StartChatPage {
            id: startPage
            visible: true
            anchors.fill: parent
            svc: root.svc; fg: root.foreground
            onClosed: { if (root.spaceForNewRoom !== "") { root.spaceForNewRoom = ""; root.nav = "space" } else root.goHome() }
            onRoomOpened: function(id) {
              // The child link is state on the SPACE, so it takes a second call.
              if (root.spaceForNewRoom !== "") {
                root.svc.addRoomToSpace(root.spaceForNewRoom, id, function () {})
                root.spaceForNewRoom = ""
              }
              root.openRoom(id)
            }
          }
        }
        Item {
          id: chatHolder
          readonly property bool active: root.chatUnder
          width: parent.width; height: parent.height; z: 1
          readonly property bool sliding: x > 0.5 && x < width - 0.5
          readonly property real targetX: active ? 0 : width
          visible: x < width - 0.5
          onTargetXChanged: chatHolderSlide.running = true
          Component.onCompleted: x = targetX
          onWidthChanged: if (!active && !chatHolderSlide.running) x = width
          FrameAnimation {
            id: chatHolderSlide
            running: false
            onTriggered: {
              var d = chatHolder.targetX - chatHolder.x
              if (Math.abs(d) < 0.5) { chatHolder.x = chatHolder.targetX; running = false; return }
              var step = d * 0.26
              var floor = Math.max(2, chatHolder.width * 0.03)
              if (Math.abs(step) < floor) step = d > 0 ? floor : -floor
              chatHolder.x += step
            }
          }
          Rectangle {
            anchors.fill: parent
            color: root.pageGround
            MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; hoverEnabled: true }
            WheelHandler { onWheel: function(e) { e.accepted = true } }
          }
          ChatPage {
            id: chatPage
            tipLayer: tipLayer
            visible: true
            anchors.fill: parent
            svc: root.svc; fg: root.foreground
            roomId: root.selectedRoomId
            sheetOpen: messageSheet.item !== null
            pageSliding: chatHolder.sliding
            visibleToUser: root.opened && root.nav === "chat" && root.page === "main"
            onBackRequested: root.goHome()
            onStartCall: function(video) { if (root.hasService) { root.svc.callStart(root.selectedRoomId, video); root.callPageOpen = true } }
            onJoinCallRequested: root.maximizeCall()
            debugCall: callPage.debugCall
            covered: root.callPageOpen || root.nav !== "chat"
            onAttachRequested: root.pickFiles()
            onOpenThreadRequested: function (rootId) { root.openThread(rootId) }
            onOpenLocation: function(it, from) { root.openLocationView(it, from) }
            onOpenAudio: function(it) { root.openAudio(it) }
            onOpenDmWith: function(uid) {
              if (!root.hasService || !uid) return
              // An existing DM is reused; `dm.create` is idempotent that way.
              root.svc.createDm(uid, function (r, e) {
                if (r && r.roomId) root.openRoom(r.roomId)
              })
            }
            onShareVcf: function(uid, name) { root.shareContactVcf(uid, name) }
            onOpenDocument: function(it) { root.openDocument(it) }
            onOpenImage: function(it, from) { root.dismissSheet(); imageViewer.show(it, from) }
            onPlayVideo: function(it, from) { root.dismissSheet(); root.openVideo(it, from) }
            chatTheme: root.chatThemes[root.selectedRoomId] || ({})
            onCloseSheetRequested: { if (messageSheet.drawerOpen) messageSheet.drawerOpen = false; else messageSheet.close() }
            onNavRequested: function(what) {
              if (what === "search") { searchPage.reset(); root.nav = "search"; Qt.callLater(searchPage.focusSearch) }
              else if (what === "addpeople") { addPeoplePage.reset(); root.addReturn = "chat"; root.nav = "addpeople"; Qt.callLater(addPeoplePage.focusSearch) }
              else if (what === "chattheme") { themePage.reset(); root.nav = "chattheme" }
              else if (what === "threads") { threadsPage.reset(); root.nav = "threads" }
              else if (what === "pins") root.openPins()
              else if (what === "roomsettings") { settingsPage.reset(); root.nav = "roomsettings" }
            }
            onMenuRequested: function(it, x, y, w, h, b) {
              // Refuse when the conversation is not the visible page: a right-click plus
              // a left-click opens the map AND requests the menu, which lands after the nav.
              if (!chatPage.visibleToUser) return
              var p = messageSheet.mapFromItem(null, x, y)
              var lp = messageSheet.mapFromItem(chatPage.listItem, 0, 0)
              messageSheet.vpTop = lp.y
              messageSheet.vpBottom = lp.y + chatPage.listItem.height
              messageSheet.openFor(it, p.x, p.y, w, h, b)
            }
          }
        }
        Item {
          id: threadsHolder
          // Stays under an open thread, so the list does not slide away behind it.
          readonly property bool active: root.nav === "threads" || root.nav === "thread"
          width: parent.width; height: parent.height; z: 3
          readonly property bool sliding: x > 0.5 && x < width - 0.5
          readonly property real targetX: active ? 0 : width
          visible: x < width - 0.5
          onTargetXChanged: threadsHolderSlide.running = true
          Component.onCompleted: x = targetX
          onWidthChanged: if (!active && !threadsHolderSlide.running) x = width
          FrameAnimation {
            id: threadsHolderSlide
            running: false
            onTriggered: {
              var d = threadsHolder.targetX - threadsHolder.x
              if (Math.abs(d) < 0.5) { threadsHolder.x = threadsHolder.targetX; running = false; return }
              var step = d * 0.26
              var floor = Math.max(2, threadsHolder.width * 0.03)
              if (Math.abs(step) < floor) step = d > 0 ? floor : -floor
              threadsHolder.x += step
            }
          }
          Rectangle {
            anchors.fill: parent
            color: root.roomPageGround
            MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; hoverEnabled: true }
            WheelHandler { onWheel: function(e) { e.accepted = true } }
          }
          ThreadsPage {
            id: threadsPage
            anchors.fill: parent
            svc: root.svc
            fg: root.foreground
            roomId: root.selectedRoomId
            chatTheme: root.chatThemes[root.selectedRoomId] || ({})
            onClosed: { root.nav = "chat"; chatPage.loadThreads() }
            onThreadPicked: function (rootId) { root.openThread(rootId) }
          }
        }
        Item {
          id: threadHolder
          readonly property bool active: root.nav === "thread"
          width: parent.width; height: parent.height; z: 3
          readonly property bool sliding: x > 0.5 && x < width - 0.5
          readonly property real targetX: active ? 0 : width
          visible: x < width - 0.5
          onTargetXChanged: threadHolderSlide.running = true
          Component.onCompleted: x = targetX
          onWidthChanged: if (!active && !threadHolderSlide.running) x = width
          FrameAnimation {
            id: threadHolderSlide
            running: false
            onTriggered: {
              var d = threadHolder.targetX - threadHolder.x
              if (Math.abs(d) < 0.5) { threadHolder.x = threadHolder.targetX; running = false; return }
              var step = d * 0.26
              var floor = Math.max(2, threadHolder.width * 0.03)
              if (Math.abs(step) < floor) step = d > 0 ? floor : -floor
              threadHolder.x += step
            }
          }
          Rectangle {
            anchors.fill: parent
            color: root.roomPageGround
            MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; hoverEnabled: true }
            WheelHandler { onWheel: function(e) { e.accepted = true } }
          }
          ChatPage {
            id: threadPage
            tipLayer: tipLayer
            anchors.fill: parent
            svc: root.svc; fg: root.foreground
            roomId: root.threadKey
            sheetOpen: messageSheet.item !== null
            pageSliding: threadHolder.sliding
            visibleToUser: root.opened && root.nav === "thread" && root.page === "main"
            onBackRequested: root.nav = "threads"
            chatTheme: root.chatThemes[root.selectedRoomId] || ({})
            covered: root.callPageOpen || root.nav !== "thread"
            onAttachRequested: root.pickFiles()
            onOpenLocation: function(it, from) { root.openLocationView(it, from) }
            onOpenAudio: function(it) { root.openAudio(it) }
            onShareVcf: function(uid, name) { root.shareContactVcf(uid, name) }
            onOpenDocument: function(it) { root.openDocument(it) }
            onOpenImage: function(it, from) { imageViewer.show(it, from) }
            onPlayVideo: function(it, from) { root.openVideo(it, from) }
            onCloseSheetRequested: { if (messageSheet.drawerOpen) messageSheet.drawerOpen = false; else messageSheet.close() }
            onMenuRequested: function(it, x, y, w, h, b) {
              // Same refusal as the room's sheet above.
              if (!threadPage.visibleToUser) return
              var p = messageSheet.mapFromItem(null, x, y)
              var lp = messageSheet.mapFromItem(threadPage.listItem, 0, 0)
              messageSheet.vpTop = lp.y
              messageSheet.vpBottom = lp.y + threadPage.listItem.height
              messageSheet.openFor(it, p.x, p.y, w, h, b)
            }
          }
        }
        Item {
          id: pinsHolder
          readonly property bool active: root.nav === "pins"
          width: parent.width; height: parent.height; z: 3
          readonly property bool sliding: x > 0.5 && x < width - 0.5
          readonly property real targetX: active ? 0 : width
          visible: x < width - 0.5
          onTargetXChanged: pinsHolderSlide.running = true
          Component.onCompleted: x = targetX
          onWidthChanged: if (!active && !pinsHolderSlide.running) x = width
          FrameAnimation {
            id: pinsHolderSlide
            running: false
            onTriggered: {
              var d = pinsHolder.targetX - pinsHolder.x
              if (Math.abs(d) < 0.5) { pinsHolder.x = pinsHolder.targetX; running = false; return }
              var step = d * 0.26
              var floor = Math.max(2, pinsHolder.width * 0.03)
              if (Math.abs(step) < floor) step = d > 0 ? floor : -floor
              pinsHolder.x += step
            }
          }
          Rectangle {
            anchors.fill: parent
            color: root.roomPageGround
            MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; hoverEnabled: true }
            WheelHandler { onWheel: function(e) { e.accepted = true } }
          }
          PinsPage {
            id: pinsPage
            anchors.fill: parent
            svc: root.svc
            fg: root.foreground
            roomId: root.selectedRoomId
            chatTheme: root.chatThemes[root.selectedRoomId] || ({})
            onClosed: root.nav = "chat"
            onJumpRequested: function (eventId) {
              root.nav = "chat"
              Qt.callLater(function () { chatPage.scrollToEvent(eventId) })
            }
          }
        }
        Item {
          id: callHolder
          readonly property bool active: root.callPageOpen && (root.callHere || callPage.debugCall !== null)
          width: parent.width; height: parent.height; z: 4
          readonly property bool sliding: x > 0.5 && x < width - 0.5
          readonly property real targetX: active ? 0 : width
          visible: x < width - 0.5

          // Minimising scales the whole holder about the PiP's centre, so the call is
          // drawn into the tile rather than cut away; minimizeCall() fixes the origin.
          property bool shrinking: false
          // Snapping the holder to the tile; without it the page animates *down* first.
          property bool instant: false
          property real shrinkOx: 0
          property real shrinkOy: 0
          property real shrinkScale: 0.2
          transform: Scale {
            origin.x: callHolder.shrinkOx
            origin.y: callHolder.shrinkOy
            xScale: callHolder.shrinking ? callHolder.shrinkScale : 1
            yScale: callHolder.shrinking ? callHolder.shrinkScale : 1
            Behavior on xScale { enabled: !callHolder.instant; NumberAnimation { duration: 230; easing.type: Easing.InOutCubic } }
            Behavior on yScale { enabled: !callHolder.instant; NumberAnimation { duration: 230; easing.type: Easing.InOutCubic } }
          }
          // Fades late on the way out, early on the way back in.
          opacity: shrinking ? 0 : 1
          Behavior on opacity {
            enabled: !callHolder.instant
            NumberAnimation { duration: 210; easing.type: callHolder.shrinking ? Easing.InCubic : Easing.OutCubic }
          }
          onTargetXChanged: callHolderSlide.running = true
          Component.onCompleted: x = targetX
          onWidthChanged: if (!active && !callHolderSlide.running) x = width
          FrameAnimation {
            id: callHolderSlide
            running: false
            onTriggered: {
              var d = callHolder.targetX - callHolder.x
              if (Math.abs(d) < 0.5) { callHolder.x = callHolder.targetX; running = false; return }
              var step = d * 0.26
              var floor = Math.max(2, callHolder.width * 0.03)
              if (Math.abs(step) < floor) step = d > 0 ? floor : -floor
              callHolder.x += step
            }
          }
          Rectangle {
            anchors.fill: parent
            color: root.pageGround
            MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; hoverEnabled: true }
            WheelHandler { onWheel: function(e) { e.accepted = true } }
          }
          CallPage {
            id: callPage
            visible: true
            anchors.fill: parent
            svc: root.svc; fg: root.foreground
            accent: chatPage.themed ? chatPage.accC : Color.accent
            onBackRequested: root.callPageOpen = false
            onMinimizeRequested: root.minimizeCall()
            onBeforeScreenshare: root.hideForExternalDialog()
          }
        }

        // Spaces use PageHolder rather than the slide-and-sink shape above. A space IS a
        // room, so the settings pages take `root.settingsRoomId` and serve both.
        PageHolder {
          active: root.nav === "space"
          ground: root.pageGround
          z: 5
          SpacePage {
            id: spacePage
            anchors.fill: parent
            svc: root.svc; fg: root.foreground
            spaceId: root.spaceId
            onClosed: root.goHome()
            onOpenRoom: function (id) { root.openRoom(id) }
            onCreateRoom: { startPage.reset(); root.spaceForNewRoom = root.spaceId; root.nav = "start"; Qt.callLater(startPage.focusSearch) }
            onAddExisting: { root.spaceRoomsMode = "add"; spaceRoomsPage.reset(); root.nav = "spacerooms" }
            onManageRooms: { root.spaceRoomsMode = "manage"; spaceRoomsPage.reset(); root.nav = "spacerooms" }
            onViewMembers: { root.membersFilter = -1; root.settingsRoomId = root.spaceId; membersPage.reset(); root.nav = "members" }
            onOpenSettings: { root.settingsRoomId = root.spaceId; root.nav = "spacesettings" }
            onLeftSpace: root.goHome()
          }
        }
        PageHolder {
          active: root.nav === "newspace"
          ground: root.pageGround
          z: 6
          NewSpacePage {
            id: newSpacePage
            anchors.fill: parent
            svc: root.svc; fg: root.foreground
            onClosed: root.goHome()
            onPickAvatar: root.pickRoomAvatar("")
            onCreated: function (id) { root.openSpace(id) }
          }
        }
        PageHolder {
          active: root.nav === "spacerooms"
          ground: root.pageGround
          z: 7
          SpaceRoomsPage {
            id: spaceRoomsPage
            anchors.fill: parent
            svc: root.svc; fg: root.foreground
            spaceId: root.spaceId
            mode: root.spaceRoomsMode
            onClosed: root.nav = "space"
          }
        }
        PageHolder {
          active: root.nav === "members"
          ground: root.pageGround
          z: 8
          MembersPage {
            id: membersPage
            anchors.fill: parent
            svc: root.svc; fg: root.foreground
            roomId: root.settingsRoomId
            filterLevel: root.membersFilter
            onClosed: root.nav = root.membersReturn
            onInvite: { addPeoplePage.reset(); root.addReturn = "members"; root.nav = "addpeople"; Qt.callLater(addPeoplePage.focusSearch) }
          }
        }
        // A space's own Settings page; a room reaches the same four from RoomSettingsPage.
        PageHolder {
          active: root.nav === "spacesettings"
          ground: root.pageGround
          z: 9
          SpaceSettingsPage {
            id: spaceSettingsPage
            anchors.fill: parent
            svc: root.svc; fg: root.foreground
            roomId: root.settingsRoomId
            onClosed: root.nav = "space"
            onPickAvatar: root.pickRoomAvatar(root.settingsRoomId)
            onOpenNotifications: root.openSettingsChild("notifications", "spacesettings")
            onOpenSecurity: root.openSettingsChild("security", "spacesettings")
            onOpenRoles: root.openSettingsChild("roles", "spacesettings")
            onOpenMembers: { root.membersFilter = -1; root.membersReturn = "spacesettings"; membersPage.reset(); root.nav = "members" }
          }
        }
        PageHolder {
          active: root.nav === "notifications"
          ground: root.pageGround
          z: 10
          NotificationsPage {
            id: notificationsPage
            anchors.fill: parent
            svc: root.svc; fg: root.foreground
            roomId: root.settingsRoomId
            onClosed: root.nav = root.settingsReturn
          }
        }
        PageHolder {
          active: root.nav === "security"
          ground: root.pageGround
          z: 11
          SecurityPage {
            id: securityPage
            anchors.fill: parent
            svc: root.svc; fg: root.foreground
            roomId: root.settingsRoomId
            onClosed: root.nav = root.settingsReturn
          }
        }
        PageHolder {
          active: root.nav === "roles"
          ground: root.pageGround
          z: 12
          RolesPage {
            id: rolesPage
            anchors.fill: parent
            svc: root.svc; fg: root.foreground
            roomId: root.settingsRoomId
            onClosed: root.nav = root.settingsReturn
            onOpenPermissions: { permissionsPage.reset(); root.nav = "permissions" }
            onOpenRole: function (level) { root.membersFilter = level; root.membersReturn = "roles"; membersPage.reset(); root.nav = "members" }
          }
        }
        PageHolder {
          active: root.nav === "permissions"
          ground: root.pageGround
          z: 13
          PermissionsPage {
            id: permissionsPage
            anchors.fill: parent
            svc: root.svc; fg: root.foreground
            roomId: root.settingsRoomId
            onClosed: root.nav = "roles"
          }
        }
      }

      Item {
        id: pagesMask
        anchors.fill: parent
        anchors.topMargin: statusStrip.height
        layer.enabled: true
        layer.smooth: true
        visible: false
        Rectangle { anchors.fill: parent; radius: Style.space(22); antialiasing: true; color: "black" }
      }

      // GM-style message sheet (reactions + actions + emoji drawer)
      MessageSheet {
        id: messageSheet
        anchors.fill: parent
        svc: root.svc; fg: root.foreground; roomId: root.selectedRoomId
        accent: chatPage.themed ? chatPage.accC : Color.accent
        surface: chatPage.themed ? Util.alpha(chatPage.surfaceC, 0.98) : Util.alpha(Color.popups.background, 0.98)
        chip: chatPage.chipC
        deepChip: chatPage.deepChipC
        pagesItem: mainPages
        onAct: function(a, it) { root.messageAction(a, it) }
        onShiftRequested: function(dy) { chatPage.shiftList(dy) }
      }

      // Minimised call.
      CallPiP {
        id: callPip
        visible: (root.callHere || callPage.debugCall !== null) && root.callMinimized && !root.callPageOpen
        // Position is the PiP's own business now — it drags between corners.
        z: 400
        svc: root.svc
        call: callPage.debugCall ? callPage.debugCall : (root.svc ? root.svc.call : ({ state: "idle" }))
        fg: root.foreground
        accent: chatPage.themed ? chatPage.accC : Color.accent
        onExpandRequested: root.maximizeCall()
        onHangupRequested: { if (root.hasService) root.svc.callHangup(); root.callMinimized = false }
      }

      // Hover tips, drawn in the card: the shared PanelToolTip is a QQC
      // ToolTip, which Qt 6.9 puts in its own popup window.
      TipLayer { id: tipLayer; anchors.fill: parent; z: 500; fg: root.foreground }

      Dialogs {
        id: dialogs
        anchors.fill: parent
        scrimRadius: Style.space(22)
        svc: root.svc; fg: root.foreground; onRoomOpened: function(id) { root.openRoom(id) }
      }
      ImageViewer {
        id: imageViewer
        tipLayer: tipLayer
        // Cover the whole card (border included) with the card's own radius.
        anchors.fill: parent
        scrimRadius: Style.space(22)
        pagesItem: mainPages
        accent: chatPage.themed ? chatPage.accC : Color.accent
        svc: root.svc; fg: root.foreground; roomId: root.selectedRoomId
      }
    }
  }

  /// Grow the map page out of the tapped bubble. Same shape as maximizeCall: park the
  /// holder first, *then* resolve the origin, or it comes out a panel width to the right.
  function openLocationView(it, from) {
    if (!it) return
    mapPage.item = it
    if (!from || from.width <= 1) { root.nav = "map"; return }
    mapHolder.instant = true
    mapHolderSlide.running = false
    mapHolder.x = 0
    var c = mapHolder.mapFromItem(null, from.x + from.width / 2, from.y + from.height / 2)
    mapHolder.shrinkOx = c.x
    mapHolder.shrinkOy = c.y
    mapHolder.shrinkScale = Math.max(0.05, from.width / Math.max(1, mapHolder.width))
    mapHolder.shrinking = true
    root.nav = "map"
    mapGrow.restart()
  }
  Timer { id: mapGrow; interval: 16; onTriggered: { mapHolder.instant = false; mapHolder.shrinking = false } }

  /// Closing fades rather than slides, so backing out never yanks the map across.
  function closeLocationView() {
    if (root.nav !== "map") return
    mapHolder.shrinking = true
    mapFade.restart()
  }
  Timer {
    id: mapFade
    interval: 210
    onTriggered: {
      root.nav = "chat"
      mapHolderSlide.running = false
      mapHolder.x = mapHolder.width
      mapHolder.shrinking = false
    }
  }

  /// Open the document reader; shown in its loading state, as a large file is not instant.
  function openDocument(it) {
    if (!it || !root.hasService) return
    docPage.reset()
    docPage.fileName = (it.media && it.media.filename) ? it.media.filename : (it.body || "Document")
    docPage.eventId = it.eventId || ""
    docPage.sizeLabel = (it.media && it.media.sizeLabel) ? it.media.sizeLabel : ""
    root.nav = "doc"
    var want = it.eventId
    root.svc.docPreview(root.selectedRoomId, it.eventId, function(res, err) {
      // The reader may have moved on, or been closed, while this was in flight.
      // Compare against the page's *current* target, not the captured item's own id.
      if (docPage.eventId !== want) return
      if (err) { docPage.status = "error"; docPage.error = err; return }
      docPage.doc = res
      docPage.status = ""
    })
  }

  /// Cover, colour and waveform are computed and cached by the engine per event.
  function openAudio(it) {
    if (!it || !root.hasService) return
    audioPage.eventId = it.eventId || ""
    audioPage.sizeLabel = (it.media && it.media.sizeLabel) ? it.media.sizeLabel : ""
    var n = (it.media && it.media.filename) ? it.media.filename : (it.body || "Audio")
    var dot = n.lastIndexOf(".")
    audioPage.title = dot > 0 ? n.substring(0, dot) : n
    var k = root.svc.docThumbKey(root.selectedRoomId, it.eventId)
    var have = root.svc.audioInfos[k]
    audioPage.info = (have && have !== true) ? have : null
    audioPage.status = audioPage.info ? "" : "loading"
    root.nav = "audio"
    // A track you have not played starts at 0, not where the last one got to.
    if (chatPage.playedVoice !== audioPage.eventId) chatPage.voicePos = 0
    root.svc.audioInfo(root.selectedRoomId, it.eventId, (it.media && it.media.size) || 0)
  }
  Connections {
    target: root.svc
    ignoreUnknownSignals: true
    function onAudioInfoReady(key) {
      if (key !== root.svc.docThumbKey(root.selectedRoomId, audioPage.eventId)) return
      var v = root.svc.audioInfos[key]
      audioPage.info = (v && v !== true) ? v : null
      audioPage.status = audioPage.info ? "" : "error"
    }
  }

  function openVideo(it, from) {
    if (!it) return
    imageViewer.show(it, from)
    Qt.callLater(function() { imageViewer.togglePlayback() })
  }

  function messageAction(a, it) {
    if (!it || !root.hasService) return
    if (a === "reply") { chatPage.replyTo = it.eventId; chatPage.replyName = it.senderName; chatPage.replyBody = it.body || ""; chatPage.editOf = ""; chatPage.focusInput() }
    else if (a === "caption") {
      chatPage.replyTo = ""
      chatPage.captionOf = it.eventId
      chatPage.editOf = ""
      chatPage.replyName = it.media && it.media.filename ? it.media.filename : "Attachment"
      var cap = (it.body && it.media && it.body !== it.media.filename) ? it.body : ""
      chatPage.replyBody = cap
      chatPage.setText(cap)
      chatPage.focusInput()
    }
    else if (a === "edit") { chatPage.replyTo = ""; chatPage.editOf = it.eventId; chatPage.replyBody = it.body || ""; chatPage.setText(it.body); chatPage.focusInput() }
    else if (a === "forward") { root.forwardItem = it; forwardPage.reset(); root.nav = "forward"; Qt.callLater(forwardPage.focusSearch) }
    else if (a === "retry") root.svc.retrySend(root.selectedRoomId, it)
    else if (a === "cancelsend") root.svc.cancelSend(root.selectedRoomId, it)
    else if (a === "react") root.svc.react(root.selectedRoomId, it.eventId, "👍")
    else if (a === "copy") Quickshell.execDetached(["sh", "-c", 'printf "%s" "$1" | wl-copy', "copy", it.body])
    else if (a === "endpoll") { if (it.eventId) root.svc.endPoll(root.selectedRoomId, it.eventId) }
    else if (a === "stoplive") root.svc.stopLiveLocation()
    else if (a === "pin") { if (it.eventId) root.svc.togglePin(root.svc.roomOfKey(root.selectedRoomId), it.eventId) }
    else if (a === "openthread") root.openThread(it.threadRoot || it.eventId)
    else if (a === "thread") root.openThread(it.eventId)
    else if (a === "redact") {
      // A message still in the send queue has no server event id — aborting the
      // queued send is the correct "delete" for it; redaction would no-op.
      if (!it.eventId || it.sendState === "sending" || it.sendState === "failed") root.svc.cancelSend(root.selectedRoomId, it)
      else root.svc.redact(root.selectedRoomId, it.eventId)
    }
  }

  Shortcut {
    sequences: ["Escape"]
    // Application-scoped: the window-scoped form never fires for this layer surface.
    context: Qt.ApplicationShortcut
    enabled: root.opened
    onActivated: root.goBack()
  }

  // The viewer grabs focus while up; hand it back so the next Escape has a target.
  Connections {
    target: imageViewer
    function onItemChanged() { if (!imageViewer.item) Qt.callLater(card.forceActiveFocus) }
  }

  // Test API for driving the live panel from the CLI (omarchy-shell sigilui …).
  IpcHandler {
    target: "sigilui"
    function viewImage(): string { root.debugOpenImage(); return "ok" }
    function closeViewer(): string { imageViewer.close(); return "ok" }
    function voicePanel(on: string): string { chatPage.debugVoicePanel(on === "1"); return "ok" }
    function voiceRecord(): string { chatPage.debugVoiceRecord(); return "ok" }
    function voiceAttach(): string { chatPage.debugVoiceAttach(); return "ok" }
    function voiceClear(): string { chatPage.debugVoiceClear(); return "ok" }
    function voiceState(): string { return chatPage.debugVoiceState() }
    function ctxMenu(x: string, y: string): string { chatPage.debugCtxMenu(Number(x), Number(y)); return "ok" }
    function scroll(notches: string): string { return String(chatPage.debugScroll(Number(notches))) }
    function listState(): string { return chatPage.debugList() }
    function openThreads(): string { threadsPage.reset(); root.nav = "threads"; return "ok" }
    function openPinsPage(): string { root.openPins(); return "ok" }
    function pinJump(idx: string): string {
      var it = pinsPage.items[Number(idx)]
      if (!it) return "no pin at " + idx
      root.nav = "chat"
      Qt.callLater(function () { chatPage.scrollToEvent(it.eventId) })
      return it.eventId
    }
    function threadState(): string {
      var r = chatPage.threadRoots || ({})
      var out = []
      for (var k in r) out.push({ root: k.substring(0, 10), count: r[k].count, latest: r[k].latestBody })
      return JSON.stringify(out)
    }
    function jumpState(): string {
      return JSON.stringify({ jumpedTo: chatPage.jumpedTo, count: chatPage.tl && chatPage.tl.model ? chatPage.tl.model.count : -1,
                              idx: chatPage.indexOfEvent(chatPage.jumpedTo) })
    }
    function pinsState(): string {
      return JSON.stringify({ count: pinsPage.items.length, loaded: pinsPage.loaded,
                              pinned: root.svc ? root.svc.pinnedIds(root.selectedRoomId) : [] })
    }
    function pickThread(idx: string): string {
      var t = threadsPage.threads
      var i = Number(idx)
      if (!t || i >= t.length) return "no thread at " + idx
      root.openThread(t[i].rootId)
      return t[i].rootId
    }
    function pinItem(idx: string): string {
      var m = chatPage.tl ? chatPage.tl.model : null
      if (!m) return "no model"
      var it = m.get(Number(idx))
      if (!it || !it.eventId) return "no eventId at " + idx
      root.svc.togglePin(root.svc.roomOfKey(root.selectedRoomId), it.eventId)
      return it.eventId
    }
    function fakeInvite(on: string): string { chatPage.debugInvite = on === "1"; return String(chatPage.debugInvite) }
    function scrollTrace(on: string): string { return chatPage.debugTrace(on === "1") }
    function noticesOff(on: string): string { chatPage.debugNoNotices = on === "1"; return String(chatPage.debugNoNotices) }
    function cacheTune(mul: string): string { chatPage.cacheMul = Number(mul); return String(chatPage.cacheMul) }
    function wheelTune(step: string, maxSpeed: string, lead: string): string {
      chatPage.wheelStep = Number(step); chatPage.wheelMaxSpeed = Number(maxSpeed); chatPage.wheelLeadScreens = Number(lead)
      return chatPage.wheelStep + "/" + chatPage.wheelMaxSpeed + "/" + chatPage.wheelLeadScreens
    }
    function wheelStep(px: string): string {
      if (px !== "") chatPage.wheelStep = Number(px)
      return String(chatPage.wheelStep)
    }
    function fakeCall(level: string): string {
      if (level === "" || Number(level) < 0) { callPage.debugCall = null; root.callPageOpen = false; return "off" }
      callPage.debugCall = {
        state: "connected", roomId: root.selectedRoomId, since: Date.now() - 65000, encrypted: true,
        local: { participantId: "me", micMuted: false, cameraOn: true, screenSharing: false, speaking: true, level: Number(level),
                 tracks: [ { key: "local-cam", kind: "camera", shmPath: "", width: 640, height: 480 } ] },
        participants: [ { participantId: "peer", userId: "@peer:example.com", displayName: "Test Peer",
                          micMuted: false, cameraOn: false, screenSharing: false, speaking: true,
                          level: Number(level), quality: "good", tracks: [] } ]
      }
      root.callPageOpen = true
      return "on"
    }
    /// A synthetic group call: `n` remote participants, optionally one sharing a screen.
    function callReactTray(on: string): string { callPage.reactOpen = on === "1"; return String(callPage.reactOpen) }
    function fakeReaction(emoji: string, who: string): string { callPage.addFloater(emoji, who); return "ok" }
    function fakeGroup(n: string, share: string): string {
      var count = Math.max(0, Number(n))
      var names = ["Alice", "Bob", "Carol", "Dave", "Status Bot", "Test Peer"]
      var ids = ["@alice", "@bob", "@carol", "@dave", "@status-bot", "@peer"]
      var ps = []
      for (var i = 0; i < count; i++) {
        var sharing = share === "1" && i === 0
        ps.push({ participantId: "p" + i, userId: ids[i % ids.length] + ":example.com",
                  displayName: names[i % names.length], micMuted: i % 3 === 1, cameraOn: false,
                  screenSharing: sharing, speaking: i === 1, level: i === 1 ? 0.7 : 0.05, quality: "good",
                  tracks: sharing ? [ { key: "sc" + i, kind: "screen", shmPath: "", width: 1920, height: 1080 } ] : [] })
      }
      callPage.debugCall = {
        state: "connected", roomId: root.selectedRoomId, since: Date.now() - 185000, encrypted: true,
        local: { participantId: "me", micMuted: false, cameraOn: true, screenSharing: share === "2", speaking: false, level: 0.1,
                 tracks: share === "2"
                   ? [ { key: "local-cam", kind: "camera", shmPath: "", width: 640, height: 480 },
                       { key: "local-screen", kind: "screen", shmPath: "", width: 1920, height: 1080 } ]
                   : [ { key: "local-cam", kind: "camera", shmPath: "", width: 640, height: 480 } ] },
        participants: ps
      }
      root.callPageOpen = true
      return "group of " + (count + 1)
    }
    function pipThrow(vx: string, vy: string): string { return callPage.debugPipThrow(Number(vx), Number(vy)) }
    function pipPos(): string { return callPage.debugPipPos() }
    function draft(text: string): string { chatPage.setText(text); return "ok" }
    function viewerMenu(): string { imageViewer.debugMoreMenu(); return "ok" }
    function viewerZoom(z: string, fx: string, fy: string): string { return imageViewer.debugZoom(Number(z), Number(fx), Number(fy)) }
    function viewerZoomReset(): string { return imageViewer.debugZoomReset() }
    function back(): string { root.goBack(); return root.opened ? root.nav : "closed" }
    /// `goto space` takes the first space when none is named.
    function goto(page: string, arg: string): string {
      if (page === "space") {
        var id = arg
        if (id === "" && root.hasService && root.svc.spaces.length > 0) id = root.svc.spaces[0].id
        if (id === "") return "no spaces"
        root.openSpace(id)
        return root.nav + " " + id
      }
      if (page === "newspace") { newSpacePage.reset(); root.nav = "newspace"; return root.nav }
      if (page === "spacerooms") { root.spaceRoomsMode = arg === "add" ? "add" : "manage"; spaceRoomsPage.reset(); root.nav = "spacerooms"; return root.nav + " " + root.spaceRoomsMode }
      if (page === "spacesettings") { root.settingsRoomId = root.spaceId; spaceSettingsPage.reset(); root.nav = "spacesettings"; return root.nav }
      if (page === "members") { root.membersFilter = arg === "" ? -1 : Number(arg); membersPage.reset(); root.nav = "members"; return root.nav }
      if (page === "notifications" || page === "security" || page === "roles" || page === "permissions") {
        if (root.settingsRoomId === "") root.settingsRoomId = root.selectedRoomId
        root.nav = page
        return root.nav + " " + root.settingsRoomId
      }
      if (page === "home") { root.goHome(); return root.nav }
      root.nav = page
      return root.nav
    }
    function spaceMenu(on: string): string { spacePage.menuOpen = (on === "1"); return String(spacePage.menuOpen) }
    /// Exercise the settings pages' write paths: QML → Service → engine.
    function notifMode(m: string): string { notificationsPage.set(m); return "ok" }
    function permLevel(key: string, level: string): string { permissionsPage.pendingKey = key; permissionsPage.apply(Number(level)); return "ok" }
    function navState(): string {
      return JSON.stringify({ nav: root.nav, spaceId: root.spaceId, settingsRoomId: root.settingsRoomId,
                              settingsReturn: root.settingsReturn, membersReturn: root.membersReturn,
                              membersFilter: root.membersFilter, spaceRoomsMode: root.spaceRoomsMode })
    }
    function pipControls(on: string): string { callPip.controlsOn = on === "1"; return "ok" }
    function fakeReact(keys: string, idx: string): string { return chatPage.debugReact(keys, idx) }
    function minimizeCall(): string { root.minimizeCall(); return "ok" }
    function maximizeCall(): string { return String(root.maximizeCall()) }
    function composerCovered(): string { return String(chatPage.covered) }
    function pipFling(vx: string, vy: string): string { return callPip.debugFling(Number(vx), Number(vy)) }
    function pipWhere(): string { return callPip.debugPos() }
    function tapMedia(): string { return chatPage.debugTapMedia() }
    function tapLocation(which: string): string { return chatPage.debugTapLocation(which) }
    function mapProbe(): string { return mapPage.debugMap() }
    function bubbleMaps(on: string): string { root.svc.debugNoBubbleMaps = on !== "1"; return String(!root.svc.debugNoBubbleMaps) }
    function mapInput(): string { return mapPage.debugEvents() }
    function mapPan(dx: string, dy: string): string { return mapPage.debugPan(Number(dx), Number(dy)) }
    function mapZoom(d: string): string { return mapPage.debugZoom(Number(d)) }
    function mapReset(): string { return mapPage.debugReset() }
    function mapTrace(on: string): string { return mapPage.debugTrace(on === "1") }
    function mapTraceRead(): string { return mapPage.debugTraceRead() }
    function mapInset(px: string): string { return mapPage.debugInset(Number(px)) }
    function fontState(): string { return Fonts.debugState() }
    function mapIsolate(n: string): string { return mapPage.debugIsolate(Number(n)) }
    function mapPanMode(m: string): string { return mapPage.debugPanMode(m) }
    function shareContact(uid: string): string { root.shareContactVcf(uid, ""); return "ok" }
    function sharePick(roomId: string): string { forwardPage.doForward(roomId); return "ok" }
    function player(): string { return audioPage.debugPlayer() }
    function playerToggle(): string { audioPage.toggleRequested(); return "ok" }
    function playerSeek(secs: string): string { audioPage.seekRequested(Number(secs)); return "ok" }
    function tapAudio(): string { return chatPage.debugTapAudio() }
    function pickerProbe(): string { return chatPage.debugPicker() }
    function tapDoc(which: string): string { return chatPage.debugTapDoc(which) }
    function sheetKind(kind: string): string { return String(root.debugSheetKind(kind)) }
    function sheet(drawer: string): string { root.debugOpenSheet(drawer === "1", ""); return "ok" }
    function confirmDelete(): string { root.debugConfirmDelete(); return "ok" }
    function closeSheet(): string { messageSheet.confirmItem = null; messageSheet.drawerOpen = false; messageSheet.close(); return "ok" }
    function fakeMessage(own: string, text: string): string { return chatPage.debugFakeMessage(own, text) }
    function itemInfo(idx: string): string { return chatPage.debugItemInfo(idx) }
    function readers(): string { return chatPage.debugReaders() }
    function geom(idx: string): string { return chatPage.debugGeomAt(idx) }
    function replayEntry(idx: string): string { return chatPage.debugReplayEntry(idx) }
    function tapDetails(): string { return chatPage.debugTapNewest() }
    function details(on: string): string { chatPage.debugDetailsAll = on === "1"; return "ok" }
    function jumpTo(eid: string): string { chatPage.scrollToEvent(eid); return "ok" }
    function stageContact(uid: string): string {
      chatPage.pendingContact = uid === "" ? null : { userId: uid, displayName: uid.split(":")[0].replace("@", ""), avatarUrl: "", avatarPath: "" }
      return "ok"
    }
    function stageFiles(paths: string): string { chatPage.addAttachments(String(paths).split(",").filter(function(x) { return x !== "" })); return "ok" }
    function attachMenu(page: string): string { chatPage.debugAttach(page); return "ok" }
    function fakeTyping(on: string): string { chatPage.debugTyping(on === "1"); return "ok" }
    function homeTab(i: string): string { homePage.tab = Number(i); homePage.spaceFilter = ""; return String(homePage.tab) }
    function enterSpace(idx: string): string {
      var sp = homePage.spaceRows[Number(idx)]
      if (!sp) return "no space at " + idx
      homePage.spaceFilter = sp.id
      homePage.spaceFilterName = sp.name || sp.id
      homePage.tab = 0
      return sp.name
    }
    function account(on: string): string { homePage.accountOpen = on === "1"; return "ok" }
    function openRoomByName(name: string): string {
      if (!root.hasService) return "no service"
      for (var i = 0; i < root.svc.rooms.length; i++) {
        var r = root.svc.rooms[i]
        if ((r.name || "").toLowerCase().indexOf(name.toLowerCase()) >= 0) { root.openRoom(r.id); return r.id }
      }
      return "not found"
    }
  }

  // Auto-open the call page on the idle→active transition while the panel is open.
  property string _lastCallState: "idle"
  Connections {
    target: root.svc
    function onCallChanged() {
      var st = root.svc.call ? root.svc.call.state : "idle"
      var active = st === "joining" || st === "connected" || st === "reconnecting"
      var wasActive = root._lastCallState === "joining" || root._lastCallState === "connected" || root._lastCallState === "reconnecting"
      if (root.opened && active && !wasActive) { root.callPageOpen = true; root.callMinimized = false }
      if (!active) { root.callPageOpen = false; root.callMinimized = false }
      root._lastCallState = st
    }
  }
}
