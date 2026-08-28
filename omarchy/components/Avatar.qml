import QtQuick
import Quickshell.Widgets
import qs.Commons
import "."

// Round avatar: image file when available, otherwise initials on a hashed hue.
Item {
  id: root
  /// Squares the avatar off for spaces. Negative means a full circle.
  property real cornerRadius: -1
  readonly property real shapeRadius: root.cornerRadius >= 0 ? root.cornerRadius : width / 2
  property string source: ""
  property string name: ""
  property string userId: ""
  property real size: Style.space(32)
  property real fontScale: 0.42
  // "" (unknown, no dot) | "online" | "away" | "busy" | "offline". Offline draws a hollow ring.
  property string status: ""
  /// Colour the ring punches through — whatever this avatar sits on.
  property color statusBackdrop: Color.menu.background
  width: size; height: size

  readonly property string initials: {
    var n = (root.name || root.userId || "?").replace(/^[@#!]/, "").trim()
    if (n === "") return "?"
    var parts = n.split(/[\s_\-.]+/).filter(function(p) { return p.length > 0 })
    if (parts.length >= 2) return (parts[0][0] + parts[1][0]).toUpperCase()
    return n.substring(0, 1).toUpperCase()
  }
  readonly property color hue: {
    var s = root.userId || root.name || ""
    var h = 0
    for (var i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) >>> 0
    var hues = [0.00, 0.08, 0.16, 0.33, 0.50, 0.58, 0.66, 0.75, 0.83, 0.92]
    return Qt.hsla(hues[h % hues.length], 0.35, 0.55, 1.0)
  }

  Rectangle {
    anchors.fill: parent
    radius: root.shapeRadius
    color: Util.alpha(root.hue, 0.55)
    visible: img.status !== Image.Ready
    Text {
      anchors.centerIn: parent
      text: root.initials
      color: Color.foreground
      font.family: Fonts.ui
      font.pixelSize: Math.max(8, root.size * root.fontScale)
      font.bold: true
    }
  }
  ClippingRectangle {
    anchors.fill: parent
    radius: root.shapeRadius
    color: "transparent"
    visible: img.status === Image.Ready
    Image {
      id: img
      anchors.fill: parent
      source: root.source ? "file://" + root.source : ""
      sourceSize.width: Math.round(root.size * 2)
      sourceSize.height: Math.round(root.size * 2)
      fillMode: Image.PreserveAspectCrop
      asynchronous: true
      cache: true
    }
  }

  // Presence dot, with a hole punched in the avatar behind it so it reads at any size.
  Rectangle {
    id: statusDot
    visible: root.status !== ""
    z: 2
    anchors.right: parent.right
    anchors.bottom: parent.bottom
    // Sunk slightly into the circle: the bounding-box corner is outside the avatar.
    anchors.rightMargin: -root.size * 0.02
    anchors.bottomMargin: -root.size * 0.02
    width: Math.max(Style.space(8), Math.round(root.size * 0.323))
    height: width
    radius: width / 2
    antialiasing: true
    color: "transparent"
    // Glyphs rather than nested Rectangles: they scale without recomputing border widths.
    IconLabel {
      anchors.centerIn: parent
      size: parent.width
      filled: true
      // At ~13px the native rasteriser snaps the circle to the grid and it reads oval.
      renderMode: Text.QtRendering
      icon: root.status === "offline" ? Icons.statusRing : Icons.statusDot
      color: root.status === "online" ? "#3fbf5f"
           : root.status === "busy" ? "#e2564a"
           : root.status === "away" ? "#eaa030"
           : Color.menu.text
    }
  }
}
