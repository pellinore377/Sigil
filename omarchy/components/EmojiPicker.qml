import QtQuick
import QtQuick.Controls as QQC
import Quickshell.Io
import qs.Commons
import qs.Ui
import ".."
import "."

// The one emoji picker. The reaction drawer and the attachment sheet both host
// this, so they cannot drift apart: same cell size, same search field, same
// category strip.
Item {
  id: root
  property color fg: Color.menu.text
  property color accent: Color.accent
  property color chip: Util.alpha(Color.background, 0.85)
  signal picked(string emoji)

  property var allEmojis: []
  property var cats: []

  FileView {
    id: emojiFile
    onLoaded: {
      try {
        root.allEmojis = JSON.parse(emojiFile.text())
        var find = function(e) { for (var i = 0; i < root.allEmojis.length; i++) if (root.allEmojis[i].e === e) return i; return -1 }
        root.cats = [
          { icon: "😀", idx: 0 }, { icon: "👋", idx: find("👋") }, { icon: "🐵", idx: find("🐵") },
          { icon: "🍇", idx: find("🍇") }, { icon: "🌍", idx: find("🌍") }, { icon: "🎃", idx: find("🎃") },
          { icon: "👓", idx: find("👓") }, { icon: "🏧", idx: find("🏧") }, { icon: "🏁", idx: find("🏁") }
        ].filter(function(c) { return c.idx >= 0 })
      } catch (e) { console.warn("emoji load failed:", e) }
    }
  }
  function load() { if (root.allEmojis.length === 0) emojiFile.path = "/usr/share/omarchy/shell/plugins/emojis/emojis.json" }
  function reset() { search.text = "" }

  readonly property var filtered: {
    var q = search.text.trim().toLowerCase()
    if (q === "") return root.allEmojis
    return root.allEmojis.filter(function(x) { return x.k.indexOf(q) >= 0 })
  }

  Column {
    anchors.fill: parent
    spacing: Style.space(8)

    Rectangle {
      width: parent.width; height: Style.space(34); radius: height / 2
      color: root.chip
      IconLabel { anchors.left: parent.left; anchors.leftMargin: Style.space(12); anchors.verticalCenter: parent.verticalCenter
        icon: Icons.search; color: Util.alpha(root.fg, 0.5); size: Style.font.bodySmall }
      QQC.TextField {
        id: search
        anchors.fill: parent; anchors.leftMargin: Style.space(32); anchors.rightMargin: Style.space(10)
        verticalAlignment: TextInput.AlignVCenter
        color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
        placeholderText: "Search emoji"
        placeholderTextColor: Util.alpha(root.fg, 0.45)
        background: Item {}
        QQC.ContextMenu.menu: null
        TextContextMenu { editor: parent }
      }
    }

    GridView {
      id: grid
      width: parent.width
      height: parent.height - Style.space(86)
      clip: true
      cellWidth: width / Math.max(1, Math.floor(width / Style.space(44)))
      cellHeight: Style.space(44)
      model: root.filtered
      boundsBehavior: Flickable.StopAtBounds
      QQC.ScrollBar.vertical: ScrollBarStyle {}
      delegate: Item {
        required property var modelData
        width: grid.cellWidth; height: grid.cellHeight
        Rectangle { anchors.fill: parent; anchors.margins: 2; radius: Style.space(8); color: gh.containsMouse ? Util.alpha(root.accent, 0.3) : "transparent" }
        Text { anchors.centerIn: parent; text: modelData.e; font.pixelSize: Style.space(22) }
        MouseArea { id: gh; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: root.picked(modelData.e) }
      }
    }

    Row {
      anchors.horizontalCenter: parent.horizontalCenter
      spacing: Style.space(4)
      // Which category the grid is currently showing.
      readonly property int curCat: {
        if (search.text.trim() !== "") return -1
        var i = grid.indexAt(Style.space(10), grid.contentY + Style.space(10))
        if (i < 0) i = 0
        var cols = Math.max(1, Math.floor(grid.width / grid.cellWidth))
        i += cols - 1
        var c = 0
        for (var k = 0; k < root.cats.length; k++) if (root.cats[k].idx <= i) c = k
        return c
      }
      Repeater {
        model: root.cats
        delegate: Rectangle {
          required property var modelData
          required property int index
          width: Style.space(32); height: Style.space(32); radius: Style.space(10)
          // Selected sits on the accent, with a softer tint on hover.
          color: parent.curCat === index ? root.accent
               : (th.containsMouse ? Util.alpha(root.accent, 0.35) : "transparent")
          Text { anchors.centerIn: parent; text: modelData.icon; font.pixelSize: Style.space(15); opacity: 0.9 }
          MouseArea { id: th; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: { search.text = ""; grid.positionViewAtIndex(modelData.idx, GridView.Beginning) } }
        }
      }
    }
  }
}
