import QtQuick
import QtQuick.Effects
import Quickshell
import qs.Commons
import qs.Ui
import "../components"

// The audio player. Flat header, full-bleed content, a rounded shelf carrying
// the transport. No skip controls: there is one track and nowhere to skip to.
Item {
  id: root
  property var svc: null
  property color fg: Color.menu.text
  property color accent: Color.accent
  signal backRequested()

  /// Set by the panel before showing the page.
  property string roomId: ""
  property string eventId: ""
  property string title: ""
  property string sizeLabel: ""
  property var info: null
  property string status: ""          // "" | "loading" | "error"

  /// Driven by the panel's playback clock, the same one the bubbles use.
  property bool playing: false
  property real position: 0           // seconds
  signal toggleRequested()
  signal seekRequested(real seconds)

  readonly property string art: info ? (info.artPath || "") : ""
  readonly property bool haveArt: root.art !== ""
  readonly property real duration: info && info.duration ? info.duration / 1000 : 0
  readonly property color tone: (info && info.accent) ? Qt.color(info.accent) : root.accent
  readonly property real frac: root.duration > 0 ? Math.max(0, Math.min(1, root.position / root.duration)) : 0

  readonly property color sheetTone: {
    var c = Color.popups.background
    return Qt.rgba(c.r, c.g, c.b, 1)
  }

  function clock(t) {
    var s = Math.max(0, Math.round(t)), m = Math.floor(s / 60)
    return m + ":" + ((s % 60) < 10 ? "0" : "") + (s % 60)
  }

  property string toast: ""
  Timer { id: toastTimer; interval: 2600; onTriggered: root.toast = "" }
  function note(t) { root.toast = t; toastTimer.restart() }
  function download() {
    if (!root.svc || root.eventId === "") return
    root.note("Saving…")
    root.svc.saveMedia(root.roomId, root.eventId, Quickshell.env("HOME") + "/Downloads", function (r, e) {
      root.note(r && r.path ? "Saved to " + r.path : "Save failed" + (e && e.message ? ": " + e.message : ""))
    })
  }

  function debugPlayer() {
    return JSON.stringify({
      title: root.title, haveArt: root.haveArt,
      duration: Math.round(root.duration), pos: Math.round(root.position),
      playing: root.playing, status: root.status, tone: String(root.tone)
    })
  }

  // Header

  Item {
    id: header
    z: 3
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
      text: "Audio"
      color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.subtitle; font.bold: true
    }
  }

  // Stage

  Item {
    id: stage
    anchors.top: header.bottom
    anchors.left: parent.left; anchors.right: parent.right
    // Runs to the bottom of the page, not to the top of the shelf: the shelf has
    // rounded top corners and would leave notches of bare page ground showing.
    anchors.bottom: parent.bottom

    Item {
      id: stageMask
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
    Item {
      anchors.fill: parent
      layer.enabled: true
      layer.smooth: true
      layer.effect: MultiEffect {
        maskEnabled: true
        maskSource: stageMask
        maskThresholdMin: 0.5
        maskSpreadAtMin: 1.0
      }

      Rectangle {
        anchors.fill: parent
        gradient: Gradient {
          GradientStop { position: 0; color: Qt.darker(root.tone, 2.1) }
          GradientStop { position: 1; color: Util.alpha(Qt.darker(root.tone, 3.0), 1) }
        }
      }

      Column {
        id: stack
        anchors.horizontalCenter: parent.horizontalCenter
        y: Math.max(Style.space(10), (parent.height - shelf.height - height) / 2)
        width: Math.min(parent.width - Style.space(56), Style.space(300))
        spacing: 0

        Item {
          id: coverBox
          visible: root.haveArt
          width: parent.width
          height: visible ? width : 0

          Item {
            id: coverMask
            anchors.fill: parent
            visible: false
            layer.enabled: true
            Rectangle { anchors.fill: parent; radius: Style.space(18); antialiasing: true; color: "black" }
          }
          Image {
            anchors.fill: parent
            source: root.haveArt ? "file://" + root.art : ""
            fillMode: Image.PreserveAspectCrop
            asynchronous: true
            cache: true
            layer.enabled: true
            layer.smooth: true
            layer.effect: MultiEffect {
              maskEnabled: true
              maskSource: coverMask
              maskThresholdMin: 0.5
              maskSpreadAtMin: 1.0
            }
          }
          scale: root.playing ? 1.0 : 0.97
          Behavior on scale { NumberAnimation { duration: 400; easing.type: Easing.OutCubic } }
        }

        Rectangle {
          visible: !root.haveArt
          width: parent.width
          height: visible ? width : 0
          radius: Style.space(18)
          antialiasing: true
          color: Util.alpha("#ffffff", 0.07)
          IconLabel {
            anchors.centerIn: parent
            icon: root.status === "loading" ? "" : Icons.music
            color: Util.alpha("#ffffff", 0.5)
            size: Style.space(72)
          }
          Text {
            anchors.centerIn: parent
            visible: root.status === "loading"
            text: "Reading the track…"
            color: Util.alpha("#ffffff", 0.5)
            font.family: Fonts.ui
            font.pixelSize: Style.font.caption
          }
        }
      }

      Rectangle {
        z: 3
        anchors.right: parent.right; anchors.top: parent.top
        anchors.margins: Style.space(12)
        width: Style.space(38); height: width; radius: width / 2
        color: Util.alpha("#000000", 0.42)
        antialiasing: true
        IconLabel { anchors.centerIn: parent
          icon: Icons.download
          color: "#ffffff"; size: Style.font.icon }
        MouseArea {
          anchors.fill: parent
          enabled: root.eventId !== ""
          cursorShape: Qt.PointingHandCursor
          onClicked: root.download()
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
      anchors.leftMargin: Style.space(16); anchors.rightMargin: Style.space(16)
      spacing: Style.space(10)

      Text {
        width: parent.width; elide: Text.ElideMiddle
        text: root.title
        color: root.fg
        font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: true
      }

      // Scrubber
      Item {
        width: parent.width
        height: Style.space(16)

        Rectangle {
          id: track
          anchors.left: parent.left; anchors.right: parent.right
          anchors.verticalCenter: parent.verticalCenter
          height: Style.space(4); radius: height / 2
          color: Util.alpha(root.fg, 0.18)
          Rectangle {
            anchors.left: parent.left; anchors.top: parent.top; anchors.bottom: parent.bottom
            width: parent.width * root.frac
            radius: height / 2
            color: root.accent
          }
        }
        Rectangle {
          id: knob
          x: Math.max(0, Math.min(track.width - width, track.width * root.frac - width / 2))
          anchors.verticalCenter: parent.verticalCenter
          width: Style.space(13); height: width; radius: width / 2
          color: root.accent
          antialiasing: true
          scale: drag.active || scrub.pressed ? 1.3 : 1.0
          Behavior on scale { NumberAnimation { duration: 100 } }
        }
        MouseArea {
          id: scrub
          anchors.fill: parent
          anchors.margins: -Style.space(6)
          enabled: root.duration > 0
          cursorShape: Qt.PointingHandCursor
          onClicked: function (m) { root.seekRequested((m.x / width) * root.duration) }
        }
        DragHandler {
          id: drag
          target: null
          enabled: root.duration > 0
          xAxis.enabled: true; yAxis.enabled: false
          onCentroidChanged: if (active) root.seekRequested((centroid.position.x / track.width) * root.duration)
        }
      }

      // Transport + times
      Item {
        width: parent.width
        height: Style.space(48)

        Text {
          anchors.left: parent.left; anchors.verticalCenter: parent.verticalCenter
          text: root.clock(root.position)
          color: Util.alpha(root.fg, 0.6)
          font.family: Fonts.ui; font.pixelSize: Style.font.caption
        }

        Rectangle {
          anchors.centerIn: parent
          width: Style.space(48); height: width; radius: width / 2
          color: root.accent
          antialiasing: true
          scale: play.pressed ? 0.93 : 1.0
          Behavior on scale { NumberAnimation { duration: 90 } }
          IconLabel { anchors.centerIn: parent
            icon: root.playing ? Icons.pause : Icons.play
            color: "#141414"; size: Style.space(22) }
          MouseArea {
            id: play
            anchors.fill: parent
            cursorShape: Qt.PointingHandCursor
            onClicked: root.toggleRequested()
          }
        }

        Text {
          anchors.right: parent.right
          anchors.verticalCenter: parent.verticalCenter
          text: root.duration > 0 ? root.clock(root.duration) : (root.sizeLabel || "")
          color: Util.alpha(root.fg, 0.6)
          font.family: Fonts.ui; font.pixelSize: Style.font.caption
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
    Text {
      id: tt
      anchors.centerIn: parent
      text: root.toast; color: "#ececec"
      font.family: Fonts.ui; font.pixelSize: Style.font.caption
      elide: Text.ElideMiddle; width: Math.min(implicitWidth, root.width - Style.space(60))
    }
  }
}
