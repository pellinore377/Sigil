import QtQuick
import QtQuick.Effects
import qs.Commons
import qs.Ui
import "../components"

// A glimpse of a document, drawn as a page rather than an icon. The engine
// sends the first lines already flattened (`doc.thumb`); nothing is parsed here
// and nothing is rasterised, so no PDF renderer is needed.
Item {
  id: root
  /// `{kind, title, pages, lines:[{t:"p",text,level} | {t:"row",cells:[…]}]}`
  property var doc: null
  property color fg: Color.menu.text
  property color accent: Color.accent
  property real topLeftRadius: Style.space(16)
  property real topRightRadius: Style.space(16)
  property real bottomLeftRadius: 0
  property real bottomRightRadius: 0

  readonly property var lines: (doc && doc.lines) ? doc.lines : []
  /// A PDF arrives already rasterised (page one); everything else is typeset here.
  readonly property string image: doc ? (doc.imagePath || "") : ""
  readonly property real imageAspect:
    (doc && doc.imageWidth > 0 && doc.imageHeight > 0) ? (doc.imageHeight / doc.imageWidth) : 1.294
  readonly property bool grid: root.lines.length > 0 && root.lines[0].t === "row"
  readonly property string kind: doc ? (doc.kind || "") : ""
  /// Corner badge text. `kind` is the engine's family name; the extension is
  /// what people recognise.
  readonly property string badge: {
    switch (root.kind) {
      case "pdf": return "PDF"
      case "sheet": return "SHEET"
      case "slides": return "SLIDES"
      case "markdown": return "MD"
      case "document": return "DOC"
      default: return "TEXT"
    }
  }

  readonly property color paper: Qt.rgba(0.97, 0.96, 0.94, 1)
  readonly property color ink: Qt.rgba(0.16, 0.15, 0.14, 1)

  /// Actual line height, so a short note is not drawn on a full blank page.
  readonly property real contentHeight: root.image !== ""
    ? root.width * root.imageAspect
    : (root.grid ? sheet.implicitHeight + Style.space(20) : prose.implicitHeight + Style.space(24))
      + Style.space(14)
  readonly property bool clipped: root.contentHeight > root.height + 1

  // `clip: true` on a Rectangle clips to its bounding box, not its rounded
  // shape, so content paints over the corners. A layer mask actually cuts them.
  Item {
    id: pageMask
    anchors.fill: parent
    visible: false
    layer.enabled: true
    Rectangle {
      anchors.fill: parent
      topLeftRadius: root.topLeftRadius
      topRightRadius: root.topRightRadius
      bottomLeftRadius: root.bottomLeftRadius
      bottomRightRadius: root.bottomRightRadius
      antialiasing: true
      color: "black"
    }
  }

  Item {
    id: page
    anchors.fill: parent
    layer.enabled: true
    layer.smooth: true
    layer.effect: MultiEffect {
      maskEnabled: true
      maskSource: pageMask
      maskThresholdMin: 0.5
      maskSpreadAtMin: 1.0
    }

    Rectangle { anchors.fill: parent; color: root.paper }

    // Anchored to the top so a tall page is cropped where the fade already says
    // there is more.
    Image {
      visible: root.image !== ""
      anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
      height: root.width * root.imageAspect
      source: root.image !== "" ? "file://" + root.image : ""
      fillMode: Image.PreserveAspectFit
      asynchronous: true
      cache: true
    }

    Column {
      id: prose
      visible: !root.grid && root.image === ""
      anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
      anchors.margins: Style.space(12)
      spacing: Style.space(3)
      Repeater {
        model: (root.grid || root.image !== "") ? [] : root.lines
        delegate: Text {
          required property var modelData
          width: prose.width
          elide: Text.ElideRight
          maximumLineCount: 2
          wrapMode: Text.Wrap
          text: modelData.text || ""
          color: root.ink
          font.family: Fonts.ui
          font.pixelSize: (modelData.level > 0 && modelData.level < 3) ? Style.font.bodySmall : Style.font.caption
          font.bold: modelData.level > 0
        }
      }
    }

    // A spreadsheet. The first row is treated as a header; in practice it is one.
    Column {
      id: sheet
      visible: root.grid
      anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
      anchors.margins: Style.space(10)
      spacing: 0
      Repeater {
        model: root.grid ? root.lines : []
        delegate: Row {
          id: sheetRow
          required property var modelData
          required property int index
          readonly property int cols: Math.max(1, (modelData.cells || []).length)
          spacing: 0
          Repeater {
            model: sheetRow.modelData.cells || []
            delegate: Rectangle {
              required property var modelData
              required property int index
              width: Math.floor(sheet.width / sheetRow.cols)
              height: Style.space(18)
              color: sheetRow.index === 0 ? Qt.rgba(0, 0, 0, 0.06) : "transparent"
              border.width: 1
              border.color: Qt.rgba(0, 0, 0, 0.08)
              Text {
                anchors.fill: parent
                anchors.leftMargin: Style.space(4); anchors.rightMargin: Style.space(3)
                verticalAlignment: Text.AlignVCenter
                elide: Text.ElideRight
                text: String(parent.modelData)
                color: root.ink
                font.family: Fonts.ui
                font.pixelSize: Style.space(8)
                font.bold: sheetRow.index === 0
              }
            }
          }
        }
      }
    }

    Rectangle {
      visible: root.clipped
      anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom
      height: Style.space(34)
      gradient: Gradient {
        GradientStop { position: 0; color: Util.alpha(root.paper, 0) }
        GradientStop { position: 1; color: root.paper }
      }
    }

    Rectangle {
      anchors.left: parent.left; anchors.bottom: parent.bottom; anchors.margins: Style.space(8)
      width: badgeText.implicitWidth + Style.space(10); height: Style.space(16); radius: Style.space(4)
      color: Util.alpha("#000000", 0.55)
      Text {
        id: badgeText
        anchors.centerIn: parent
        text: root.badge + (root.doc && root.doc.pages > 1 ? " · " + root.doc.pages : "")
        color: "#ffffff"
        font.family: Fonts.ui; font.pixelSize: Style.space(8); font.bold: true
      }
    }
  }
}
