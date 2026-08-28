import QtQuick
import "."

// One icon, centred. Over a bare Text this adds a square box of a known size, the
// right family for filled vs outlined, and the native rasteriser — Qt's
// distance-field renderer drops the thin left edges of icon glyphs at these sizes.
// Qt already centres Material Symbols to within half a pixel: do not "correct" the ink.
Item {
  id: root

  /// The glyph, from the `Icons` singleton — never a literal.
  property string icon: ""
  property color color: "#ffffff"
  /// Pixel size of the glyph; the item defaults to a square of this size.
  property real size: 16
  /// Solid rather than outlined.
  property bool filled: false
  /// Override to `Text.QtRendering` for an icon that animates its scale or
  /// rotation, where the native rasteriser's fixed-size glyphs would smear.
  property int renderMode: Text.NativeRendering

  /// Hug the glyph horizontally instead of reserving a square: beside a label, a square
  /// box pads a narrow glyph. The box then becomes the glyph's **ink** width, not its advance.
  property bool fitWidth: false

  // Only needed for `fitWidth`: ink extent, narrower than the advance and not centred in it.
  TextMetrics {
    id: tm
    font.family: root.filled ? Fonts.iconFilled : Fonts.icon
    font.pixelSize: root.size
    text: root.icon
  }

  implicitWidth: root.fitWidth ? tm.tightBoundingRect.width : root.size
  implicitHeight: root.size

  Text {
    id: label
    text: root.icon
    color: root.color
    font: tm.font
    renderType: root.renderMode
    anchors.verticalCenter: parent.verticalCenter
    // With `fitWidth` the box IS the ink, so shift the text left by the ink's own offset.
    x: root.fitWidth ? -tm.tightBoundingRect.x : (parent.width - width) / 2
  }
}
