import QtQuick
import qs.Commons
import qs.Ui

// A bottom sheet that asks one question with a short list of answers, used where
// a settings row's value is one of a handful of choices. A dropdown would need a
// popup positioned against a row inside a scrolling list; a sheet cannot land offscreen.
Item {
  id: root
  /// [{ t, v, sub }] — label, value, optional second line.
  property var model: []
  property var value: null
  property string title: ""
  property color fg: Color.menu.text
  property bool open: false

  signal chose(var value)

  anchors.fill: parent
  visible: open || sheet.y < root.height - 0.5

  Rectangle {
    anchors.fill: parent
    color: Qt.rgba(0, 0, 0, root.open ? 0.5 : 0)
    Behavior on color { ColorAnimation { duration: 140 } }
    MouseArea { anchors.fill: parent; enabled: root.open; onClicked: root.open = false }
  }

  Rectangle {
    id: sheet
    width: parent.width
    height: col.implicitHeight + Style.space(28)
    y: root.open ? parent.height - height : parent.height
    Behavior on y { NumberAnimation { duration: 180; easing.type: Easing.OutCubic } }
    topLeftRadius: Style.space(22); topRightRadius: Style.space(22)
    color: Color.popups.background

    // Grab handle, so the sheet reads as a sheet rather than a docked panel.
    Rectangle {
      anchors.horizontalCenter: parent.horizontalCenter
      anchors.top: parent.top; anchors.topMargin: Style.space(8)
      width: Style.space(34); height: Style.space(4); radius: height / 2
      color: Util.alpha(root.fg, 0.25)
    }

    Column {
      id: col
      anchors.left: parent.left; anchors.right: parent.right
      anchors.top: parent.top; anchors.topMargin: Style.space(22)
      Text {
        visible: root.title !== ""
        x: Style.space(22)
        text: root.title; color: Util.alpha(root.fg, 0.55)
        font.family: Fonts.ui; font.pixelSize: Style.font.caption; font.bold: true
        bottomPadding: Style.space(6)
      }
      Repeater {
        model: root.model
        delegate: SettingsRow {
          required property var modelData
          fg: root.fg
          label: modelData.t
          sublabel: modelData.sub || ""
          trailing: "radio"
          on: root.value === modelData.v
          onClicked: { root.open = false; root.chose(modelData.v) }
        }
      }
    }
  }
}
