import QtQuick
import qs.Commons
import qs.Ui

// The header every settings page wears: back, title, and at most one trailing text
// action. Kept in one file so the back inset and title baseline cannot drift.
Item {
  id: root
  property color fg: Color.menu.text
  property string title: ""
  property string action: ""
  /// A greyed action still shows what the page can do; hiding it made the header jump.
  property bool actionEnabled: true

  signal back()
  signal actioned()

  width: parent ? parent.width : 0
  height: Style.space(56)

  PanelActionButton {
    id: backBtn
    anchors.left: parent.left; anchors.leftMargin: Style.space(6)
    anchors.verticalCenter: parent.verticalCenter
    fontFamily: Fonts.icon
    iconText: Icons.back
    foreground: root.fg
    onClicked: root.back()
  }

  Text {
    anchors.left: backBtn.right; anchors.leftMargin: Style.space(6)
    anchors.right: act.left; anchors.rightMargin: Style.space(10)
    anchors.verticalCenter: parent.verticalCenter
    text: root.title; color: root.fg
    font.family: Fonts.ui; font.pixelSize: Style.font.heading; font.bold: true
    elide: Text.ElideRight
  }

  Text {
    id: act
    anchors.right: parent.right; anchors.rightMargin: Style.space(18)
    anchors.verticalCenter: parent.verticalCenter
    visible: root.action !== ""
    text: root.action
    color: root.actionEnabled ? Color.accent : Util.alpha(root.fg, 0.35)
    font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: true
    MouseArea {
      anchors.fill: parent; anchors.margins: -Style.space(8)
      enabled: root.actionEnabled
      cursorShape: Qt.PointingHandCursor
      onClicked: root.actioned()
    }
  }
}
