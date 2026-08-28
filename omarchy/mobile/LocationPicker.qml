import QtQuick
import QtQuick.Effects
import qs.Commons
import qs.Ui
import "../components"

// The attachment sheet's three location pages: one picker in three modes.
//   "current" — your position, avatar marker, share once
//   "live"    — your position plus how long to keep sharing it
//   "pin"     — tap the map to place a point, then share that
// Only "pin" works without a position source (GeoClue on the system bus).
Item {
  id: root
  property var svc: null
  property string mode: "pin"          // current | live | pin
  property color fg: Color.menu.text
  property color accent: Color.accent
  property color surface: Util.alpha(Color.menu.text, 0.10)
  property color chip: Util.alpha(Color.background, 0.85)
  /// Tone for anything read over a map; alpha is forced to 1.
  property color menuSurface: Color.popups.background
  readonly property color menuSolid: Qt.rgba(root.menuSurface.r, root.menuSurface.g, root.menuSurface.b, 1)

  signal backRequested()
  signal closeRequested()
  /// `durationMs` is 0 for a one-off share.
  signal shareRequested(real lat, real lon, real durationMs)

  readonly property bool pinMode: root.mode === "pin"
  readonly property bool liveMode: root.mode === "live"
  readonly property string styleUrl: root.svc ? (root.svc.mapStyleUrl || "") : ""
  readonly property bool mapReady: root.styleUrl !== "" && !!(root.svc && root.svc.mapsAvailable)

  /// Whether the position source has a fix; false until GeoClue answers.
  readonly property bool haveFix: !!(root.svc && root.svc.positionKnown)
  readonly property real fixLat: root.svc && root.svc.positionKnown ? root.svc.positionLat : 0
  readonly property real fixLon: root.svc && root.svc.positionKnown ? root.svc.positionLon : 0

  // What the map points at. In pin mode the tap moves it, otherwise it follows the fix.
  property real markLat: 0
  property real markLon: 0
  property bool marked: false

  // Fallback centre when there is nothing better.
  readonly property real openLat: root.marked ? root.markLat : (root.haveFix ? root.fixLat : 39.5)
  readonly property real openLon: root.marked ? root.markLon : (root.haveFix ? root.fixLon : -98.35)

  property var durations: [
    { label: "Share for 15m", ms: 15 * 60 * 1000 },
    { label: "Share for 1h", ms: 60 * 60 * 1000 },
    { label: "Share for 8h", ms: 8 * 60 * 60 * 1000 }
  ]
  property int durationIndex: 0
  property bool durationOpen: false

  readonly property bool canShare: root.pinMode ? root.marked : root.haveFix

  /// Test hook: what the picker and its map actually believe.
  function debugState() {
    var it = mapLoader.item
    return JSON.stringify({
      mode: root.mode, mapReady: root.mapReady, haveFix: root.haveFix,
      fix: [root.fixLat, root.fixLon], open: [root.openLat, root.openLon],
      svcKnown: root.svc ? root.svc.positionKnown : null,
      avatar: root.svc ? (root.svc.avatarPath || "") : "",
      map: it ? { lat: it.lat, lon: it.lon, markerVisible: it.markerVisible,
                  markerAvatar: it.markerAvatar, interactive: it.interactive } : "not built"
    })
  }

  function reset() {
    root.marked = false
    root.durationOpen = false
    root.durationIndex = 0
  }

  // Map
  Item {
    id: mapMask
    anchors.fill: mapBox
    visible: false
    layer.enabled: true
    Rectangle { anchors.fill: parent; radius: Style.space(14); antialiasing: true; color: "black" }
  }

  Item {
    id: mapBox
    anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
    anchors.leftMargin: Style.space(16); anchors.rightMargin: Style.space(16)
    anchors.topMargin: Style.space(16)
    anchors.bottom: bottomBar.top
    anchors.bottomMargin: Style.space(14)
    // The map ignores its parent's rounded clip, so the whole box is masked.
    layer.enabled: true
    layer.smooth: true
    layer.effect: MultiEffect {
      maskEnabled: true
      maskSource: mapMask
      maskThresholdMin: 0.5
      maskSpreadAtMin: 1.0
    }

    Rectangle { anchors.fill: parent; color: root.surface }

    Loader {
      id: mapLoader
      anchors.fill: parent
      active: root.mapReady
      // Only the style must be right at construction: the MapLibre plugin reads it
      // once. Everything else is *bound* after loading — construction is a snapshot.
      function build() {
        setSource(Qt.resolvedUrl("MapView.qml"), { styleUrl: root.styleUrl, interactive: true, tappable: true })
      }
      function rebuild() { if (active) build() }
      onActiveChanged: { if (active) Qt.callLater(rebuild); else setSource("", {}) }
      Component.onCompleted: if (active) Qt.callLater(rebuild)
      onLoaded: {
        item.lat = Qt.binding(function () { return root.openLat })
        item.lon = Qt.binding(function () { return root.openLon })
        item.zoom = Qt.binding(function () { return (root.haveFix || root.marked) ? 16 : 4 })
        item.pinColor = Qt.binding(function () { return root.accent })
        item.markerAvatar = Qt.binding(function () {
          return root.pinMode ? "" : (root.svc ? (root.svc.avatarPath || "") : "")
        })
        item.markerVisible = Qt.binding(function () {
          return root.pinMode ? root.marked : root.haveFix
        })
        item.mapTapped.connect(function (la, lo) {
          if (!root.pinMode) return
          root.markLat = la; root.markLon = lo; root.marked = true
        })
      }
    }

    Column {
      anchors.centerIn: parent
      width: parent.width - Style.space(40)
      spacing: Style.space(8)
      visible: !root.mapReady
      IconLabel { filled: true; anchors.horizontalCenter: parent.horizontalCenter
        icon: Icons.location; color: Util.alpha(root.accent, 0.8); size: Style.space(36) }
      Text {
        width: parent.width; horizontalAlignment: Text.AlignHCenter; wrapMode: Text.Wrap
        text: root.styleUrl === ""
          ? "No map style is configured for this homeserver."
          : "The MapLibre plugin is not installed."
        color: Util.alpha(root.fg, 0.6)
        font.family: Fonts.ui; font.pixelSize: Style.font.caption
      }
    }
  }

  // Floating chrome
  Rectangle {
    anchors.left: mapBox.left; anchors.top: mapBox.top; anchors.margins: Style.space(8)
    width: Style.space(30); height: width; radius: width / 2
    color: root.chip
    z: 3
    IconLabel { anchors.centerIn: parent; icon: Icons.back; color: root.fg; size: Style.font.icon }
    MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.backRequested() }
  }

  Rectangle {
    anchors.right: mapBox.right; anchors.top: mapBox.top
    anchors.rightMargin: Style.space(8); anchors.topMargin: Style.space(8)
    width: Style.space(30); height: width; radius: width / 2
    color: root.chip
    z: 3
    visible: root.mapReady
    IconLabel { filled: true; anchors.centerIn: parent
      icon: Icons.recentre
      color: root.haveFix || root.marked ? root.accent : Util.alpha(root.fg, 0.4); size: Style.font.icon }
    MouseArea {
      anchors.fill: parent
      cursorShape: Qt.PointingHandCursor
      onClicked: if (mapLoader.item) mapLoader.item.resetView()
    }
  }

  Column {
    anchors.right: mapBox.right; anchors.bottom: mapBox.bottom
    anchors.rightMargin: Style.space(8); anchors.bottomMargin: Style.space(8)
    spacing: Style.space(4)
    z: 3
    visible: root.pinMode && root.mapReady
    Repeater {
      model: [{ t: "+", zoomIn: true }, { t: "−", zoomIn: false }]
      delegate: Rectangle {
        required property var modelData
        width: Style.space(28); height: width; radius: Style.space(8)
        color: root.chip
        Text { anchors.centerIn: parent; text: modelData.t; color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: true }
        MouseArea {
          anchors.fill: parent
          cursorShape: Qt.PointingHandCursor
          onClicked: {
            if (!mapLoader.item) return
            if (modelData.zoomIn) mapLoader.item.zoomIn(); else mapLoader.item.zoomOut()
          }
        }
      }
    }
  }

  Rectangle {
    anchors.horizontalCenter: mapBox.horizontalCenter
    anchors.top: mapBox.top; anchors.topMargin: Style.space(8)
    width: hint.implicitWidth + Style.space(20); height: Style.space(26); radius: height / 2
    color: root.chip
    z: 3
    visible: root.mapReady && (root.pinMode || !root.haveFix)
    MouseArea {
      anchors.fill: parent
      enabled: !root.pinMode && !root.haveFix
      cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
      onClicked: if (root.svc) root.svc.refreshPosition()
    }
    Text {
      id: hint
      anchors.centerIn: parent
      text: root.pinMode
        ? (root.marked ? "Click to move the pin" : "Click to drop a pin")
        : (root.svc && root.svc.positionError !== "" ? "Location unavailable — tap to retry"
                                                     : "Finding your location…")
      color: root.fg
      font.family: Fonts.ui; font.pixelSize: Style.font.caption
    }
  }

  // Bottom bar
  Column {
    id: bottomBar
    anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom
    anchors.leftMargin: Style.space(16); anchors.rightMargin: Style.space(16)
    anchors.bottomMargin: Style.space(16)
    spacing: Style.space(8)

    // How long to keep sharing, for live location only.
    Item {
      width: parent.width
      height: root.liveMode ? Style.space(38) : 0
      visible: root.liveMode

      Rectangle {
        id: durationField
        anchors.fill: parent
        radius: Style.space(11)
        color: root.menuSolid
        Text {
          anchors.left: parent.left; anchors.leftMargin: Style.space(14)
          anchors.verticalCenter: parent.verticalCenter
          text: root.durations[root.durationIndex].label
          color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
        }
        IconLabel { anchors.right: parent.right; anchors.rightMargin: Style.space(12)
          anchors.verticalCenter: parent.verticalCenter
          icon: root.durationOpen ? Icons.chevronUp : Icons.chevronDown
          color: Util.alpha(root.fg, 0.7); size: Style.font.icon }
        MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.durationOpen = !root.durationOpen }
      }
    }

    Rectangle {
      width: parent.width
      height: Style.space(44)
      radius: height / 2
      antialiasing: true
      color: root.canShare ? Util.alpha(root.accent, 0.92) : root.chip
      opacity: root.canShare ? 1 : 0.6
      Text {
        anchors.centerIn: parent
        text: "Share location"
        color: root.canShare ? "#141414" : Util.alpha(root.fg, 0.7)
        font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: true
      }
      MouseArea {
        anchors.fill: parent
        enabled: root.canShare
        cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
        onClicked: {
          var la = root.pinMode ? root.markLat : root.fixLat
          var lo = root.pinMode ? root.markLon : root.fixLon
          root.shareRequested(la, lo, root.liveMode ? root.durations[root.durationIndex].ms : 0)
        }
      }
    }
  }

  Rectangle {
    visible: root.durationOpen && root.liveMode
    z: 5
    anchors.left: bottomBar.left; anchors.right: bottomBar.right
    anchors.bottom: bottomBar.bottom
    anchors.bottomMargin: Style.space(56)
    height: durCol.implicitHeight + Style.space(8)
    radius: Style.space(12)
    color: root.menuSolid
    Column {
      id: durCol
      anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
      anchors.margins: Style.space(4)
      Repeater {
        model: root.durations
        delegate: Rectangle {
          required property var modelData
          required property int index
          width: durCol.width; height: Style.space(34); radius: Style.space(9)
          color: index === root.durationIndex ? Util.alpha(root.accent, 0.25)
               : (dh.containsMouse ? Util.alpha(root.fg, 0.08) : "transparent")
          Text {
            anchors.left: parent.left; anchors.leftMargin: Style.space(12)
            anchors.verticalCenter: parent.verticalCenter
            text: modelData.label
            color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
          }
          MouseArea {
            id: dh
            anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor
            onClicked: { root.durationIndex = index; root.durationOpen = false }
          }
        }
      }
    }
  }
}
