import QtQuick
import qs.Commons
import qs.Ui

// One line of a settings list: leading icon, label, optional sub-label, and one
// trailing control. The metrics live here once — icon column at 22, text at 56,
// trailing furniture inset 20 — so pages cannot disagree by a few pixels.
Item {
  id: root
  property color fg: Color.menu.text
  property string icon: ""
  property string label: ""
  property string sublabel: ""
  /// none | chevron | value | toggle | radio | count | check
  property string trailing: "none"
  property string value: ""
  property bool on: false
  property bool danger: false
  /// A disabled row still reads: it says the thing exists and you may not change it.
  property bool enabled: true
  property bool filled: false

  signal clicked()

  readonly property color ink: root.danger ? Color.urgent
                             : (root.enabled ? root.fg : Util.alpha(root.fg, 0.38))

  width: parent ? parent.width : 0
  height: root.sublabel !== "" ? Style.space(60) : Style.space(48)

  Rectangle {
    anchors.fill: parent
    anchors.margins: Style.space(4)
    anchors.leftMargin: Style.space(10); anchors.rightMargin: Style.space(10)
    radius: Style.space(12)
    color: (hover.containsMouse && root.enabled) ? Util.alpha(root.fg, 0.05) : "transparent"
  }

  IconLabel {
    visible: root.icon !== ""
    anchors.left: parent.left; anchors.leftMargin: Style.space(22)
    anchors.verticalCenter: parent.verticalCenter
    icon: root.icon
    color: root.danger ? Color.urgent : Util.alpha(root.ink, 0.75)
    filled: root.filled
    size: Style.font.icon
  }

  Column {
    anchors.left: parent.left; anchors.leftMargin: root.icon !== "" ? Style.space(56) : Style.space(22)
    anchors.right: tail.left; anchors.rightMargin: Style.space(10)
    anchors.verticalCenter: parent.verticalCenter
    spacing: Style.space(2)
    Text {
      width: parent.width; elide: Text.ElideRight
      text: root.label; color: root.ink
      font.family: Fonts.ui; font.pixelSize: Style.font.body
    }
    Text {
      width: parent.width; visible: root.sublabel !== ""
      text: root.sublabel; color: Util.alpha(root.ink, 0.55)
      font.family: Fonts.ui; font.pixelSize: Style.font.caption
      wrapMode: Text.Wrap; maximumLineCount: 2; elide: Text.ElideRight
    }
  }

  Item {
    id: tail
    anchors.right: parent.right; anchors.rightMargin: Style.space(20)
    anchors.verticalCenter: parent.verticalCenter
    width: tailLoader.item ? tailLoader.item.implicitWidth : 0
    height: parent.height
    Loader {
      id: tailLoader
      anchors.centerIn: parent
      sourceComponent: root.trailing === "toggle" ? toggleC
                     : root.trailing === "radio"  ? radioC
                     : root.trailing === "check"  ? checkC
                     : root.trailing === "chevron" ? chevC
                     : (root.trailing === "value" || root.trailing === "count") ? valueC
                     : null
    }
  }

  Component {
    id: toggleC
    Rectangle {
      implicitWidth: Style.space(38); implicitHeight: Style.space(22)
      radius: height / 2
      color: root.on ? Util.alpha(Color.accent, 0.8) : Util.alpha(root.fg, 0.15)
      opacity: root.enabled ? 1 : 0.45
      Rectangle {
        width: Style.space(16); height: Style.space(16); radius: width / 2
        anchors.verticalCenter: parent.verticalCenter
        x: root.on ? parent.width - width - Style.space(3) : Style.space(3)
        color: root.fg
        Behavior on x { NumberAnimation { duration: 120 } }
      }
    }
  }

  Component {
    id: radioC
    Rectangle {
      implicitWidth: Style.space(20); implicitHeight: Style.space(20)
      radius: width / 2
      color: "transparent"
      border.width: Math.max(1, Style.space(2))
      border.color: root.on ? Color.accent : Util.alpha(root.fg, 0.35)
      opacity: root.enabled ? 1 : 0.45
      Rectangle {
        anchors.centerIn: parent
        width: parent.width * 0.5; height: width; radius: width / 2
        visible: root.on
        color: Color.accent
      }
    }
  }

  Component {
    id: checkC
    Rectangle {
      implicitWidth: Style.space(20); implicitHeight: Style.space(20)
      radius: Style.space(4)
      color: root.on ? Color.accent : "transparent"
      border.width: Math.max(1, Style.space(2))
      border.color: root.on ? Color.accent : Util.alpha(root.fg, 0.35)
      IconLabel {
        anchors.centerIn: parent
        visible: root.on
        icon: Icons.check; color: Color.background
        filled: true; size: Style.font.caption
      }
    }
  }

  Component {
    id: chevC
    IconLabel {
      implicitWidth: Style.font.icon
      icon: Icons.chevronRight
      color: Util.alpha(root.ink, 0.4)
      filled: true; size: Style.font.icon
    }
  }

  Component {
    id: valueC
    Text {
      text: root.value
      color: Util.alpha(root.ink, 0.5)
      font.family: Fonts.ui; font.pixelSize: Style.font.body
    }
  }

  MouseArea {
    id: hover
    anchors.fill: parent
    hoverEnabled: true
    enabled: root.enabled
    cursorShape: Qt.PointingHandCursor
    onClicked: root.clicked()
  }
}
