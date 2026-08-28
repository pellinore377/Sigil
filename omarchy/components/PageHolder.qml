import QtQuick

// A slide-in page layer, factored out of the copies in Panel.qml: a frame-driven
// slide from the right, its own opaque ground, and an event sink.
//
// Frame-driven rather than a timed Behavior on purpose: opening a room stalls the
// UI thread while its timeline builds, and a time-based animation spends that
// stall and arrives already finished.
Item {
  id: root
  default property alias content: body.data
  property bool active: false
  /// Opaque backdrop. Must not be transparent, or the page below shows through.
  property color ground: "#000000"

  width: parent ? parent.width : 0
  height: parent ? parent.height : 0
  readonly property bool sliding: x > 0.5 && x < width - 0.5
  readonly property real targetX: active ? 0 : width
  visible: x < width - 0.5

  onTargetXChanged: slide.running = true
  Component.onCompleted: x = targetX
  onWidthChanged: if (!active && !slide.running) x = width

  FrameAnimation {
    id: slide
    running: false
    onTriggered: {
      var d = root.targetX - root.x
      if (Math.abs(d) < 0.5) { root.x = root.targetX; running = false; return }
      var step = d * 0.26
      var floor = Math.max(2, root.width * 0.03)
      if (Math.abs(step) < floor) step = d > 0 ? floor : -floor
      root.x += step
    }
  }

  Rectangle {
    anchors.fill: parent
    color: root.ground
    // The sink sits *behind* the page, so the page's own lists get the wheel first.
    MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; hoverEnabled: true }
    WheelHandler { onWheel: function(e) { e.accepted = true } }
  }

  Item { id: body; anchors.fill: parent }
}
