import QtQuick
import qs.Commons
import qs.Ui
import "../calls"
import "../components"

// The call shrunk into a corner. Tapping reveals expand and end-call.
Item {
  id: root
  property var svc: null
  property color fg: Color.menu.text
  property color accent: Color.accent
  signal expandRequested()
  signal hangupRequested()

  // Overridable so a synthetic call (the fakeCall test hook) can drive it too.
  property var call: svc ? svc.call : ({ state: "idle" })
  readonly property var remotes: call && call.participants ? call.participants : []
  readonly property var firstRemote: remotes.length > 0 ? remotes[0] : null
  // Same preference as the call page: a screen share, then a camera.
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

  property bool controlsOn: false
  Timer { id: hideControls; interval: 2600; onTriggered: root.controlsOn = false }

  width: Style.space(132)
  height: Style.space(172)

  // Draggable corner to corner: release velocity carries into a corner spring.
  property real vx: 0
  property real vy: 0
  property real targetX: x
  property real targetY: y
  property bool flying: false
  readonly property real edge: Style.space(14)
  readonly property real maxX: Math.max(edge, (parent ? parent.width : width) - width - edge)
  readonly property real maxY: Math.max(edge, (parent ? parent.height : height) - height - edge)

  function cornerFor(px, py) {
    var pw = parent ? parent.width : width
    var ph = parent ? parent.height : height
    return {
      x: (px + width / 2 < pw / 2) ? edge : maxX,
      y: (py + height / 2 < ph / 2) ? edge : maxY
    }
  }
  /// Settle into the nearest corner without animating.
  function park() {
    var c = everThrown ? cornerFor(x, y) : { x: maxX, y: maxY }
    flying = false; vx = 0; vy = 0
    x = c.x; y = c.y; targetX = c.x; targetY = c.y
  }
  // Until thrown once the tile belongs bottom right: the card has no height at
  // completion, so "nearest corner" would read as top.
  property bool everThrown: false
  Component.onCompleted: { x = maxX; y = maxY; targetX = x; targetY = y }
  /// Test hook: throw the tile as a release would, without synthetic input.
  function debugFling(ivx, ivy) {
    vx = ivx; vy = ivy
    var c = cornerFor(x + vx * 0.14, y + vy * 0.14)
    targetX = c.x; targetY = c.y; everThrown = true; flying = true
    return Math.round(x) + "," + Math.round(y) + " -> " + Math.round(targetX) + "," + Math.round(targetY)
  }
  function debugPos() { return Math.round(x) + "," + Math.round(y) + " flying=" + flying }
  onMaxXChanged: if (!dragArea.drag.active && !flying) park()
  onMaxYChanged: if (!dragArea.drag.active && !flying) park()

  FrameAnimation {
    id: pipTrack
    running: dragArea.drag.active
    property real lastX: 0
    property real lastY: 0
    onRunningChanged: { lastX = root.x; lastY = root.y; root.vx = 0; root.vy = 0 }
    onTriggered: {
      var dt = Math.max(0.001, frameTime)
      // Low-passed, so one jittery frame cannot define the throw.
      root.vx = root.vx * 0.55 + ((root.x - lastX) / dt) * 0.45
      root.vy = root.vy * 0.55 + ((root.y - lastY) / dt) * 0.45
      lastX = root.x; lastY = root.y
    }
  }

  FrameAnimation {
    id: pipSpring
    running: root.flying
    onTriggered: {
      var dt = Math.min(0.033, Math.max(0.001, frameTime))
      var k = 120       // spring stiffness
      var c = 15        // damping
      root.vx += ((root.targetX - root.x) * k - root.vx * c) * dt
      root.vy += ((root.targetY - root.y) * k - root.vy * c) * dt
      root.x += root.vx * dt
      root.y += root.vy * dt
      if (root.x < root.edge) { root.x = root.edge; root.vx = Math.abs(root.vx) * 0.3 }
      else if (root.x > root.maxX) { root.x = root.maxX; root.vx = -Math.abs(root.vx) * 0.3 }
      if (root.y < root.edge) { root.y = root.edge; root.vy = Math.abs(root.vy) * 0.3 }
      else if (root.y > root.maxY) { root.y = root.maxY; root.vy = -Math.abs(root.vy) * 0.3 }
      if (Math.abs(root.vx) < 8 && Math.abs(root.vy) < 8
          && Math.abs(root.targetX - root.x) < 0.6 && Math.abs(root.targetY - root.y) < 0.6) {
        root.x = root.targetX; root.y = root.targetY
        root.vx = 0; root.vy = 0; root.flying = false
      }
    }
  }

  scale: visible ? 1 : 0.8
  opacity: visible ? 1 : 0
  Behavior on scale { NumberAnimation { duration: 180; easing.type: Easing.OutBack; easing.overshoot: 1.6 } }
  Behavior on opacity { NumberAnimation { duration: 140 } }

  Rectangle {
    anchors.fill: parent
    radius: Style.space(16)
    antialiasing: true
    color: Qt.rgba(0.08, 0.08, 0.09, 1)
  }

  ParticipantTile {
    id: tile
    anchors.fill: parent
    tileRadius: Style.space(16)
    participant: root.featured ? root.featured.p : root.firstRemote
    track: root.featured ? root.featured.t : null
    fg: root.fg
    accent: root.accent
  }

  Rectangle {
    anchors.top: parent.top; anchors.left: parent.left; anchors.margins: Style.space(6)
    width: dur.implicitWidth + Style.space(12); height: Style.space(18); radius: height / 2
    color: Util.alpha(Color.background, 0.6)
    visible: !root.controlsOn
    property real now: Date.now()
    Timer { interval: 1000; running: root.visible; repeat: true; onTriggered: parent.now = Date.now() }
    Text {
      id: dur
      anchors.centerIn: parent
      text: {
        var since = root.call && root.call.since ? root.call.since : 0
        if (!since) return "•"
        var s = Math.max(0, Math.floor((parent.now - since) / 1000))
        var m = Math.floor(s / 60), sec = s % 60
        return m + ":" + (sec < 10 ? "0" : "") + sec
      }
      color: "#ececec"; font.family: Fonts.ui; font.pixelSize: Style.space(9)
    }
  }

  MouseArea {
    id: dragArea
    anchors.fill: parent
    cursorShape: drag.active ? Qt.ClosedHandCursor : Qt.PointingHandCursor
    drag.target: root
    drag.minimumX: root.edge; drag.maximumX: root.maxX
    drag.minimumY: root.edge; drag.maximumY: root.maxY
    // A press that never became a drag is a tap.
    onClicked: { root.controlsOn = !root.controlsOn; if (root.controlsOn) hideControls.restart() }
    onPressed: { root.flying = false; root.vx = 0; root.vy = 0 }
    onReleased: {
      if (!pipTrack.running && root.vx === 0 && root.vy === 0) return
      // Aim at where the throw is heading, not where the finger let go.
      var c = root.cornerFor(root.x + root.vx * 0.14, root.y + root.vy * 0.14)
      root.targetX = c.x; root.targetY = c.y
      root.everThrown = true
      root.flying = true
    }
  }

  Rectangle {
    anchors.fill: parent
    radius: Style.space(16)
    color: Util.alpha(Color.background, 0.55)
    visible: opacity > 0.01
    opacity: root.controlsOn ? 1 : 0
    Behavior on opacity { NumberAnimation { duration: 140 } }

    Rectangle {
      anchors.centerIn: parent
      width: Style.space(42); height: width; radius: width / 2
      color: Util.alpha(Color.background, 0.85)
      IconLabel { anchors.centerIn: parent; icon: Icons.fullscreen; color: "#ececec"; filled: true; size: Style.font.icon }
      MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.expandRequested() }
    }

    Rectangle {
      anchors.top: parent.top; anchors.right: parent.right; anchors.margins: Style.space(6)
      width: Style.space(26); height: width; radius: width / 2
      color: Util.alpha(Color.urgent, 0.95)
      IconLabel { anchors.centerIn: parent; icon: Icons.close; color: "#ffffff"; filled: true; size: Style.font.bodySmall }
      MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.hangupRequested() }
    }
  }
}
