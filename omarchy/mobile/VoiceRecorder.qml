import QtQuick
import qs.Commons
import qs.Ui
import "../components"

// Voice recorder under the composer: idle -> recording -> review (play/attach).
Item {
  id: root
  property var svc: null
  property color fg: Color.menu.text
  property color accent: Color.accent
  property color surface: Util.alpha(Color.menu.text, 0.07)
  property color chip: Util.alpha(Color.background, 0.85)

  // A chat theme hands us light tints that the recorder's greys vanish against,
  // so pills sink to a dark value and card ink switches to dark.
  function lum(c) { return (0.299 * c.r + 0.587 * c.g + 0.114 * c.b) * c.a }
  readonly property color cardC: root.surface
  // Already sunk by the page (ChatPage.deepChipC).
  readonly property color pillC: root.chip
  // Ink for anything on the card; the theme's own text is too pale on a light tint.
  readonly property color ink: root.lum(root.cardC) > 0.30 ? "#17131b" : root.fg
  property string state: "idle"        // idle | recording | ready
  property real elapsed: 0
  property string clipPath: ""
  property real clipDuration: 0
  property var clipWaveform: []
  property var levels: []
  signal attached(string path, real duration, var waveform)
  signal cancelled()

  implicitHeight: Style.space(230)

  function reset() {
    root.state = "idle"; root.elapsed = 0; root.clipPath = ""
    root.clipDuration = 0; root.clipWaveform = []; root.levels = []
  }
  function startRecording() {
    if (!root.svc) return
    root.levels = []; root.elapsed = 0
    root.svc.voiceStart(function(r, e) {
      if (e) { root.state = "idle"; return }
      root.state = "recording"
    })
  }
  function stopRecording() {
    if (!root.svc) return
    root.svc.voiceStop(function(r, e) {
      if (r && r.path) {
        root.clipPath = r.path
        root.clipDuration = r.duration || root.elapsed
        root.clipWaveform = r.waveform || []
        root.state = "ready"
      } else root.state = "idle"
    })
  }
  function cancelAll() {
    if (root.svc && root.state === "recording") root.svc.voiceCancel()
    root.reset()
    root.cancelled()
  }
  function attach() {
    if (root.state === "recording") { root.stopRecording(); return }
    if (root.clipPath !== "") root.attached(root.clipPath, root.clipDuration, root.clipWaveform)
  }

  Timer {
    interval: 100; repeat: true; running: root.state === "recording"
    onTriggered: {
      root.elapsed += 0.1
      var l = root.levels.slice()
      l.push(root.svc ? root.svc.voiceLevel : 0)
      if (l.length > 60) l.shift()
      root.levels = l
    }
  }

  function fmt(t) {
    var s = Math.max(0, Math.floor(t)), m = Math.floor(s / 60)
    return (m < 10 ? "0" : "") + m + ":" + ((s % 60) < 10 ? "0" : "") + (s % 60)
  }

  Column {
    anchors.fill: parent
    anchors.margins: Style.space(10)
    spacing: Style.space(10)

    // Stage card
    Rectangle {
      id: stage
      width: parent.width
      height: parent.height - controls.height - Style.space(10)
      radius: Style.space(18)
      antialiasing: true
      color: root.cardC

      // idle
      Column {
        anchors.centerIn: parent
        spacing: Style.space(10)
        visible: root.state === "idle"
        Row {
          anchors.horizontalCenter: parent.horizontalCenter
          spacing: Style.space(4)
          Repeater {
            model: [0.35, 0.6, 0.85, 1.0, 0.8, 0.5, 0.3]
            delegate: Rectangle {
              required property var modelData
              width: Style.space(5)
              height: Math.round(Style.space(46) * modelData)
              radius: width / 2
              antialiasing: true
              color: Util.alpha(root.ink, 0.55)
              anchors.verticalCenter: parent.verticalCenter
            }
          }
        }
        Text { anchors.horizontalCenter: parent.horizontalCenter; text: "Tap to record your voice"; color: Util.alpha(root.ink, 0.8); font.family: Fonts.ui; font.pixelSize: Style.font.body }
      }

      // recording / ready
      Column {
        anchors.centerIn: parent
        width: parent.width - Style.space(28)
        spacing: Style.space(14)
        visible: root.state !== "idle"
        Row {
          anchors.horizontalCenter: parent.horizontalCenter
          spacing: Style.space(7)
          Rectangle {
            visible: root.state === "recording"
            width: Style.space(9); height: Style.space(9); radius: width / 2
            color: "#e88b90"
            anchors.verticalCenter: parent.verticalCenter
            SequentialAnimation on opacity { running: root.state === "recording"; loops: Animation.Infinite; NumberAnimation { to: 0.25; duration: 700 } NumberAnimation { to: 1; duration: 700 } }
          }
          Text {
            text: root.fmt(root.state === "recording" ? root.elapsed : root.clipDuration)
            color: root.ink; font.family: Fonts.ui; font.pixelSize: Style.font.title
            anchors.verticalCenter: parent.verticalCenter
          }
        }
        Item {
          width: parent.width; height: Style.space(48)
          Row {
            anchors.centerIn: parent
            spacing: Style.space(2)
            Repeater {
              model: root.state === "recording" ? root.levels : root.clipWaveform
              delegate: Rectangle {
                required property var modelData
                width: Style.space(3)
                height: Math.max(Style.space(3), Style.space(44) * Math.min(1, modelData))
                radius: width / 2
                color: Util.alpha(root.ink, 0.8)
                anchors.verticalCenter: parent.verticalCenter
              }
            }
          }
        }
      }
    }

    // Controls: three pills under the card
    Row {
      id: controls
      width: parent.width
      height: Style.space(46)
      spacing: Style.space(8)
      readonly property real sideW: (width - Style.space(16) - Style.space(120)) / 2

      Rectangle {
        width: controls.sideW; height: parent.height; radius: height / 2
        antialiasing: true
        color: root.pillC
        Row {
          anchors.centerIn: parent; spacing: Style.space(6)
          IconLabel { icon: root.state === "recording" ? Icons.refresh : Icons.close; color: root.fg; anchors.verticalCenter: parent.verticalCenter; size: Style.font.icon; fitWidth: true }
          Text { text: root.state === "recording" ? "Restart" : "Cancel"; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body; anchors.verticalCenter: parent.verticalCenter }
        }
        MouseArea {
          anchors.fill: parent; cursorShape: Qt.PointingHandCursor
          onClicked: {
            if (root.state === "recording") { root.svc.voiceCancel(); root.state = "idle"; Qt.callLater(root.startRecording) }
            else root.cancelAll()
          }
        }
      }

      Rectangle {
        width: Style.space(120); height: parent.height; radius: height / 2
        antialiasing: true
        color: root.state === "recording" ? Qt.lighter(Color.urgent, 1.25) : Qt.lighter(root.accent, 1.3)
        IconLabel { filled: true; anchors.centerIn: parent
          icon: root.state === "recording" ? Icons.stop : Icons.record
          color: Color.background; size: Style.space(22) }
        MouseArea {
          anchors.fill: parent; cursorShape: Qt.PointingHandCursor
          onClicked: {
            if (root.state === "recording") root.stopRecording()
            else if (root.state === "ready") { root.reset(); root.startRecording() }
            else root.startRecording()
          }
        }
      }

      Rectangle {
        width: controls.sideW; height: parent.height; radius: height / 2
        antialiasing: true
        color: root.state === "idle" ? root.pillC : Qt.lighter(root.accent, 1.3)
        opacity: root.state === "idle" ? 0.6 : 1
        Row {
          anchors.centerIn: parent; spacing: Style.space(6)
          IconLabel { icon: Icons.check; color: root.state === "idle" ? root.fg : "#1a1a1a"; anchors.verticalCenter: parent.verticalCenter; size: Style.font.icon; fitWidth: true }
          Text { text: "Attach"; color: root.state === "idle" ? root.fg : "#1a1a1a"; font.family: Fonts.ui; font.pixelSize: Style.font.body; anchors.verticalCenter: parent.verticalCenter }
        }
        MouseArea { anchors.fill: parent; enabled: root.state !== "idle"; cursorShape: Qt.PointingHandCursor; onClicked: root.attach() }
      }
    }
  }
}
