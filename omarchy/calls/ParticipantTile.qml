import QtQuick
import QtQuick.Effects
import qs.Commons
import qs.Ui
import "../components"

// One call tile: video (via VideoTile) or avatar fallback, name strip, mute glyph, speaking ring.
Item {
  id: root
  property var participant: null      // engine participant object (or local pseudo-participant)
  property var track: null            // {key, kind, shmPath, width, height} or null
  property bool isLocal: false
  property bool fitVideo: false
  property color fg: Color.menu.text
  property color accent: Color.accent   // chat theme colour when the room has one
  // Style.cornerRadius mirrors Hyprland rounding and can be 0; call tiles always round.
  property real tileRadius: Style.space(12)
  // A camera turned off only *mutes* the track — LiveKit keeps it published — so the
  // surface would sit on its last frame forever. Treat a muted source as no video.
  readonly property bool trackLive: {
    if (!root.track || !root.track.shmPath) return false
    if (!root.participant) return true
    if (root.track.kind === "screen") return root.participant.screenSharing !== false
    return root.participant.cameraOn !== false
  }
  readonly property bool hasVideo: root.trackLive
  readonly property bool speaking: !!(participant && participant.speaking)
  readonly property bool micMuted: !!(participant && participant.micMuted)

  Rectangle {
    anchors.fill: parent
    radius: root.tileRadius
    color: Util.alpha(Color.background, 0.55)
    border.width: root.speaking ? 2 : 1
    border.color: root.speaking ? root.accent : Util.alpha(root.fg, 0.12)
    clip: true
    Loader {
      id: videoLoader
      anchors.fill: parent
      anchors.margins: root.speaking ? 2 : 1
      active: root.hasVideo
      source: "VideoTile.qml"
      onLoaded: { item.shmPath = Qt.binding(function() { return root.track ? root.track.shmPath : "" }); item.mirrored = Qt.binding(function() { return root.isLocal && root.track && root.track.kind === "camera" }); if (root.fitVideo || (root.track && root.track.kind === "screen")) item.fillMode = 1 }
      onStatusChanged: if (status === Loader.Error) console.warn("VideoTile unavailable:", sourceComponent ? sourceComponent.errorString() : "")
      // Rounded clipping (plain clip is rectangular). VideoSurface only pumps frames while
      // visible, so the loader stays visible and the layer effect masks its rendering.
      layer.enabled: true
      layer.effect: MultiEffect { maskEnabled: true; maskThresholdMin: 0.5; maskSpreadAtMin: 1.0; maskSource: tileMask }
    }
    Rectangle {
      id: tileMask
      anchors.fill: parent
      anchors.margins: root.speaking ? 2 : 1
      radius: Math.max(1, root.tileRadius - 1)
      color: "black"; visible: false; layer.enabled: true; layer.smooth: true
    }
    // Rings expanding out of the avatar, sized by the live audio level the engine reports.
    SpeakingRipple {
      anchors.centerIn: parent
      // Tied to the track's mute state only: keying on hasFrame flickered on every camera hiccup.
      visible: !root.hasVideo
      size: Math.min(parent.width, parent.height) * 0.32
      accent: root.accent
      speaking: root.speaking
      level: root.participant && root.participant.level !== undefined ? root.participant.level : 0
    }
    Avatar {
      anchors.centerIn: parent
      visible: !root.hasVideo
      size: Math.min(parent.width, parent.height) * 0.32
      source: root.participant ? (root.participant.avatarPath || "") : ""
      name: root.participant ? root.participant.displayName : ""
      userId: root.participant ? root.participant.userId : ""
    }
    Rectangle {
      anchors.left: parent.left; anchors.bottom: parent.bottom; anchors.margins: Style.space(8)
      // Bounded by the tile: the panel's 96-wide thumb strip hard-cut the name with no ellipsis.
      width: Math.min(nameRow.implicitWidth + Style.space(14), root.width - Style.space(16))
      height: Style.space(22); radius: height / 2
      color: Util.alpha(Color.background, 0.6)
      Row {
        id: nameRow; anchors.centerIn: parent; spacing: Style.space(5)
        IconLabel { visible: root.micMuted; icon: Icons.micOff; color: Color.urgent; anchors.verticalCenter: parent.verticalCenter; filled: true; size: Style.font.bodySmall }
        Text {
          text: (root.isLocal ? "You" : (root.participant ? root.participant.displayName : "")) + (root.track && root.track.kind === "screen" ? " (screen)" : "")
          color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
          anchors.verticalCenter: parent.verticalCenter
          // The pill caps at `root.width - 16`; subtract the mute glyph and spacing the Row adds.
          width: Math.min(implicitWidth, root.width - Style.space(30) - (root.micMuted ? Style.space(17) : 0))
          elide: Text.ElideRight
        }
      }
    }
    IconLabel { visible: root.participant && root.participant.quality === "poor" || (root.participant && root.participant.quality === "lost")
      anchors.right: parent.right; anchors.top: parent.top; anchors.margins: Style.space(8)
      icon: Icons.signalOff; color: Color.urgent; filled: true; size: Style.font.icon }
  }
}
