import QtQuick
import qs.Commons
import qs.Ui
import "../components"

// "Someone is typing": their avatar, then three dots bouncing in sequence.
Item {
  id: root
  property var typers: []            // [{userId, displayName, avatarPath}]
  property color fg: Color.menu.text
  property color surface: Color.popups.background
  readonly property bool active: typers.length > 0
  // Keep the last typer while the panel folds away: reading typers[0] once it
  // empties gives Avatar an empty id, which renders as a red placeholder.
  property var lastTyper: null
  onTypersChanged: if (typers.length > 0) root.lastTyper = typers[0]
  readonly property var shown: typers.length > 0 ? typers[0] : root.lastTyper

  implicitHeight: active ? Style.space(38) : 0
  height: implicitHeight
  visible: opacity > 0.01
  opacity: active ? 1 : 0
  Behavior on opacity { NumberAnimation { duration: 160 } }
  Behavior on implicitHeight { NumberAnimation { duration: 180; easing.type: Easing.OutCubic } }

  Row {
    anchors.left: parent.left
    anchors.leftMargin: Style.space(14)
    anchors.verticalCenter: parent.verticalCenter
    spacing: Style.space(7)

    Avatar {
      anchors.verticalCenter: parent.verticalCenter
      size: Style.space(22)
      source: root.shown ? (root.shown.avatarPath || "") : ""
      name: root.shown ? (root.shown.displayName || "") : ""
      userId: root.shown ? (root.shown.userId || "") : ""
    }

    Item {
      anchors.verticalCenter: parent.verticalCenter
      width: dots.width + Style.space(6)
      height: Style.space(30)

      Row {
        id: dots
        anchors.centerIn: parent
        spacing: Style.space(5)
        Repeater {
          model: 3
          delegate: Rectangle {
            required property int index
            width: Style.space(6); height: width; radius: width / 2
            antialiasing: true
            color: Util.alpha(root.fg, 0.55)
            // Each dot trails the one before it by a third of the cycle.
            y: 0
            SequentialAnimation on y {
              running: root.active
              loops: Animation.Infinite
              PauseAnimation { duration: index * 140 }
              NumberAnimation { to: -Style.space(4); duration: 220; easing.type: Easing.OutQuad }
              NumberAnimation { to: 0; duration: 260; easing.type: Easing.InQuad }
              PauseAnimation { duration: 420 - index * 140 }
            }
          }
        }
      }
    }
  }
}
