import QtQuick
import QtQuick.Effects
import qs.Commons
import qs.Ui
import "../components"
import ".."

// Chat theme: live mini preview, Apply/Reset, tri-colour palettes derived from
// one base hue (bubbles / send button / background tint) + wallpapers.
Item {
  id: root
  property var svc: null
  property color fg: Color.menu.text
  property string roomId: ""
  property var theme: ({})
  property var pending: ({})
  signal closed()
  signal applied(var t)
  signal choosePhoto()

  function reset() { root.pending = JSON.parse(JSON.stringify(root.theme || {})) }
  function scrollToEnd() { themeFlick.contentY = Math.max(0, themeFlick.contentHeight - themeFlick.height); console.log("THEMESCROLL contentH", themeFlick.contentHeight, "viewH", themeFlick.height, "bodyImplicit", themeFlick.contentItem.children[0] ? themeFlick.contentItem.children[0].implicitHeight : -1) }

  readonly property var palette: ["", "#7c9fd4", "#5cb8d6", "#b48ad6", "#9aab7e", "#e0a370", "#d98aa8"]
  readonly property bool customSel: (root.pending.accent || "") !== "" && root.palette.indexOf(root.pending.accent) < 0
  property bool pickingColor: false
  property real pickH: 0.6
  property real pickS: 0.7
  property real pickV: 0.8
  function hexOf(c) {
    function b(v) { return ("0" + Math.round(Math.max(0, Math.min(1, v)) * 255).toString(16)).slice(-2) }
    return "#" + b(c.r) + b(c.g) + b(c.b)
  }
  // Nine gradients derived from the pending accent (3 hue shifts x 3 depths)
  function gradPair(i) {
    var base = root.pendAccent
    var h = base.hslHue < 0 ? 0.6 : base.hslHue
    var hh = (h + [-0.04, 0, 0.04][i % 3] + 1) % 1
    var row = Math.floor(i / 3)
    var sat = Math.max(0.5, base.hslSaturation)
    var l = Math.max(0.25, Math.min(0.55, base.hslLightness))
    var top = Math.min(0.62, [l * 1.15, l * 0.85, l * 0.6][row])
    var bot = [l * 0.45, l * 0.3, l * 0.18][row]
    return [Qt.hsla(hh, sat, top, 1), Qt.hsla(hh, sat, bot, 1)]
  }
  readonly property color pendAccent: (root.pending.accent || "") !== "" ? Qt.color(root.pending.accent) : Color.accent
  readonly property var roomInfo: (svc && roomId) ? svc.room(roomId) : null

  function tintBg(base, amt) {
    var d = Qt.darker(Color.menu.background, 1.35)
    return Qt.rgba(d.r * (1 - amt) + base.r * amt, d.g * (1 - amt) + base.g * amt, d.b * (1 - amt) + base.b * amt, 1)
  }
  function bubbleFill(base) {
    var bg = Color.popups.background
    return Qt.rgba(base.r * 0.42 + bg.r * 0.58, base.g * 0.42 + bg.g * 0.58, base.b * 0.42 + bg.b * 0.58, 1)
  }
  readonly property color otherFill: {
    var base = bubbleFill(fg)
    if ((root.pending.accent || "") === "") return base
    var a = root.pendAccent
    return Qt.rgba(base.r * 0.82 + a.r * 0.18, base.g * 0.82 + a.g * 0.18, base.b * 0.82 + a.b * 0.18, 1)
  }

  // Custom colour picker: bottom sheet
  Item {
    anchors.fill: parent
    z: 50
    Rectangle {
      anchors.fill: parent; radius: Style.space(22); antialiasing: true; color: "#000000"
      opacity: root.pickingColor ? 0.45 : 0
      visible: opacity > 0
      Behavior on opacity { NumberAnimation { duration: 180 } }
      MouseArea { anchors.fill: parent; enabled: root.pickingColor; onClicked: root.pickingColor = false }
    }
    Rectangle {
      anchors.left: parent.left; anchors.right: parent.right
      height: cardCol.implicitHeight + Style.space(26)
      y: root.pickingColor ? parent.height - height : parent.height + Style.space(6)
      topLeftRadius: Style.space(20); topRightRadius: Style.space(20)
      antialiasing: true
      color: Util.alpha(Color.popups.background, 0.98)
      Behavior on y { NumberAnimation { duration: 220; easing.type: Easing.OutCubic } }
      MouseArea { anchors.fill: parent }
      Column {
        id: cardCol
        anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
        anchors.margins: Style.space(16)
        anchors.topMargin: Style.space(10)
        spacing: Style.space(12)
        Rectangle { width: Style.space(36); height: Style.space(4); radius: 2; color: Util.alpha(root.fg, 0.25); anchors.horizontalCenter: parent.horizontalCenter }
        Text { width: parent.width; horizontalAlignment: Text.AlignHCenter; text: "Custom color"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.subtitle; font.bold: true }
        // Colour wheel: angle = hue, radius = saturation
        Item {
          width: parent.width; height: Style.space(190)
          Canvas {
            id: wheel
            x: (parent.width - width) / 2
            width: Style.space(190); height: Style.space(190)
            renderStrategy: Canvas.Immediate
            property real v: root.pickV
            onVChanged: requestPaint()
            onVisibleChanged: if (visible) requestPaint()
            onPaint: {
              var ctx = getContext("2d")
              var cx = width / 2, cy = height / 2, R = width / 2
              ctx.reset()
              ctx.clearRect(0, 0, width, height)
              var hueGrad = ctx.createConicalGradient(cx, cy, 0)
              for (var i = 0; i <= 6; i++) hueGrad.addColorStop(i / 6, Qt.hsla(i / 6 === 1 ? 0 : i / 6, 1, 0.5, 1))
              ctx.beginPath(); ctx.arc(cx, cy, R, 0, Math.PI * 2); ctx.closePath()
              ctx.fillStyle = hueGrad; ctx.fill()
              var satGrad = ctx.createRadialGradient(cx, cy, 0, cx, cy, R)
              satGrad.addColorStop(0, "rgba(255,255,255,1)")
              satGrad.addColorStop(1, "rgba(255,255,255,0)")
              ctx.fillStyle = satGrad; ctx.fill()
              ctx.fillStyle = "rgba(0,0,0," + (1 - v) + ")"; ctx.fill()
            }
            Rectangle {
              width: Style.space(14); height: Style.space(14); radius: width / 2
              color: "transparent"; border.width: 2; border.color: "white"
              antialiasing: true
              x: wheel.width / 2 + Math.cos(root.pickH * 2 * Math.PI) * root.pickS * (wheel.width / 2) - width / 2
              y: wheel.height / 2 - Math.sin(root.pickH * 2 * Math.PI) * root.pickS * (wheel.height / 2) - height / 2
            }
            MouseArea {
              anchors.fill: parent
              function pick(mx, my) {
                var cx = wheel.width / 2, cy = wheel.height / 2
                var dx = mx - cx, dy = cy - my
                root.pickS = Math.min(1, Math.sqrt(dx * dx + dy * dy) / cx)
                root.pickH = (Math.atan2(dy, dx) / (2 * Math.PI) + 1) % 1
              }
              onPressed: function(m) { pick(m.x, m.y) }
              onPositionChanged: function(m) { if (pressed) pick(m.x, m.y) }
            }
          }
        }
        // Brightness strip
        Rectangle {
          id: strip
          width: parent.width; height: Style.space(18); radius: height / 2
          antialiasing: true
          gradient: Gradient {
            orientation: Gradient.Horizontal
            GradientStop { position: 0; color: "black" }
            GradientStop { position: 1; color: Qt.hsva(root.pickH, root.pickS, 1, 1) }
          }
          Rectangle {
            width: Style.space(14); height: Style.space(14); radius: width / 2
            anchors.verticalCenter: parent.verticalCenter
            x: Math.round(root.pickV * (strip.width - width))
            color: Qt.hsva(root.pickH, root.pickS, root.pickV, 1); border.width: 2; border.color: "white"
            antialiasing: true
          }
          MouseArea {
            anchors.fill: parent
            function pick(mx) { root.pickV = Math.max(0.05, Math.min(1, mx / strip.width)) }
            onPressed: function(m) { pick(m.x) }
            onPositionChanged: function(m) { if (pressed) pick(m.x) }
          }
        }
        Row {
          anchors.horizontalCenter: parent.horizontalCenter
          spacing: Style.space(14)
          Rectangle { width: Style.space(36); height: Style.space(36); radius: width / 2; antialiasing: true; color: Qt.hsva(root.pickH, root.pickS, root.pickV, 1); anchors.verticalCenter: parent.verticalCenter }
          Rectangle {
            width: Style.space(104); height: Style.space(36); radius: height / 2
            antialiasing: true
            color: Qt.lighter(Qt.hsva(root.pickH, root.pickS, root.pickV, 1), 1.35)
            anchors.verticalCenter: parent.verticalCenter
            Text { anchors.centerIn: parent; text: "Accept"; color: "#1a1a1a"; font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: true }
            MouseArea {
              anchors.fill: parent; cursorShape: Qt.PointingHandCursor
              onClicked: {
                var t = JSON.parse(JSON.stringify(root.pending))
                t.accent = root.hexOf(Qt.hsva(root.pickH, root.pickS, root.pickV, 1))
                root.pending = t
                root.pickingColor = false
              }
            }
          }
        }
      }
    }
  }

  Column {
    anchors.fill: parent
    spacing: 0
    Item {
      width: parent.width; height: Style.space(56)
      PanelActionButton { id: backBtn; anchors.left: parent.left; anchors.leftMargin: Style.space(6); anchors.verticalCenter: parent.verticalCenter; fontFamily: Fonts.icon; iconText: Icons.back; foreground: root.fg; onClicked: root.closed() }
      Column {
        anchors.left: backBtn.right; anchors.leftMargin: Style.space(6); anchors.verticalCenter: parent.verticalCenter
        Text { text: "Chat theme"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.heading; font.bold: true }
        Text { text: "Only you will see these changes"; color: Util.alpha(root.fg, 0.55); font.family: Fonts.ui; font.pixelSize: Style.font.caption }
      }
      Rectangle {
        anchors.right: parent.right; anchors.rightMargin: Style.space(12); anchors.verticalCenter: parent.verticalCenter
        width: Style.space(74); height: Style.space(34); radius: height / 2
        antialiasing: true
        color: Qt.lighter(root.pendAccent, 1.35)
        Text { anchors.centerIn: parent; text: "Apply"; color: "#1a1a1a"; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall; font.bold: true }
        MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.applied(root.pending) }
      }
    }
    Flickable {
      id: themeFlick
      width: parent.width
      height: parent.height - y
      contentHeight: body.implicitHeight
      clip: true
      boundsBehavior: Flickable.StopAtBounds
      Column {
        id: body
        width: parent.width
        spacing: 0
    Item { width: parent.width; height: Style.space(6) }
    // Mini preview
    Item {
      width: parent.width; height: Style.space(300)
      Item {
        id: preview
        anchors.horizontalCenter: parent.horizontalCenter
        width: Style.space(196); height: Style.space(296)
        layer.enabled: true
        layer.smooth: true
        layer.effect: MultiEffect { maskEnabled: true; maskThresholdMin: 0.5; maskSpreadAtMin: 1.0; maskSource: prevMask }
        Rectangle { anchors.fill: parent; color: root.tintBg(root.pendAccent, (root.pending.accent || "") !== "" ? 0.18 : 0) }
        Image {
          anchors.fill: parent
          visible: (root.pending.wallpaper || "") !== "" && (root.pending.wallpaper || "").indexOf("grad:") !== 0
          fillMode: Image.PreserveAspectCrop
          asynchronous: true
          source: visible ? "file://" + root.pending.wallpaper : ""
        }
        Rectangle {
          id: prevGrad
          anchors.fill: parent
          visible: (root.pending.wallpaper || "").indexOf("grad:") === 0
          readonly property var gcols: root.gradPair(Math.min(8, Math.max(0, parseInt((root.pending.wallpaper || "grad:0").substring(5)) || 0)))
          gradient: Gradient {
            GradientStop { position: 0; color: prevGrad.gcols[0] }
            GradientStop { position: 1; color: prevGrad.gcols[1] }
          }
        }
        Row {
          x: Style.space(10); y: Style.space(10); spacing: Style.space(6)
          Rectangle { width: Style.space(16); height: Style.space(16); radius: width / 2; color: Qt.lighter(root.pendAccent, 1.3); anchors.verticalCenter: parent.verticalCenter }
          Text { text: root.roomInfo ? (root.roomInfo.name || "Chat") : "Chat"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.space(10); font.bold: true; anchors.verticalCenter: parent.verticalCenter }
        }
        Column {
          anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: miniComposer.top
          anchors.margins: Style.space(10); anchors.bottomMargin: Style.space(8)
          spacing: Style.space(5)
          Rectangle { width: Style.space(96); height: Style.space(18); radius: Style.space(9); antialiasing: true; color: bubbleFill(root.pendAccent); anchors.right: parent.right }
          Rectangle { width: Style.space(80); height: Style.space(18); radius: Style.space(9); antialiasing: true; color: root.otherFill }
          Rectangle { width: Style.space(110); height: Style.space(18); radius: Style.space(9); antialiasing: true; color: root.otherFill }
          Rectangle { width: Style.space(72); height: Style.space(18); radius: Style.space(9); antialiasing: true; color: bubbleFill(root.pendAccent); anchors.right: parent.right }
        }
        Row {
          id: miniComposer
          anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom
          anchors.margins: Style.space(10)
          spacing: Style.space(6)
          Rectangle { width: parent.width - Style.space(28); height: Style.space(22); radius: height / 2; antialiasing: true; color: Util.alpha(root.fg, 0.1)
            Text { anchors.verticalCenter: parent.verticalCenter; anchors.left: parent.left; anchors.leftMargin: Style.space(8); text: "Message"; color: Util.alpha(root.fg, 0.4); font.family: Fonts.ui; font.pixelSize: Style.space(9) } }
          Rectangle { width: Style.space(22); height: Style.space(22); radius: width / 2; antialiasing: true; color: Qt.darker(root.pendAccent, 1.15) }
        }
      }
      Item {
        id: prevMask
        anchors.fill: preview
        layer.enabled: true
        layer.smooth: true
        visible: false
        Rectangle { anchors.fill: parent; radius: Style.space(18); antialiasing: true; color: "black" }
      }
    }
    Item { width: parent.width; height: Style.space(8) }
    Rectangle {
      anchors.horizontalCenter: parent.horizontalCenter
      width: Style.space(150); height: Style.space(32); radius: height / 2
      antialiasing: true
      color: Util.alpha(root.fg, 0.1)
      Text { anchors.centerIn: parent; text: "Reset to default"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall }
      MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.pending = ({}) }
    }
    Item { width: parent.width; height: Style.space(10) }
    // Tinted lower section
    Rectangle {
      width: parent.width
      height: secCol.implicitHeight + Style.space(32)
      topLeftRadius: Style.space(20); topRightRadius: Style.space(20)
      bottomLeftRadius: Style.space(22); bottomRightRadius: Style.space(22)
      antialiasing: true
      color: root.tintBg(root.pendAccent, (root.pending.accent || "") !== "" ? 0.35 : 0.06)
      Column {
        id: secCol
        anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
        anchors.margins: Style.space(16)
        spacing: Style.space(8)
        Text { text: "Colors"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.subtitle; font.bold: true }
        Row {
          width: parent.width
          spacing: Style.space(8)
          Repeater {
            model: root.palette
            delegate: Item {
              required property var modelData
              readonly property color base: modelData === "" ? Color.accent : Qt.color(modelData)
              readonly property bool sel: (root.pending.accent || "") === modelData
              width: Style.space(38); height: Style.space(38)
              Rectangle { anchors.fill: parent; radius: width / 2; antialiasing: true; color: root.tintBg(parent.base, 0.4) }
              Item {
                width: parent.width; height: parent.height / 2
                clip: true
                Rectangle { width: parent.width; height: parent.width; radius: width / 2; antialiasing: true; color: Qt.lighter(parent.parent.base, 1.35) }
              }
              Rectangle { visible: parent.sel; anchors.fill: parent; anchors.margins: -3; radius: width / 2; color: "transparent"; border.width: 2; border.color: root.fg; antialiasing: true }
              Rectangle {
                visible: swatchHover.hovered && !parent.sel
                anchors.fill: parent; anchors.margins: -3; radius: width / 2
                color: "transparent"; border.width: 2; border.color: Util.alpha(root.fg, 0.45); antialiasing: true
              }
              scale: swatchHover.hovered ? 1.08 : 1
              Behavior on scale { NumberAnimation { duration: 110; easing.type: Easing.OutCubic } }
              HoverHandler { id: swatchHover }
              MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: { var t = JSON.parse(JSON.stringify(root.pending)); t.accent = modelData; root.pending = t } }
            }
          }
          Item {
            width: Style.space(38); height: Style.space(38)
            Rectangle {
              anchors.fill: parent; radius: width / 2; antialiasing: true
              gradient: Gradient {
                orientation: Gradient.Horizontal
                GradientStop { position: 0.0; color: "#d96a6a" }
                GradientStop { position: 0.25; color: "#d9c76a" }
                GradientStop { position: 0.5; color: "#6ad98c" }
                GradientStop { position: 0.75; color: "#6a9ed9" }
                GradientStop { position: 1.0; color: "#b06ad9" }
              }
            }
            Text { anchors.centerIn: parent; text: "+"; color: "#1a1a1a"; font.family: Fonts.ui; font.pixelSize: Style.font.icon; font.bold: true }
            Rectangle { visible: root.customSel; anchors.fill: parent; anchors.margins: -3; radius: width / 2; color: "transparent"; border.width: 2; border.color: root.fg; antialiasing: true }
            Rectangle {
              visible: customHover.hovered && !root.customSel
              anchors.fill: parent; anchors.margins: -3; radius: width / 2
              color: "transparent"; border.width: 2; border.color: Util.alpha(root.fg, 0.45); antialiasing: true
            }
            scale: customHover.hovered ? 1.08 : 1
            Behavior on scale { NumberAnimation { duration: 110; easing.type: Easing.OutCubic } }
            HoverHandler { id: customHover }
            MouseArea {
              anchors.fill: parent; cursorShape: Qt.PointingHandCursor
              onClicked: {
                if (root.customSel) { var c = Qt.color(root.pending.accent); root.pickH = Math.max(0, c.hsvHue); root.pickS = c.hsvSaturation; root.pickV = c.hsvValue }
                root.pickingColor = true
              }
            }
          }
        }
        Item { width: 1; height: Style.space(4) }
        Text { text: "Wallpapers"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.subtitle; font.bold: true }
        Rectangle {
          anchors.horizontalCenter: parent.horizontalCenter
          width: parent.width - Style.space(90); height: Style.space(38); radius: height / 2
          antialiasing: true
          color: Qt.lighter(root.pendAccent, 1.35)
          Row { anchors.centerIn: parent; spacing: Style.space(8)
            IconLabel { icon: Icons.image; color: "#1a1a1a"; anchors.verticalCenter: parent.verticalCenter; size: Style.font.icon }
            Text { text: "Choose a photo"; color: "#1a1a1a"; font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: true; anchors.verticalCenter: parent.verticalCenter } }
          MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.choosePhoto() }
        }
        Grid {
          id: gradGrid
          columns: 3
          columnSpacing: Style.space(8); rowSpacing: Style.space(8)
          width: parent.width
          readonly property real cell: (width - 2 * Style.space(8)) / 3
          Repeater {
            model: 9
            delegate: Rectangle {
              required property int index
              readonly property var gcols: root.gradPair(index)
              width: gradGrid.cell; height: gradGrid.cell; radius: Style.space(12)
              antialiasing: true
              gradient: Gradient {
                GradientStop { position: 0; color: gcols[0] }
                GradientStop { position: 1; color: gcols[1] }
              }
              readonly property bool sel: (root.pending.wallpaper || "") === ("grad:" + index)
              border.width: sel ? 2 : (gradHover.hovered ? 2 : 0)
              border.color: sel ? root.fg : Util.alpha(root.fg, 0.45)
              scale: gradHover.hovered && !sel ? 1.04 : 1
              Behavior on scale { NumberAnimation { duration: 110; easing.type: Easing.OutCubic } }
              HoverHandler { id: gradHover }
              MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: { var t = JSON.parse(JSON.stringify(root.pending)); t.wallpaper = "grad:" + index; root.pending = t } }
            }
          }
        }
      }
    }
      }
    }
  }
}
