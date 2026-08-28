import QtQuick
import QtQuick.Effects
import qs.Commons
import qs.Ui
import "../components"

// An `m.location` message laid out like a photo. With a MapLibre style URL
// (MSC3488) and the QtLocation MapLibre plugin this is a real map; otherwise it
// falls back to a pin on the bubble's tone.
Item {
  id: root
  property var location: null
  property color fg: Color.menu.text
  property color accent: Color.accent
  property color surface: Util.alpha(Color.menu.text, 0.10)
  /// Kept for callers; the map's corners are rounded by a layer mask now.
  property color frameC: Color.menu.background
  property var svc: null
  /// Draw the sender's face instead of a pin (MSC3488 `m.self`).
  property string markerAvatar: ""
  property bool live: false
  /// A live share whose window has passed: last position stays, greyed.
  property bool ended: false
  readonly property bool openable: !root.ended
  /// Share expiry in epoch ms. `real`, not `int`: epoch ms overflows QML's int.
  property real expiresAt: 0
  signal openRequested()

  property real topLeftRadius: Style.space(16)
  property real topRightRadius: Style.space(16)
  property real bottomLeftRadius: Style.space(16)
  property real bottomRightRadius: Style.space(16)

  readonly property real lat: (location && location.lat !== null && location.lat !== undefined) ? location.lat : 0
  readonly property real lon: (location && location.lon !== null && location.lon !== undefined) ? location.lon : 0
  readonly property bool haveFix: !!(location && location.lat !== null && location.lat !== undefined
                                     && location.lon !== null && location.lon !== undefined)
  readonly property string styleUrl: root.svc ? (root.svc.mapStyleUrl || "") : ""
  /// Set by the timeline when the bubble is near the viewport; true outside a list.
  property bool mapAllowed: true
  /// Latched: once built, keep the map — a bare proximity threshold rebuilds the
  /// renderer on every pass. Cleared when the delegate is recycled (`location` changes).
  property bool mapLatched: false
  onMapAllowedChanged: if (root.mapAllowed) root.mapLatched = true
  onLocationChanged: root.mapLatched = root.mapAllowed
  readonly property bool mapReady: root.haveFix && root.styleUrl !== "" && root.mapLatched
                                   && !!(root.svc && root.svc.mapsAvailable)
                                   && !(root.svc && root.svc.debugNoBubbleMaps)

  // The map draws through its own scene graph node and ignores the parent's
  // rounded clip, so the whole card is masked into a layer.
  Item {
    id: cardMask
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
    id: card
    anchors.fill: parent
    layer.enabled: true
    layer.smooth: true
    layer.effect: MultiEffect {
      maskEnabled: true
      maskSource: cardMask
      // A hard cutoff aliases the curve; these soften the edge.
      maskThresholdMin: 0.5
      maskSpreadAtMin: 1.0
      saturation: root.ended ? -1.0 : 0.0
      Behavior on saturation { NumberAnimation { duration: 600; easing.type: Easing.OutCubic } }
    }

    Rectangle { anchors.fill: parent; color: root.surface }

    Loader {
      id: mapLoader
      anchors.fill: parent
      // The MapLibre plugin reads its style parameter once, at construction:
      // properties assigned in onLoaded arrive too late and the map comes up
      // with no style at all. setSource passes them in at creation.
      active: root.mapReady
      function build() {
        setSource(Qt.resolvedUrl("MapView.qml"),
                  { styleUrl: root.styleUrl, lat: root.lat, lon: root.lon,
                    zoom: 15, interactive: false, pinColor: root.accent })
      }
      onActiveChanged: { if (active) build(); else setSource("", {}) }
      Component.onCompleted: if (active) build()
      // Everything except the style is bound *after* construction: setSource's
      // property map is a one-time snapshot and never updates again.
      onLoaded: {
        item.lat = Qt.binding(function () { return root.lat })
        item.lon = Qt.binding(function () { return root.lon })
        item.pinColor = Qt.binding(function () { return root.accent })
        item.markerAvatar = Qt.binding(function () { return root.markerAvatar })
        item.markerRipple = Qt.binding(function () { return root.live })
      }
    }

    Text {
      anchors.centerIn: parent
      visible: !root.mapReady
      text: Icons.location
      color: Util.alpha(root.accent, 0.85)
      font.family: Fonts.iconFilled; renderType: Text.NativeRendering
      font.pixelSize: Style.space(40)
    }

    Rectangle {
      id: countdown
      visible: root.live || root.ended
      anchors.right: parent.right
      anchors.top: parent.top
      anchors.margins: Style.space(8)
      // The dot and its spacing are part of the row: size the pill from the row.
      width: chipRow.width + Style.space(20)
      height: Style.space(24)
      radius: height / 2
      color: Util.alpha(Color.background, 0.72)

      // Ticks only while something is counting down.
      property real now: Date.now()
      Timer {
        interval: 1000
        repeat: true
        running: root.live && countdown.visible
        onTriggered: countdown.now = Date.now()
      }
      readonly property real remaining: Math.max(0, root.expiresAt - now)

      Row {
        id: chipRow
        anchors.centerIn: parent
        spacing: Style.space(5)
        Rectangle {
          anchors.verticalCenter: parent.verticalCenter
          width: Style.space(7); height: width; radius: width / 2
          color: root.live ? "#e8646a" : Util.alpha("#ececec", 0.5)
          SequentialAnimation on opacity {
            running: root.live
            loops: Animation.Infinite
            NumberAnimation { to: 0.3; duration: 800 }
            NumberAnimation { to: 1; duration: 800 }
          }
        }
        Text {
          id: countText
          anchors.verticalCenter: parent.verticalCenter
          text: {
            if (!root.live) return "Live location ended"
            var s = Math.floor(countdown.remaining / 1000)
            var h = Math.floor(s / 3600)
            var m = Math.floor((s % 3600) / 60)
            var sec = s % 60
            if (h > 0) return h + "h " + (m < 10 ? "0" : "") + m + "m"
            return m + ":" + (sec < 10 ? "0" : "") + sec
          }
          color: "#ececec"
          font.family: Fonts.ui
          font.pixelSize: Style.space(10)
          font.bold: true
        }
      }
    }

    MouseArea {
      anchors.fill: parent
      enabled: root.openable
      cursorShape: Qt.PointingHandCursor
      onClicked: root.openRequested()
    }
  }
}
