import QtQuick
import qs.Commons
import "."
Text {
  id: root
  property real size: Style.font.icon
  text: Icons.spinner
  color: Color.foreground
  font.family: Fonts.icon; renderType: Text.NativeRendering
  font.pixelSize: size
  RotationAnimation on rotation { from: 0; to: 360; duration: 900; loops: Animation.Infinite; running: root.visible }
}
