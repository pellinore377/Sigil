import QtQuick
import QtQuick.Controls as QQC
import qs.Commons
QQC.ScrollBar {
  id: bar
  // Set while something scrolls the list without the Flickable itself
  // moving (the chat's fixed-step wheel), which never sets `active`.
  property bool forceActive: false
  implicitWidth: Style.space(7)
  // Keep the handle clear of the card's rounded corners and its right edge.
  topPadding: Style.space(14)
  bottomPadding: Style.space(14)
  rightPadding: Style.space(3)
  minimumSize: 0.14
  // AsNeeded still shows the bar when content overhangs by a pixel, so a list that
  // fits looked permanently scrollable. Show it only while it is being used.
  policy: QQC.ScrollBar.AsNeeded
  visible: bar.size > 0 && bar.size < 0.995
  contentItem: Rectangle {
    implicitWidth: Style.space(4)
    radius: width / 2
    color: Util.alpha(Color.menu.text, bar.pressed ? 0.45 : (bar.hovered ? 0.35 : 0.2))
    opacity: bar.active || bar.hovered || bar.pressed || bar.forceActive ? 1 : 0
    Behavior on opacity { NumberAnimation { duration: 220 } }
    // Variable-height delegates re-estimate contentHeight as they realise, so the
    // handle size is recomputed constantly; ease it rather than letting it snap.
    Behavior on height { NumberAnimation { duration: 240; easing.type: Easing.OutCubic } }
  }
  background: Item {}
}
