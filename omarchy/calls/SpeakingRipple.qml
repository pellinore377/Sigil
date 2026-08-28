import QtQuick
import qs.Commons
import qs.Ui

// Rings expanding out of a call avatar while that participant talks. The
// engine reports LiveKit's audio level per participant, so the rings breathe
// with the actual voice instead of pulsing at a fixed rate.
Item {
  id: root
  property color accent: Color.accent
  property bool speaking: false
  property real level: 0            // 0..1 from the engine
  property real size: Style.space(110)

  // Levels arrive in bursts; ease towards them so the rings never jump.
  property real smooth: 0
  FrameAnimation {
    running: root.visible && (root.speaking || root.smooth > 0.001)
    onTriggered: {
      var target = root.speaking ? Math.max(0.18, Math.min(1, root.level * 3.2)) : 0
      root.smooth += (target - root.smooth) * Math.min(1, frameTime * 9)
    }
  }

  width: root.size * 2.1
  height: width

  Repeater {
    model: 2
    delegate: Rectangle {
      required property int index
      anchors.centerIn: parent
      readonly property real phase: (pulse.value + index * 0.5) % 1.0
      width: root.size * (1 + phase * 1.05 * (0.35 + root.smooth))
      height: width
      radius: width / 2
      color: "transparent"
      border.width: Math.max(1, Style.space(2) * (0.4 + root.smooth))
      border.color: Util.alpha(root.accent, (1 - phase) * 0.55 * Math.min(1, root.smooth * 1.6))
      antialiasing: true
      visible: root.smooth > 0.01
    }
  }

  // Free-running phase for the expansion; amplitude comes from `smooth`.
  QtObject {
    id: pulse
    property real value: 0
  }
  NumberAnimation {
    target: pulse; property: "value"
    from: 0; to: 1; duration: 1600
    loops: Animation.Infinite
    running: root.visible
  }
}
