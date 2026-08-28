import QtQuick
import qs.Commons
import qs.Ui
import "../components"

// Replaces the timeline column while a call is active in the selected room.
Item {
  id: root
  property var svc: null
  property color fg: Color.menu.text
  signal beforeScreenshare()
  readonly property var call: svc ? svc.call : ({ state: "idle" })
  property int now: Date.now()
  Timer { interval: 1000; running: root.visible; repeat: true; onTriggered: root.now = Date.now() }

  readonly property var tiles: {
    var out = []
    var c = root.call
    if (!c) return out
    var parts = c.participants || []
    for (var i = 0; i < parts.length; i++) {
      var p = parts[i]
      var cam = null, screen = null
      for (var t = 0; t < (p.tracks || []).length; t++) { if (p.tracks[t].kind === "screen") screen = p.tracks[t]; else cam = p.tracks[t] }
      out.push({ participant: p, track: cam, isLocal: false })
      if (screen) out.push({ participant: p, track: screen, isLocal: false })
    }
    var lp = { displayName: "You", userId: root.svc ? root.svc.userId : "", avatarPath: root.svc ? root.svc.avatarPath : "", micMuted: c.local ? c.local.micMuted : false, speaking: c.local ? c.local.speaking : false, quality: "good" }
    var lcam = null, lscreen = null
    for (var k = 0; k < ((c.local && c.local.tracks) || []).length; k++) { if (c.local.tracks[k].kind === "screen") lscreen = c.local.tracks[k]; else lcam = c.local.tracks[k] }
    out.push({ participant: lp, track: lcam, isLocal: true })
    if (lscreen) out.push({ participant: lp, track: lscreen, isLocal: true })
    return out
  }

  Column {
    anchors.fill: parent
    Item {
      width: parent.width; height: Style.space(36)
      Row {
        anchors.centerIn: parent; spacing: Style.space(10)
        Text { text: root.call.state === "joining" ? ("Connecting… " + (root.call.step || "")) : root.call.state === "reconnecting" ? "Reconnecting…" : root.call.state === "leaving" ? "Leaving…" : (root.call.since ? fmt(root.now - root.call.since) : ""); color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body; anchors.verticalCenter: parent.verticalCenter }
        IconLabel { visible: !!root.call.encrypted; icon: Icons.lock; color: Util.alpha(root.fg, 0.6); anchors.verticalCenter: parent.verticalCenter; filled: true; size: Style.font.bodySmall }
        Text { visible: !!root.call.error; text: root.call.error || ""; color: Color.urgent; font.family: Fonts.ui; font.pixelSize: Style.font.caption; anchors.verticalCenter: parent.verticalCenter }
      }
    }
    ParticipantGrid { width: parent.width; height: parent.height - Style.space(36) - controls.height; tiles: root.tiles; fg: root.fg }
    CallControls { id: controls; width: parent.width; svc: root.svc; fg: root.fg; onBeforeScreenshare: root.beforeScreenshare() }
  }

  function fmt(ms) {
    var s = Math.max(0, Math.floor(ms / 1000)), h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), sec = s % 60
    var pad = function(n) { return (n < 10 ? "0" : "") + n }
    return h > 0 ? h + ":" + pad(m) + ":" + pad(sec) : m + ":" + pad(sec)
  }
}
