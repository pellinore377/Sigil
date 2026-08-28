import QtQuick
import QtQuick.Controls as QQC
import qs.Commons
import qs.Ui
import "../components"
import ".."

// Conversation search: text results while typing; media and links when idle.
Item {
  id: root
  property var svc: null
  property color fg: Color.menu.text
  property string roomId: ""
  signal closed()
  signal jumpTo(string eventId)
  signal openImage(var item)

  function reset() { search.text = "" }
  function focusSearch() { search.forceActiveFocus() }

  readonly property var tl: (svc && roomId) ? svc.timelineFor(roomId) : null
  property int rev: 0
  Connections { target: root.svc; function onTimelineChanged(rid, ops) { if (rid === root.roomId) root.rev++ } }

  function collect(kind, q, limit) {
    var out = []
    if (!root.tl) return out
    var m = root.tl.model
    for (var i = 0; i < m.count && out.length < limit; i++) {
      var it = m.get(i)
      if (kind === "text") {
        if (!it.body || it.kind === "image") continue
        if (it.body.toLowerCase().indexOf(q) >= 0) out.push({ eventId: it.eventId, senderName: it.senderName, isOwn: !!it.isOwn, ts: it.ts, body: it.body })
      } else if (kind === "image") {
        if (it.kind === "image" && it.media) out.push({ eventId: it.eventId, senderName: it.senderName, isOwn: !!it.isOwn, ts: it.ts, body: it.body, kind: "image", media: it.media, can: it.can })
      } else if (kind === "link") {
        if (it.body && /https?:\/\/\S+/.test(it.body)) {
          var mm = it.body.match(/https?:\/\/\S+/)
          out.push({ eventId: it.eventId, senderName: it.senderName, isOwn: !!it.isOwn, ts: it.ts, url: mm[0] })
        }
      }
    }
    return out
  }
  readonly property string query: search.text.trim().toLowerCase()
  readonly property var results: { var d = root.rev; return root.query.length >= 2 ? collect("text", root.query, 40) : [] }
  readonly property var images: { var d = root.rev; return collect("image", "", 12) }
  readonly property var links: { var d = root.rev; return collect("link", "", 10) }

  Column {
    anchors.fill: parent
    spacing: 0
    Item {
      width: parent.width; height: Style.space(54)
      PanelActionButton { id: backBtn; anchors.left: parent.left; anchors.leftMargin: Style.space(6); anchors.verticalCenter: parent.verticalCenter; fontFamily: Fonts.icon; iconText: Icons.back; foreground: root.fg; onClicked: root.closed() }
      Rectangle {
        anchors.left: backBtn.right; anchors.leftMargin: Style.space(4); anchors.right: parent.right; anchors.rightMargin: Style.space(14); anchors.verticalCenter: parent.verticalCenter
        height: Style.space(36); radius: height / 2
        color: Util.alpha(root.fg, 0.07)
        border.width: 1; border.color: search.activeFocus ? Util.alpha(Color.accent, 0.4) : "transparent"
        QQC.TextField {
          id: search
          anchors.fill: parent; anchors.leftMargin: Style.space(14); anchors.rightMargin: Style.space(12)
          color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body
          placeholderText: "Search conversation"
          placeholderTextColor: Util.alpha(root.fg, 0.45)
          background: Item {}
          QQC.ContextMenu.menu: null
          TextContextMenu { editor: parent }
          Keys.onPressed: function(e) { if (e.key === Qt.Key_Escape) { root.closed(); e.accepted = true } }
        }
      }
    }
    Flickable {
      width: parent.width
      height: parent.height - y
      contentHeight: content.implicitHeight + Style.space(20)
      clip: true
      boundsBehavior: Flickable.StopAtBounds
      Column {
        id: content
        width: parent.width
        spacing: Style.space(4)
        // text results
        Repeater {
          model: root.results
          delegate: Item {
            required property var modelData
            width: content.width; height: Style.space(52)
            Rectangle { anchors.fill: parent; anchors.margins: Style.space(4); anchors.leftMargin: Style.space(10); anchors.rightMargin: Style.space(10); radius: Style.space(12); color: rh.containsMouse ? Util.alpha(root.fg, 0.05) : "transparent" }
            Column {
              anchors.left: parent.left; anchors.leftMargin: Style.space(18); anchors.right: parent.right; anchors.rightMargin: Style.space(18); anchors.verticalCenter: parent.verticalCenter
              Text { width: parent.width; elide: Text.ElideRight; text: (modelData.isOwn ? "You" : modelData.senderName); color: Util.alpha(root.fg, 0.6); font.family: Fonts.ui; font.pixelSize: Style.font.caption; font.bold: true }
              Text { width: parent.width; elide: Text.ElideRight; text: modelData.body; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall }
            }
            MouseArea { id: rh; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; onClicked: root.jumpTo(modelData.eventId) }
          }
        }
        Text { visible: root.query.length >= 2 && root.results.length === 0; width: parent.width; horizontalAlignment: Text.AlignHCenter; topPadding: Style.space(20); text: "No matches"; color: Util.alpha(root.fg, 0.5); font.family: Fonts.ui; font.pixelSize: Style.font.body }
        // idle: media + links
        Text { visible: root.query.length < 2 && root.images.length > 0; text: "Images"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.subtitle; font.bold: true; leftPadding: Style.space(16); topPadding: Style.space(8) }
        Flow {
          visible: root.query.length < 2
          width: parent.width - Style.space(28)
          anchors.horizontalCenter: parent.horizontalCenter
          spacing: Style.space(6)
          Repeater {
            model: root.images
            delegate: Rectangle {
              required property var modelData
              width: Style.space(84); height: Style.space(84); radius: Style.space(10)
              antialiasing: true
              clip: true
              color: Util.alpha(root.fg, 0.08)
              Image { anchors.fill: parent; fillMode: Image.PreserveAspectCrop; asynchronous: true; sourceSize.width: 200; source: (modelData.media && modelData.media.thumbnailPath) ? "file://" + modelData.media.thumbnailPath : "" }
              MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.openImage(modelData) }
            }
          }
        }
        Text { visible: root.query.length < 2 && root.links.length > 0; text: "Links"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.subtitle; font.bold: true; leftPadding: Style.space(16); topPadding: Style.space(10) }
        Repeater {
          model: root.query.length < 2 ? root.links : []
          delegate: Item {
            required property var modelData
            width: content.width; height: Style.space(56)
            Rectangle { anchors.fill: parent; anchors.margins: Style.space(4); anchors.leftMargin: Style.space(14); anchors.rightMargin: Style.space(14); radius: Style.space(14); color: Util.alpha(root.fg, 0.06) }
            Column {
              anchors.left: parent.left; anchors.leftMargin: Style.space(24); anchors.right: parent.right; anchors.rightMargin: Style.space(24); anchors.verticalCenter: parent.verticalCenter
              Text { width: parent.width; elide: Text.ElideMiddle; text: modelData.url; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall }
              Text { width: parent.width; elide: Text.ElideRight; text: (modelData.isOwn ? "You" : modelData.senderName); color: Util.alpha(root.fg, 0.5); font.family: Fonts.ui; font.pixelSize: Style.font.caption }
            }
            MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: Qt.openUrlExternally(modelData.url) }
          }
        }
      }
    }
  }
}
