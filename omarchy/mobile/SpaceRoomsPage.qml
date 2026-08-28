import QtQuick
import QtQuick.Controls as QQC
import qs.Commons
import qs.Ui
import "../components"

// Add rooms to a space, or take them out again. One page in two modes: the
// same multi-select screen with the list inverted and the verb flipped.
Item {
  id: root
  property var svc: null
  property string spaceId: ""
  property color fg: Color.menu.text
  /// "manage" removes the space's current children; "add" puts joined rooms in.
  property string mode: "manage"

  signal closed()
  signal changed()

  readonly property bool adding: root.mode === "add"
  property var selected: ({})
  property int selectedCount: 0
  property var children_: []
  property bool loading: false
  property bool busy: false
  property string note: ""
  Timer { id: noteTimer; interval: 2600; onTriggered: root.note = "" }

  /// The space's current children, by id, so "add" can hide what is already in.
  readonly property var childIds: {
    var m = {}
    for (var i = 0; i < root.children_.length; i++) m[root.children_[i].id] = true
    return m
  }

  readonly property var rows: {
    if (!root.adding) return root.children_
    if (!root.svc) return []
    // Rooms only: adding a space from here would allow nesting one inside itself.
    return root.svc.rooms.filter(function (r) {
      return !r.isSpace && !root.childIds[r.id] && r.id !== root.spaceId
    })
  }

  function reset() {
    root.selected = ({})
    root.selectedCount = 0
    root.busy = false
    root.note = ""
    root.load()
  }

  function load() {
    if (!root.svc || !root.spaceId) return
    root.loading = true
    root.svc.spaceHierarchy(root.spaceId, function (r) {
      root.loading = false
      root.children_ = (r && r.rooms) ? r.rooms : []
    })
  }

  onSpaceIdChanged: root.reset()

  function toggle(id) {
    var s = Object.assign({}, root.selected)
    if (s[id]) delete s[id]; else s[id] = true
    root.selected = s
    var n = 0
    for (var k in s) n++
    root.selectedCount = n
  }

  function apply() {
    if (root.selectedCount === 0 || root.busy || !root.svc) return
    root.busy = true
    var ids = []
    for (var k in root.selected) ids.push(k)
    var left = ids.length
    var failed = 0
    var done = function (r, e) {
      if (e) failed++
      if (--left > 0) return
      root.busy = false
      root.selected = ({}); root.selectedCount = 0
      root.changed()
      if (failed > 0) { root.note = failed + " could not be changed"; noteTimer.restart() }
      root.load()
    }
    for (var i = 0; i < ids.length; i++) {
      if (root.adding) root.svc.addRoomToSpace(root.spaceId, ids[i], done)
      else root.svc.removeRoomFromSpace(root.spaceId, ids[i], done)
    }
  }

  Rectangle { anchors.fill: parent; color: Qt.lighter(Color.menu.background, 1.35) }

  Column {
    anchors.fill: parent
    spacing: 0

    Item {
      width: parent.width; height: Style.space(56)
      PanelActionButton {
        id: closeBtn
        anchors.left: parent.left; anchors.leftMargin: Style.space(6)
        anchors.verticalCenter: parent.verticalCenter
        fontFamily: Fonts.icon; iconText: Icons.close; foreground: root.fg
        onClicked: root.closed()
      }
      Text {
        anchors.left: closeBtn.right; anchors.leftMargin: Style.space(6)
        anchors.right: act.left; anchors.rightMargin: Style.space(10)
        anchors.verticalCenter: parent.verticalCenter
        text: root.selectedCount + " selected"
        color: root.fg; elide: Text.ElideRight
        font.family: Fonts.ui; font.pixelSize: Style.font.heading; font.bold: true
      }
      Text {
        id: act
        anchors.right: parent.right; anchors.rightMargin: Style.space(18)
        anchors.verticalCenter: parent.verticalCenter
        text: root.busy ? "Working…" : (root.adding ? "Add" : "Remove")
        color: root.selectedCount > 0 && !root.busy
               ? (root.adding ? Color.accent : Color.urgent)
               : Util.alpha(root.fg, 0.35)
        font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: true
        MouseArea {
          anchors.fill: parent; anchors.margins: -Style.space(8)
          enabled: root.selectedCount > 0 && !root.busy
          cursorShape: Qt.PointingHandCursor
          onClicked: root.apply()
        }
      }
    }

    Item {
      width: parent.width
      height: parent.height - y

      Column {
        anchors.centerIn: parent
        width: parent.width - Style.space(60)
        spacing: Style.space(10)
        visible: root.rows.length === 0
        Text {
          width: parent.width; horizontalAlignment: Text.AlignHCenter
          text: root.loading ? "Looking…"
                : (root.adding ? "Every room you are in is already here"
                               : "No rooms in this space")
          color: Util.alpha(root.fg, 0.6)
          font.family: Fonts.ui; font.pixelSize: Style.font.body
        }
      }

      ListView {
        anchors.fill: parent
        visible: root.rows.length > 0
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        QQC.ScrollBar.vertical: ScrollBarStyle {}
        model: root.rows

        delegate: Item {
          id: row
          required property var modelData
          readonly property bool picked: !!root.selected[row.modelData.id]
          width: ListView.view.width
          height: Style.space(64)

          Rectangle {
            anchors.fill: parent
            anchors.margins: Style.space(4)
            anchors.leftMargin: Style.space(8); anchors.rightMargin: Style.space(8)
            radius: Style.space(14)
            color: row.picked ? Util.alpha(Color.accent, 0.12)
                 : (rh.containsMouse ? Util.alpha(root.fg, 0.05) : "transparent")
          }

          Avatar {
            id: av
            anchors.left: parent.left; anchors.leftMargin: Style.space(16)
            anchors.verticalCenter: parent.verticalCenter
            size: Style.space(44)
            source: row.modelData.avatarPath || ""
            name: row.modelData.name || row.modelData.id
            userId: row.modelData.id
          }

          Column {
            anchors.left: av.right; anchors.leftMargin: Style.space(12)
            anchors.right: box.left; anchors.rightMargin: Style.space(12)
            anchors.verticalCenter: parent.verticalCenter
            spacing: Style.space(2)
            Text {
              width: parent.width; elide: Text.ElideRight
              text: row.modelData.name || row.modelData.id
              color: root.fg
              font.family: Fonts.ui; font.pixelSize: Style.font.subtitle; font.bold: true
            }
            Row {
              spacing: Style.space(5)
              IconLabel {
                anchors.verticalCenter: parent.verticalCenter
                icon: (row.modelData.worldReadable === undefined
                       ? !row.modelData.isEncrypted : row.modelData.worldReadable)
                      ? Icons.globe : Icons.lock
                color: Util.alpha(root.fg, 0.45); filled: true; size: Style.font.caption
              }
              Text {
                anchors.verticalCenter: parent.verticalCenter
                text: (row.modelData.worldReadable === undefined
                       ? !row.modelData.isEncrypted : row.modelData.worldReadable)
                      ? "Public" : "Private"
                color: Util.alpha(root.fg, 0.55)
                font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
              }
            }
            Text {
              visible: text !== ""
              text: {
                var n = row.modelData.memberCount !== undefined
                        ? row.modelData.memberCount : (row.modelData.joinedMembers || 0)
                return n === 0 ? "" : (n === 1 ? "1 Member" : n + " Members")
              }
              color: Util.alpha(root.fg, 0.55)
              font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
            }
          }

          Rectangle {
            id: box
            anchors.right: parent.right; anchors.rightMargin: Style.space(22)
            anchors.verticalCenter: parent.verticalCenter
            width: Style.space(22); height: Style.space(22)
            radius: Style.space(4)
            color: row.picked ? Color.accent : "transparent"
            border.width: Math.max(1, Style.space(2))
            border.color: row.picked ? Color.accent : Util.alpha(root.fg, 0.35)
            IconLabel {
              anchors.centerIn: parent
              visible: row.picked
              icon: Icons.check; color: Color.background
              filled: true; size: Style.font.bodySmall
            }
          }

          MouseArea {
            id: rh
            anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor
            onClicked: root.toggle(row.modelData.id)
          }
        }
      }
    }
  }

  Rectangle {
    anchors.horizontalCenter: parent.horizontalCenter
    anchors.bottom: parent.bottom; anchors.bottomMargin: Style.space(20)
    width: noteT.implicitWidth + Style.space(28); height: Style.space(34)
    radius: height / 2
    color: Color.popups.background
    opacity: root.note !== "" ? 1 : 0
    visible: opacity > 0.01
    Behavior on opacity { NumberAnimation { duration: 150 } }
    Text { id: noteT; anchors.centerIn: parent; text: root.note; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall }
  }
}
