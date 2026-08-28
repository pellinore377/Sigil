import QtQuick
import qs.Commons
import qs.Ui

// A titled band of settings rows. The title sits at the rows' own content edge (22)
// so headings and labels share one left margin.
Column {
  id: root
  default property alias rows: holder.data
  property color fg: Color.menu.text
  property string title: ""
  property bool divided: true

  width: parent ? parent.width : 0

  Rectangle {
    width: parent.width; height: Math.max(1, Style.space(1))
    visible: root.divided
    color: Util.alpha(root.fg, 0.08)
  }
  Item { width: parent.width; height: root.title !== "" ? Style.space(14) : Style.space(6) }
  Text {
    visible: root.title !== ""
    x: Style.space(22)
    text: root.title; color: Util.alpha(root.fg, 0.55)
    font.family: Fonts.ui; font.pixelSize: Style.font.caption; font.bold: true
  }
  Item { width: parent.width; height: root.title !== "" ? Style.space(6) : 0 }
  Column { id: holder; width: parent.width }
  Item { width: parent.width; height: Style.space(6) }
}
