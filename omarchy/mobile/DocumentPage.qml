import QtQuick
import Quickshell
import QtQuick.Controls as QQC
import QtQuick.Effects
import qs.Commons
import qs.Ui
import "../components"

// Reader for the engine's document previews. Three shapes: a sheet
// (spreadsheets, CSV, TSV), rendered Markdown, and blocks for everything else,
// with slides and PDF pages arriving as sections.
Item {
  id: root
  property var svc: null
  property color fg: Color.menu.text
  property color accent: Color.accent
  property color surface: Util.alpha(Color.menu.text, 0.08)
  signal backRequested()

  property var doc: null
  property string fileName: ""
  property string status: ""          // "" | "loading" | "error"
  property string error: ""
  property string roomId: ""
  property string eventId: ""
  property string sizeLabel: ""

  readonly property color paperTone: Qt.rgba(0.97, 0.96, 0.94, 1)
  readonly property color ink: Qt.rgba(0.16, 0.15, 0.14, 1)

  readonly property color sheetTone: {
    var c = Color.popups.background
    return Qt.rgba(c.r, c.g, c.b, 1)
  }

  property string toast: ""
  Timer { id: toastTimer; interval: 2600; onTriggered: root.toast = "" }
  function note(t) { root.toast = t; toastTimer.restart() }

  function download() {
    if (!root.svc || root.eventId === "") return
    root.note("Saving…")
    root.svc.saveMedia(root.roomId, root.eventId, Quickshell.env("HOME") + "/Downloads", function(r, e) {
      root.note(r && r.path ? "Saved to " + r.path : "Save failed" + (e && e.message ? ": " + e.message : ""))
    })
  }

  readonly property string kindWord: {
    switch (root.kind) {
      case "pdf": return "PDF"
      case "sheet": return "Spreadsheet"
      case "slides": return "Slides"
      case "markdown": return "Markdown"
      case "text": return "Text file"
      default: return "Document"
    }
  }

  readonly property string kind: doc ? (doc.kind || "") : ""
  readonly property var blocks: (doc && doc.blocks) ? doc.blocks : []
  /// A PDF the engine can draw; otherwise the reader falls back to extracted text.
  readonly property bool pdfPages: root.kind === "pdf" && !!(doc && doc.rasterisable) && root.pageCount > 0
  readonly property int pageCount: (doc && doc.pageCount) ? doc.pageCount : 0
  readonly property real pageAspect:
    (doc && doc.pageW > 0 && doc.pageH > 0) ? (doc.pageH / doc.pageW) : 1.294
  readonly property var sheets: (doc && doc.sheets) ? doc.sheets : []
  property int sheetIndex: 0
  readonly property var sheet: (root.sheets.length > root.sheetIndex) ? root.sheets[root.sheetIndex] : null

  function reset() { root.doc = null; root.status = "loading"; root.error = ""; root.sheetIndex = 0 }

  readonly property string subtitle: {
    if (root.status === "loading") return "Reading…"
    if (root.status === "error") return "Could not be read"
    if (!root.doc) return ""
    var bits = []
    if (root.kind === "sheet" && root.sheets.length > 0)
      bits.push(root.sheets.length === 1 ? "1 sheet" : root.sheets.length + " sheets")
    else if (root.doc.pages > 1)
      bits.push(root.doc.pages + (root.kind === "slides" ? " slides" : " pages"))
    if (root.doc.truncated) bits.push("preview shortened")
    if (root.doc.note !== "") bits.push(root.doc.note)
    return bits.join(" · ")
  }

  Item {
    id: header
    z: 2
    width: parent.width; height: Style.space(52)
    PanelActionButton {
      id: backBtn
      fontFamily: Fonts.icon
      anchors.left: parent.left; anchors.leftMargin: Style.space(6)
      anchors.verticalCenter: parent.verticalCenter
      iconText: Icons.back; foreground: root.fg
      onClicked: root.backRequested()
    }
    Text {
      anchors.left: backBtn.right; anchors.leftMargin: Style.space(8)
      anchors.right: parent.right; anchors.rightMargin: Style.space(12)
      anchors.verticalCenter: parent.verticalCenter
      elide: Text.ElideRight
      text: "Document"
      color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.subtitle; font.bold: true
    }
  }

  // Sheet tabs, for a workbook with more than one.
  Flickable {
    id: tabs
    visible: root.kind === "sheet" && root.sheets.length > 1
    anchors.top: header.bottom
    anchors.left: parent.left; anchors.right: parent.right
    anchors.topMargin: Style.space(12)
    anchors.leftMargin: Style.space(16); anchors.rightMargin: Style.space(16)
    height: visible ? Style.space(34) : 0
    contentWidth: tabRow.width
    flickableDirection: Flickable.HorizontalFlick
    clip: true
    Row {
      id: tabRow
      spacing: Style.space(6)
      Repeater {
        model: root.sheets
        delegate: Rectangle {
          required property var modelData
          required property int index
          readonly property bool on: index === root.sheetIndex
          height: Style.space(28)
          width: tabText.implicitWidth + Style.space(22)
          radius: height / 2
          antialiasing: true
          color: on ? Util.alpha(root.accent, 0.85) : Qt.rgba(0, 0, 0, 0.07)
          Text {
            id: tabText
            anchors.centerIn: parent
            text: modelData.name
            color: parent.on ? "#141414" : root.ink
            font.family: Fonts.ui; font.pixelSize: Style.font.caption; font.bold: parent.on
          }
          MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.sheetIndex = index }
        }
      }
    }
  }

  Item {
    id: paper
    anchors.top: header.bottom
    anchors.left: parent.left; anchors.right: parent.right
    anchors.bottom: parent.bottom

    Item {
      id: paperMask
      anchors.fill: parent
      visible: false
      layer.enabled: true
      Rectangle {
        anchors.fill: parent
        topLeftRadius: Style.space(24); topRightRadius: Style.space(24)
        antialiasing: true
        color: "black"
      }
    }
    Rectangle {
      anchors.fill: parent
      layer.enabled: true
      layer.smooth: true
      layer.effect: MultiEffect {
        maskEnabled: true
        maskSource: paperMask
        maskThresholdMin: 0.5
        maskSpreadAtMin: 1.0
      }
      color: root.paperTone
    }
  }

  Item {
    id: body
    anchors.top: tabs.visible ? tabs.bottom : header.bottom
    anchors.topMargin: Style.space(14)
    anchors.left: parent.left; anchors.right: parent.right
    anchors.bottom: shelf.top
    anchors.leftMargin: Style.space(16); anchors.rightMargin: Style.space(16)
    anchors.bottomMargin: Style.space(10)

    // Loading / error / empty
    Column {
      anchors.centerIn: parent
      width: parent.width - Style.space(40)
      spacing: Style.space(10)
      visible: root.status !== "" || (root.doc && root.kind === "")
      IconLabel {
        anchors.horizontalCenter: parent.horizontalCenter
        icon: root.status === "error" ? Icons.errorCircle : Icons.file
        color: root.status === "error" ? Color.urgent : Util.alpha(root.ink, 0.7)
        size: Style.space(40)
      }
      Text {
        width: parent.width
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.Wrap
        text: root.status === "error" ? root.error : "Reading the document…"
        color: Util.alpha(root.ink, 0.65)
        font.family: Fonts.ui; font.pixelSize: Style.font.caption
      }
    }

    // Markdown
    QQC.ScrollView {
      anchors.fill: parent
      visible: root.status === "" && root.kind === "markdown"
      clip: true
      contentWidth: availableWidth
      Text {
        width: parent.width
        text: root.doc ? (root.doc.html || "") : ""
        textFormat: Text.RichText
        wrapMode: Text.Wrap
        color: root.ink
        font.family: Fonts.ui
        font.pixelSize: Style.font.body
        // Sanitised by the engine; nothing here should reach the network either.
        onLinkActivated: function(link) { Qt.openUrlExternally(link) }
      }
    }

    // Sheet — a grid, not a table widget: scrolls both ways with a frozen first row.
    Flickable {
      id: grid
      anchors.fill: parent
      visible: root.status === "" && root.kind === "sheet"
      clip: true
      contentWidth: Math.max(width, rowsCol.width)
      contentHeight: rowsCol.height
      readonly property var rows: root.sheet ? root.sheet.rows : []
      readonly property int cols: {
        var n = 0
        for (var i = 0; i < grid.rows.length; i++) n = Math.max(n, grid.rows[i].length)
        return n
      }
      readonly property real colW: Style.space(120)

      Column {
        id: rowsCol
        width: Math.max(grid.width, grid.cols * grid.colW)
        Repeater {
          model: grid.rows
          delegate: Rectangle {
            required property var modelData
            required property int index
            width: rowsCol.width
            height: Style.space(30)
            // A spreadsheet's first row is nearly always a header.
            color: index === 0 ? Util.alpha(root.accent, 0.22)
                 : (index % 2 === 0 ? "transparent" : Qt.rgba(0, 0, 0, 0.035))
            Row {
              anchors.fill: parent
              Repeater {
                model: grid.cols
                delegate: Item {
                  required property int index
                  width: grid.colW
                  height: parent.height
                  Text {
                    anchors.fill: parent
                    anchors.leftMargin: Style.space(8); anchors.rightMargin: Style.space(8)
                    verticalAlignment: Text.AlignVCenter
                    text: index < parent.parent.parent.modelData.length ? parent.parent.parent.modelData[index] : ""
                    color: root.ink
                    font.family: Fonts.ui
                    font.pixelSize: Style.font.caption
                    font.bold: parent.parent.parent.index === 0
                    elide: Text.ElideRight
                  }
                  Rectangle {
                    anchors.right: parent.right; anchors.top: parent.top; anchors.bottom: parent.bottom
                    width: 1
                    color: Qt.rgba(0, 0, 0, 0.09)
                  }
                }
              }
            }
          }
        }
      }
    }

    // PDF pages — one rendered page per row, requested as it scrolls into view.
    // The placeholder is already the right shape, so scroll positions hold.
    ListView {
      id: pageList
      anchors.fill: parent
      visible: root.status === "" && root.pdfPages
      clip: true
      spacing: Style.space(10)
      cacheBuffer: Math.max(0, height * 2)
      model: root.pdfPages ? root.pageCount : 0
      delegate: Rectangle {
        required property int index
        width: pageList.width
        height: Math.round(width * root.pageAspect)
        color: root.paperTone
        radius: Style.space(4)
        antialiasing: true

        property string src: ""
        property bool failed: false
        Component.onCompleted: {
          if (!root.svc || root.eventId === "") return
          root.svc.docPage(root.roomId, root.eventId, index, Math.round(width * 2), function (r, e) {
            if (r && r.path) src = "file://" + r.path
            else failed = true
          })
        }
        Image {
          anchors.fill: parent
          source: parent.src
          fillMode: Image.PreserveAspectFit
          asynchronous: true
          cache: false
        }
        Text {
          anchors.centerIn: parent
          visible: parent.src === ""
          text: parent.failed ? "Page " + (parent.index + 1) + " could not be drawn"
                              : (parent.index + 1)
          color: Util.alpha(root.ink, 0.35)
          font.family: Fonts.ui
          font.pixelSize: parent.failed ? Style.font.caption : Style.font.subtitle
        }
      }
    }

    // Blocks
    ListView {
      id: blockList
      anchors.fill: parent
      visible: root.status === "" && !root.pdfPages
               && (root.kind === "document" || root.kind === "text"
                   || root.kind === "pdf" || root.kind === "slides")
      clip: true
      spacing: Style.space(4)
      model: root.blocks
      delegate: Item {
        required property var modelData
        width: blockList.width
        height: content.implicitHeight + (modelData.t === "section" ? Style.space(18) : Style.space(2))

        Rectangle {
          visible: modelData.t === "section"
          anchors.left: parent.left; anchors.right: parent.right
          anchors.verticalCenter: parent.verticalCenter
          height: 1
          color: Qt.rgba(0, 0, 0, 0.12)
        }
        Rectangle {
          visible: modelData.t === "section"
          anchors.centerIn: parent
          width: secLabel.implicitWidth + Style.space(16)
          height: Style.space(20)
          radius: height / 2
          color: Qt.rgba(0, 0, 0, 0.06)
          Text {
            id: secLabel
            anchors.centerIn: parent
            text: modelData.title || ""
            color: Util.alpha(root.ink, 0.6)
            font.family: Fonts.ui; font.pixelSize: Style.space(10); font.bold: true
          }
        }

        Text {
          id: content
          visible: modelData.t === "p"
          width: parent.width
          text: (modelData.bullet ? "•  " : "") + (modelData.text || "")
          color: root.ink
          wrapMode: Text.Wrap
          font.family: Fonts.ui
          // Level 1 is a title, 6 is barely larger than body text.
          font.pixelSize: modelData.level > 0
            ? Style.font.body + Style.space(Math.max(0, 7 - modelData.level))
            : Style.font.body
          font.bold: modelData.level > 0
          leftPadding: modelData.bullet ? Style.space(10) : 0
          topPadding: modelData.level > 0 ? Style.space(8) : 0
        }
      }
    }
  }

  // Shelf

  Rectangle {
    id: shelf
    z: 4
    anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom
    height: shelfCol.implicitHeight + Style.space(32)
    topLeftRadius: Style.space(24)
    topRightRadius: Style.space(24)
    antialiasing: true
    color: root.sheetTone
    MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons }

    Column {
      id: shelfCol
      anchors.left: parent.left; anchors.right: parent.right
      anchors.top: parent.top; anchors.topMargin: Style.space(14)
      anchors.leftMargin: Style.space(14); anchors.rightMargin: Style.space(14)
      spacing: Style.space(10)

      Text {
        width: parent.width
        text: "This file"
        color: root.fg
        font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: true
      }

      Rectangle {
        width: parent.width
        height: Style.space(60)
        radius: Style.space(18)
        antialiasing: true
        color: Util.alpha(root.fg, 0.10)

        Rectangle {
          id: fileIcon
          anchors.left: parent.left; anchors.leftMargin: Style.space(11)
          anchors.verticalCenter: parent.verticalCenter
          width: Style.space(38); height: width; radius: height / 2
          color: Util.alpha(root.accent, 0.25)
          IconLabel {
            anchors.centerIn: parent
            icon: Icons.file
            color: root.fg
            size: Style.font.icon
          }
        }

        Column {
          anchors.left: fileIcon.right; anchors.leftMargin: Style.space(11)
          anchors.right: saveBtn.left; anchors.rightMargin: Style.space(10)
          anchors.verticalCenter: parent.verticalCenter
          spacing: Style.space(2)
          Text {
            width: parent.width; elide: Text.ElideMiddle
            text: root.fileName
            color: root.fg
            font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: true
          }
          Text {
            width: parent.width; elide: Text.ElideRight
            text: {
              var bits = [root.kindWord]
              if (root.sizeLabel !== "") bits.push(root.sizeLabel)
              if (root.subtitle !== "") bits.push(root.subtitle)
              return bits.join(" · ")
            }
            color: Util.alpha(root.fg, 0.6)
            font.family: Fonts.ui; font.pixelSize: Style.font.caption
          }
        }

        Rectangle {
          id: saveBtn
          anchors.right: parent.right; anchors.rightMargin: Style.space(10)
          anchors.verticalCenter: parent.verticalCenter
          width: saveLabel.implicitWidth + Style.space(26)
          height: Style.space(34)
          radius: height / 2
          antialiasing: true
          color: Util.alpha(root.accent, 0.90)
          Text {
            id: saveLabel
            anchors.centerIn: parent
            text: "Download"
            color: "#141414"
            font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall; font.bold: true
          }
          MouseArea {
            anchors.fill: parent
            enabled: root.eventId !== ""
            cursorShape: Qt.PointingHandCursor
            onClicked: root.download()
          }
        }
      }
    }
  }

  Rectangle {
    z: 5
    visible: root.toast !== ""
    anchors.horizontalCenter: parent.horizontalCenter
    anchors.bottom: shelf.top; anchors.bottomMargin: Style.space(12)
    width: tt.implicitWidth + Style.space(22); height: Style.space(28); radius: height / 2
    color: Util.alpha(Color.background, 0.85)
    Text { id: tt; anchors.centerIn: parent; text: root.toast; color: "#ececec"; font.family: Fonts.ui; font.pixelSize: Style.font.caption; elide: Text.ElideMiddle; width: Math.min(implicitWidth, root.width - Style.space(60)) }
  }
}
