import QtQuick
import Quickshell
import qs.Commons
import qs.Ui
import "../components"
import "../calls"
import ".."

// In-panel call experience: voice = big avatar card; video = remote fills the
// page with self-view PiP; screen share gets the full page. Round controls.
Item {
  id: root
  property var svc: null
  property color fg: Color.menu.text
  property color accent: Color.accent   // the room's chat theme colour
  signal backRequested()
  signal minimizeRequested()
  signal beforeScreenshare()
  property var debugCall: null
  readonly property var call: root.debugCall ? root.debugCall : (svc ? svc.call : ({ state: "idle" }))
  readonly property var remotes: call && call.participants ? call.participants : []
  readonly property var firstRemote: remotes.length > 0 ? remotes[0] : null
  /// In-call reactions, over the LiveKit data channel, not the Matrix room.
  property bool reactOpen: false
  onVisibleChanged: if (!visible) { root.reactOpen = false; floaterModel.clear() }
  /// The same six the message quick-react pill offers.
  readonly property var reactEmoji: ["👍", "❤️", "😂", "😮", "😢", "😡"]
  /// Live floaters. A ListModel, NOT a JS array: reassigning an array is a model
  /// reset, which destroys every in-flight delegate and replays its animation.
  ListModel { id: floaterModel }
  property int floaterSeq: 0
  function addFloater(emoji, who) {
    floaterModel.append({ fid: ++root.floaterSeq, emoji: emoji, who: who || "" })
    while (floaterModel.count > 12) floaterModel.remove(0)
  }
  function dropFloater(id) {
    for (var i = 0; i < floaterModel.count; i++) {
      if (floaterModel.get(i).fid === id) { floaterModel.remove(i); return }
    }
  }
  Connections {
    target: root.svc
    ignoreUnknownSignals: true
    function onCallReaction(emoji, who, own) {
      root.addFloater(emoji, who)
      // Only for other people's reactions; your own is confirmed by the tap.
      if (!own) root.playReactionSound()
    }
  }
  /// Through PipeWire, not the engine's audio path: the engine owns the call's ADM.
  function playReactionSound() {
    Quickshell.execDetached(["sh", "-c",
      "pw-play --volume=0.4 /usr/share/sounds/freedesktop/stereo/message.oga 2>/dev/null" +
      " || paplay --volume=26214 /usr/share/sounds/freedesktop/stereo/message.oga 2>/dev/null || true"])
  }

  property real now: Date.now()   // int is 32-bit; an epoch ms overflows it
  Timer { interval: 1000; running: root.visible; repeat: true; onTriggered: root.now = Date.now() }

  // Featured video track: remote screen > remote camera. A camera-muted participant
  // stays featured, or the layout flips on every camera toggle.
  readonly property var featured: {
    for (var i = 0; i < remotes.length; i++) {
      var t = remotes[i].tracks || []
      for (var k = 0; k < t.length; k++) if (t[k].kind === "screen") return { p: remotes[i], t: t[k] }
    }
    for (var j = 0; j < remotes.length; j++) {
      var t2 = remotes[j].tracks || []
      for (var m = 0; m < t2.length; m++) if (t2[m].kind === "camera") return { p: remotes[j], t: t2[m] }
    }
    return null
  }
  readonly property var localCam: {
    var t = (call && call.local && call.local.tracks) || []
    for (var i = 0; i < t.length; i++) if (t[i].kind === "camera") return t[i]
    return null
  }

  // A video call stays one when the far camera goes off; the tile shows a face.
  readonly property bool videoMode: root.featured !== null || root.localCam !== null

  /// Everyone in the call, self last; no video still gets an avatar tile.
  readonly property var gridTiles: {
    var out = []
    for (var i = 0; i < root.remotes.length; i++) {
      var p = root.remotes[i]
      var tks = p.tracks || []
      var best = null
      for (var k = 0; k < tks.length; k++) {
        if (tks[k].kind === "screen") { best = tks[k]; break }
        if (tks[k].kind === "camera" && !best) best = tks[k]
      }
      out.push({ participant: p, track: best, isLocal: false })
    }
    var lt = (root.call && root.call.local && root.call.local.tracks) || []
    var lbest = null
    for (var j = 0; j < lt.length; j++) {
      if (lt[j].kind === "screen") { lbest = lt[j]; break }
      if (lt[j].kind === "camera" && !lbest) lbest = lt[j]
    }
    out.push({ participant: root.selfParticipant, track: lbest, isLocal: true })
    return out
  }
  readonly property var selfParticipant: ({
    displayName: "You",
    userId: root.svc ? root.svc.userId : "",
    avatarPath: root.svc ? root.svc.avatarPath : "",
    micMuted: root.call.local ? !!root.call.local.micMuted : false,
    cameraOn: root.call.local ? !!root.call.local.cameraOn : false,
    speaking: root.call.local ? !!root.call.local.speaking : false,
    level: root.call.local ? (root.call.local.level || 0) : 0,
    quality: "good"
  })
  readonly property bool anyScreenShare: {
    for (var i = 0; i < root.gridTiles.length; i++) {
      var t = root.gridTiles[i].track
      if (t && t.kind === "screen") return true
    }
    return false
  }
  /// A share needs the spotlight layout even in a two-person call.
  readonly property bool groupMode: root.remotes.length > 1 || root.anyScreenShare

  // Minimising scales the page about the PiP's centre; `clip` hides the settings sheet below.
  clip: true
  property bool shrinking: false
  property real shrinkOx: 0
  property real shrinkOy: 0
  property real shrinkScale: 0.2
  transform: Scale {
    origin.x: root.shrinkOx
    origin.y: root.shrinkOy
    xScale: root.shrinking ? root.shrinkScale : 1
    yScale: root.shrinking ? root.shrinkScale : 1
    Behavior on xScale { NumberAnimation { duration: 230; easing.type: Easing.InOutCubic } }
    Behavior on yScale { NumberAnimation { duration: 230; easing.type: Easing.InOutCubic } }
  }
  // Fades late, so most of the travel is still visible.
  opacity: root.shrinking ? 0 : 1
  Behavior on opacity { NumberAnimation { duration: 210; easing.type: Easing.InCubic } }

  // Rounded to match the panel card, else the corners poke out as dark tips.
  Rectangle { anchors.fill: parent; radius: Style.space(22); antialiasing: true; color: Util.alpha(Color.background, 0.35) }

  // Minimise to the app-wide PiP; sits where a back arrow would be.
  Rectangle {
    z: 40
    anchors.top: parent.top; anchors.left: parent.left
    anchors.topMargin: Style.space(14); anchors.leftMargin: Style.space(14)
    width: Style.space(34); height: width; radius: width / 2
    color: Util.alpha(Color.background, 0.55)
    IconLabel { anchors.centerIn: parent; icon: Icons.pip; color: root.fg; filled: true; size: Style.font.icon }
    MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.minimizeRequested() }
  }

  // Featured remote video (or avatar backdrop for voice)
  Item {
    anchors.fill: parent
    anchors.bottomMargin: controls.height

    ParticipantTile {
      visible: root.videoMode && !root.groupMode
      anchors.fill: parent
      anchors.margins: Style.space(10)
      tileRadius: Style.space(20)
      participant: root.featured ? root.featured.p : root.firstRemote
      track: root.featured ? root.featured.t : null
      fitVideo: root.featured !== null && root.featured.t.kind === "screen"
      fg: root.fg; accent: root.accent
    }

    CallGrid {
      visible: root.groupMode
      anchors.fill: parent
      // 2 here + the grid's own 8 = the 10 a single featured tile uses.
      anchors.margins: Style.space(2)
      tiles: root.groupMode ? root.gridTiles : []
      fg: root.fg; accent: root.accent
    }

    // Voice-call / waiting layout
    Column {
      // Group mode has its own layout: an all-cameras-off group call leaves `videoMode` false.
      visible: !root.videoMode && !root.groupMode
      anchors.centerIn: parent
      spacing: Style.space(14)
      Item {
        anchors.horizontalCenter: parent.horizontalCenter
        width: Style.space(110); height: Style.space(110)
        SpeakingRipple {
          anchors.centerIn: parent
          size: Style.space(110)
          accent: root.accent
          speaking: !!(root.firstRemote && root.firstRemote.speaking)
          level: root.firstRemote && root.firstRemote.level !== undefined ? root.firstRemote.level : 0
        }
        Avatar {
          anchors.centerIn: parent
          size: Style.space(110)
          source: root.firstRemote ? (root.firstRemote.avatarPath || "") : (root.roomInfo ? root.roomInfo.avatarPath : "")
          name: root.firstRemote ? root.firstRemote.displayName : (root.roomInfo ? root.roomInfo.name : "")
          userId: root.firstRemote ? root.firstRemote.userId : ""
        }
      }
      Text { anchors.horizontalCenter: parent.horizontalCenter; text: root.firstRemote ? root.firstRemote.displayName : (root.roomInfo ? root.roomInfo.name : ""); color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.heading; font.bold: true }
      Row {
        anchors.horizontalCenter: parent.horizontalCenter; spacing: Style.space(6)
        Text { text: root.statusText; color: Util.alpha(root.fg, 0.6); font.family: Fonts.ui; font.pixelSize: Style.font.body }
        IconLabel { visible: !!root.call.encrypted; icon: Icons.lock; color: Util.alpha(root.fg, 0.5); anchors.verticalCenter: parent.verticalCenter; filled: true; size: Style.font.bodySmall }
      }
    }

    // Self-view PiP: draggable, rounded (rounding done by ParticipantTile)
    Item {
      id: pip
      // In group mode you are already a tile in the grid.
      visible: root.localCam !== null && !root.groupMode
      width: Style.space(110); height: Style.space(150)
      x: parent.width - width - Style.space(16)
      y: Style.space(16)

      // Drag velocity feeds a spring toward the nearest corner.
      property real vx: 0
      property real vy: 0
      property real targetX: x
      property real targetY: y
      property bool flying: false
      readonly property real edge: Style.space(16)

      function cornerFor(px, py) {
        return {
          x: (px + pip.width / 2 < pip.parent.width / 2) ? pip.edge : pip.parent.width - pip.width - pip.edge,
          y: (py + pip.height / 2 < pip.parent.height / 2) ? pip.edge : pip.parent.height - pip.height - pip.edge
        }
      }

      FrameAnimation {
        id: pipTrack
        running: pipDrag.drag.active
        property real lastX: 0
        property real lastY: 0
        onRunningChanged: { lastX = pip.x; lastY = pip.y; pip.vx = 0; pip.vy = 0 }
        onTriggered: {
          var dt = Math.max(0.001, frameTime)
          // Low-passed, so one jittery frame cannot define the throw.
          pip.vx = pip.vx * 0.55 + ((pip.x - lastX) / dt) * 0.45
          pip.vy = pip.vy * 0.55 + ((pip.y - lastY) / dt) * 0.45
          lastX = pip.x; lastY = pip.y
        }
      }

      FrameAnimation {
        id: pipSpring
        running: pip.flying
        onTriggered: {
          var dt = Math.min(0.033, Math.max(0.001, frameTime))
          var k = 120       // spring stiffness
          var c = 15        // damping
          pip.vx += ((pip.targetX - pip.x) * k - pip.vx * c) * dt
          pip.vy += ((pip.targetY - pip.y) * k - pip.vy * c) * dt
          pip.x += pip.vx * dt
          pip.y += pip.vy * dt
          // Bounce off the edges instead of sliding through them.
          var minX = pip.edge, maxX = pip.parent.width - pip.width - pip.edge
          var minY = pip.edge, maxY = pip.parent.height - pip.height - pip.edge
          if (pip.x < minX) { pip.x = minX; pip.vx = Math.abs(pip.vx) * 0.3 }
          else if (pip.x > maxX) { pip.x = maxX; pip.vx = -Math.abs(pip.vx) * 0.3 }
          if (pip.y < minY) { pip.y = minY; pip.vy = Math.abs(pip.vy) * 0.3 }
          else if (pip.y > maxY) { pip.y = maxY; pip.vy = -Math.abs(pip.vy) * 0.3 }
          if (Math.abs(pip.vx) < 8 && Math.abs(pip.vy) < 8
              && Math.abs(pip.targetX - pip.x) < 0.6 && Math.abs(pip.targetY - pip.y) < 0.6) {
            pip.x = pip.targetX; pip.y = pip.targetY
            pip.vx = 0; pip.vy = 0; pip.flying = false
          }
        }
      }
      ParticipantTile {
        anchors.fill: parent
        participant: ({ displayName: "You", userId: root.svc ? root.svc.userId : "", avatarPath: root.svc ? root.svc.avatarPath : "", micMuted: root.call.local ? root.call.local.micMuted : false, cameraOn: root.call.local ? root.call.local.cameraOn : true, speaking: root.call.local ? !!root.call.local.speaking : false, level: root.call.local ? (root.call.local.level || 0) : 0, quality: "good" })
        track: root.localCam
        isLocal: true
        fg: root.fg; accent: root.accent
        tileRadius: Style.space(16)
      }
      MouseArea {
        id: pipDrag
        anchors.fill: parent
        cursorShape: pressed ? Qt.ClosedHandCursor : Qt.OpenHandCursor
        drag.target: pip
        drag.minimumX: Style.space(6); drag.maximumX: pip.parent.width - pip.width - Style.space(6)
        drag.minimumY: Style.space(6); drag.maximumY: pip.parent.height - pip.height - Style.space(6)
        onReleased: {
          // Aim at where the throw is heading, not where the finger let go.
          var c = pip.cornerFor(pip.x + pip.vx * 0.14, pip.y + pip.vy * 0.14)
          pip.targetX = c.x
          pip.targetY = c.y
          pip.flying = true
        }
      }
    }

    Rectangle {
      visible: (root.videoMode || root.groupMode) && root.call.state === "connected"
      anchors.top: parent.top; anchors.horizontalCenter: parent.horizontalCenter; anchors.topMargin: Style.space(16)
      width: tchip.implicitWidth + Style.space(18); height: Style.space(24); radius: height / 2
      color: Util.alpha(Color.background, 0.55)
      Text { id: tchip; anchors.centerIn: parent; text: root.statusText; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.caption }
    }
  }


  // Page-owned, not tile-owned: the sender may have no tile on screen.
  Item {
    id: floatLayer
    anchors.fill: parent
    anchors.bottomMargin: controls.height
    // No z: declaration order stacks this below the controls and the sheet.
    Repeater {
      model: floaterModel
      delegate: Item {
        id: fl
        required property int fid
        required property string emoji
        required property string who
        width: Style.space(52); height: Style.space(52)
        // Lane seeded from the id so a burst spreads out, captured once so the sway has a fixed point.
        readonly property real lane: floatLayer.width * (0.22 + ((fl.fid * 37) % 100) / 100 * 0.56) - width / 2
        x: fl.lane
        y: floatLayer.height - height - Style.space(10)
        opacity: 0
        Column {
          anchors.centerIn: parent
          spacing: Style.space(1)
          Text {
            anchors.horizontalCenter: parent.horizontalCenter
            // No font.family: the colour-emoji font has to resolve.
            text: fl.emoji
            font.pixelSize: Style.space(30)
          }
          // A chip, not bare white: white is unreadable over a bright frame.
          Rectangle {
            anchors.horizontalCenter: parent.horizontalCenter
            visible: whoText.text !== ""
            width: Math.min(whoText.implicitWidth + Style.space(12), Style.space(110))
            height: Style.space(18); radius: height / 2
            color: Util.alpha(Color.background, 0.6)
            Text {
              id: whoText
              anchors.centerIn: parent
              width: Math.min(implicitWidth, parent.width - Style.space(10))
              elide: Text.ElideRight
              text: fl.who
              color: "#ececec"
              font.family: Fonts.ui; font.pixelSize: Style.font.caption
            }
          }
        }
        // Up and fading, with a sway; a straight line reads as a progress bar.
        ParallelAnimation {
          running: true
          NumberAnimation { target: fl; property: "y"; to: floatLayer.height * 0.22; duration: 2600; easing.type: Easing.OutCubic }
          SequentialAnimation {
            NumberAnimation { target: fl; property: "opacity"; from: 0; to: 1; duration: 220 }
            PauseAnimation { duration: 1600 }
            NumberAnimation { target: fl; property: "opacity"; to: 0; duration: 780 }
          }
          SequentialAnimation {
            NumberAnimation { target: fl; property: "x"; to: fl.lane + Style.space(16); duration: 900; easing.type: Easing.InOutSine }
            NumberAnimation { target: fl; property: "x"; to: fl.lane - Style.space(12); duration: 1000; easing.type: Easing.InOutSine }
          }
          onFinished: root.dropFloater(fl.fid)
        }
      }
    }
  }

  // Page level, not in the voice column: that column hides in group mode.
  Text {
    visible: !!root.call.error
    anchors.horizontalCenter: parent.horizontalCenter
    anchors.bottom: controls.top
    anchors.bottomMargin: Style.space(10)
    width: root.width - Style.space(60)
    wrapMode: Text.Wrap
    horizontalAlignment: Text.AlignHCenter
    text: root.call.error || ""
    color: Color.urgent
    font.family: Fonts.ui; font.pixelSize: Style.font.caption
  }

  // On the PAGE, not inside `controls`: there it covers the buttons and each needs two taps.
  MouseArea {
    anchors.fill: parent
    enabled: root.reactOpen
    onClicked: root.reactOpen = false
  }

  // Controls: dark circles, inverted when off, red hangup pill.
  Item {
    id: controls
    anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom
    height: Style.space(84)
    // Emoji tray above the controls: a short row, not the full picker.
    Rectangle {
      id: reactTray
      visible: opacity > 0.01
      opacity: root.reactOpen ? 1 : 0
      Behavior on opacity { NumberAnimation { duration: 140 } }
      anchors.horizontalCenter: parent.horizontalCenter
      anchors.bottom: parent.top
      anchors.bottomMargin: Style.space(8)
      // The house quick-react pill's metrics, on the control row's exact fill.
      width: trayRow.implicitWidth + Style.space(20)
      height: Style.space(48)
      radius: height / 2
      antialiasing: true
      color: Qt.rgba(0.16, 0.17, 0.18, 0.92)
      scale: root.reactOpen ? 1 : 0.9
      Behavior on scale { NumberAnimation { duration: 180; easing.type: Easing.OutBack; easing.overshoot: 1.6 } }
      // Swallow taps on the pill's own padding, or a near-miss dismisses it.
      MouseArea { anchors.fill: parent }
      Row {
        id: trayRow
        anchors.centerIn: parent
        spacing: Style.space(6)
        Repeater {
          model: root.reactEmoji
          delegate: Rectangle {
            required property var modelData
            width: Style.space(36); height: Style.space(36); radius: width / 2
            color: eh.containsMouse ? Util.alpha(root.accent, 0.32) : "transparent"
            // 1.18 overlaps neighbours at this pitch and overflows the pill.
            scale: eh.containsMouse ? 1.08 : 1
            Behavior on scale { NumberAnimation { duration: 110; easing.type: Easing.OutCubic } }
            TextMetrics { id: em3; font.pixelSize: Style.space(19); text: modelData }
            Text {
              // No font.family (the colour-emoji font must resolve), positioned from the ink.
              anchors.verticalCenter: parent.verticalCenter
              x: (parent.width - em3.tightBoundingRect.width) / 2 - em3.tightBoundingRect.x
              text: modelData
              font.pixelSize: Style.space(19)
            }
            MouseArea {
              id: eh
              anchors.fill: parent
              hoverEnabled: true
              cursorShape: Qt.PointingHandCursor
              onClicked: {
                if (root.svc) root.svc.callReact(modelData)
                root.reactOpen = false
              }
            }
          }
        }
      }
    }

    Row {
      anchors.centerIn: parent
      spacing: Style.space(12)
      Repeater {
        model: [
          { icon: (root.call.local && root.call.local.micMuted) ? Icons.micOff : Icons.micOn, off: !!(root.call.local && root.call.local.micMuted), active: false, inert: false, a: "mic" },
          { icon: (root.call.local && root.call.local.cameraOn) ? Icons.videoOn : Icons.videoOff, off: !(root.call.local && root.call.local.cameraOn), active: false, inert: false, a: "camera" },
          { icon: Icons.screenShareAlt, off: false, active: !!(root.call.local && root.call.local.screenSharing), inert: false, a: "screen" },
          { icon: Icons.emoji, off: false, active: root.reactOpen, inert: false, a: "react" },
          { icon: Icons.settings, off: false, active: false, inert: false, a: "settings" }
        ]
        delegate: Rectangle {
          required property var modelData
          width: Style.space(46); height: Style.space(46); radius: height / 2
          color: modelData.off ? "#e8e8e8" : (modelData.active ? Util.alpha(Color.accent, 0.85) : Qt.rgba(0.16, 0.17, 0.18, 0.92))
          opacity: modelData.inert ? 0.55 : 1
          IconLabel { anchors.centerIn: parent; icon: modelData.icon; color: modelData.off ? "#202124" : "#ececec"; filled: true; size: Style.space(19) }
          MouseArea {
            anchors.fill: parent; cursorShape: modelData.inert ? Qt.ArrowCursor : Qt.PointingHandCursor
            onClicked: {
              if (!root.svc || modelData.inert) return
              if (modelData.a === "mic") root.svc.callSetMic(!!(root.call.local && root.call.local.micMuted))
              else if (modelData.a === "camera") root.svc.callSetCamera(!(root.call.local && root.call.local.cameraOn))
              else if (modelData.a === "screen") { if (!(root.call.local && root.call.local.screenSharing)) root.beforeScreenshare(); root.svc.callScreenshare(!(root.call.local && root.call.local.screenSharing)) }
              else if (modelData.a === "react") root.reactOpen = !root.reactOpen
              else if (modelData.a === "settings") root.settingsOpen = true
            }
          }
        }
      }
      Rectangle {
        width: Style.space(64); height: Style.space(46); radius: height / 2
        color: Util.alpha(Color.urgent, 0.95)
        Text { anchors.centerIn: parent; text: Icons.phone; rotation: 135; color: "#ffffff"; font.family: Fonts.iconFilled; renderType: Text.NativeRendering; font.pixelSize: Style.space(20) }
        MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: { if (root.svc) root.svc.callHangup(); root.backRequested() } }
      }
    }
  }

  // Settings sheet: input/output devices, slides in from the bottom
  property bool settingsOpen: false
  onSettingsOpenChanged: if (settingsOpen && root.svc) root.svc.refreshDevices()
  Timer { id: devRefresh; interval: 600; repeat: false; onTriggered: if (root.svc) root.svc.refreshDevices() }
  Rectangle {
    anchors.fill: parent; radius: Style.space(22); antialiasing: true; color: "#000000"
    opacity: root.settingsOpen ? 0.35 : 0
    visible: opacity > 0
    Behavior on opacity { NumberAnimation { duration: 180 } }
    MouseArea { anchors.fill: parent; enabled: root.settingsOpen; onClicked: root.settingsOpen = false }
  }
  Rectangle {
    id: sheet
    anchors.left: parent.left; anchors.right: parent.right
    height: sheetCol.implicitHeight + Style.space(34)
    y: root.settingsOpen ? parent.height - height : parent.height + Style.space(6)
    topLeftRadius: Style.space(20); topRightRadius: Style.space(20)
    color: Util.alpha(Color.popups.background, 0.97)
    border.width: 1; border.color: Util.alpha(Color.popups.text, 0.08)
    Behavior on y { NumberAnimation { duration: 220; easing.type: Easing.OutCubic } }
    Column {
      id: sheetCol
      anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
      anchors.leftMargin: Style.space(16); anchors.rightMargin: Style.space(16); anchors.topMargin: Style.space(10)
      spacing: Style.space(2)
      Rectangle { width: Style.space(36); height: Style.space(4); radius: 2; color: Util.alpha(Color.popups.text, 0.25); anchors.horizontalCenter: parent.horizontalCenter }
      Text { text: "Call settings"; color: Color.popups.text; font.family: Fonts.ui; font.pixelSize: Style.font.subtitle; font.bold: true; topPadding: Style.space(6); bottomPadding: Style.space(4) }
      Repeater {
        model: [ { title: "Microphone", kind: "mic" }, { title: "Speaker", kind: "speaker" } ]
        delegate: Column {
          id: devSection
          required property var modelData
          width: parent.width
          readonly property string kind: modelData.kind
          readonly property var devs: !root.svc ? [] : (kind === "mic" ? (root.svc.devices.mics || []) : (root.svc.devices.speakers || []))
          readonly property string sel: (root.svc && root.svc.devices.selected) ? (root.svc.devices.selected[kind] || "") : ""
          Text { text: devSection.modelData.title; color: Util.alpha(Color.popups.text, 0.55); font.family: Fonts.ui; font.pixelSize: Style.font.caption; font.bold: true; topPadding: Style.space(8); bottomPadding: Style.space(2) }
          Repeater {
            model: devSection.devs
            delegate: Rectangle {
              required property var modelData
              width: devSection.width; height: Style.space(32); radius: Style.space(8)
              readonly property bool selRow: devSection.sel === modelData.id || (devSection.sel === "" && modelData.default)
              color: dh.containsMouse ? Util.alpha(Color.popups.text, 0.08) : "transparent"
              Row { anchors.verticalCenter: parent.verticalCenter; anchors.left: parent.left; anchors.leftMargin: Style.space(6); spacing: Style.space(8)
                Text { text: parent.parent.selRow ? Icons.check : " "; color: Color.accent; font.family: Fonts.iconFilled; renderType: Text.NativeRendering; font.pixelSize: Style.font.body; width: Style.space(16) }
                Text { text: modelData.name; color: Color.popups.text; font.family: Fonts.ui; font.pixelSize: Style.font.body; elide: Text.ElideRight; width: sheet.width - Style.space(70) } }
              MouseArea { id: dh; anchors.fill: parent; hoverEnabled: true; onClicked: if (root.svc) { root.svc.callSelectDevice(devSection.kind, modelData.id); devRefresh.restart() } }
            }
          }
          Text { visible: devSection.devs.length === 0; text: "No devices found"; color: Util.alpha(Color.popups.text, 0.5); font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall }
        }
      }
    }
  }

  property var roomInfo: (svc && call && call.roomId) ? svc.room(call.roomId) : null
  readonly property string statusText: {
    var s = root.call.state
    if (s === "joining") return "Calling…"
    if (s === "reconnecting") return "Reconnecting…"
    if (s === "leaving") return "Ending…"
    if (s === "connected") {
      if (root.remotes.length === 0) return "Ringing…"
      return fmt(root.now - (root.call.since || root.now))
    }
    return s
  }
  // Test hook: throw the self-view PiP with a given velocity (px/s).
  function debugPipThrow(vx, vy) {
    pip.vx = vx; pip.vy = vy
    var c = pip.cornerFor(pip.x + vx * 0.14, pip.y + vy * 0.14)
    pip.targetX = c.x; pip.targetY = c.y
    pip.flying = true
    return Math.round(pip.x) + "," + Math.round(pip.y)
  }
  function debugPipPos() { return Math.round(pip.x) + "," + Math.round(pip.y) + (pip.flying ? " flying" : " settled") }

  function fmt(ms) { var s = Math.max(0, Math.floor(ms / 1000)), h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), sec = s % 60; var p = function(n) { return (n < 10 ? "0" : "") + n }; return h > 0 ? h + ":" + p(m) + ":" + p(sec) : m + ":" + p(sec) }
}
