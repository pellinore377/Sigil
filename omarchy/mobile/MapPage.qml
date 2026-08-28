import QtQuick
import QtQuick.Window
import QtQuick.Effects
import qs.Commons
import qs.Ui
import "../components"

// Full-page view of a shared location. Full bleed is deliberate:
// `Panel.pagesMask` already round-clips every page.
Item {
  id: root
  property var svc: null
  property var item: null
  property color fg: Color.menu.text
  property color accent: Color.accent
  signal backRequested()

  /// Near-opaque: the translucent surfaces used elsewhere are unreadable here.
  readonly property color chip: Util.alpha(Color.background, 0.85)
  readonly property color sheetTone: {
    var c = Color.popups.background
    return Qt.rgba(c.r, c.g, c.b, 1)
  }

  readonly property var loc: (item && item.location) ? item.location : null
  readonly property var share: (item && item.liveShare) ? item.liveShare : null
  readonly property bool haveFix: !!(loc && loc.lat !== null && loc.lat !== undefined
                                     && loc.lon !== null && loc.lon !== undefined)
  readonly property real lat: root.haveFix ? loc.lat : 0
  readonly property real lon: root.haveFix ? loc.lon : 0
  readonly property string styleUrl: root.svc ? (root.svc.mapStyleUrl || "") : ""
  readonly property bool mapReady: root.haveFix && root.styleUrl !== "" && !!(root.svc && root.svc.mapsAvailable)

  readonly property bool own: !!(item && item.isOwn)
  readonly property string who: root.own ? "You" : ((item && item.senderName) || "Someone")
  readonly property bool liveKind: !!(item && item.kind === "liveLocation")
  readonly property bool live: !!(root.share && root.share.live)
  readonly property bool ended: root.liveKind && !root.live
  /// `real`, not `int`: an epoch millisecond overflows QML's 32-bit int.
  readonly property real expiresAt: root.share ? (root.share.expiresAt || 0) : 0
  /// Only a share we are still publishing can be stopped from here.
  readonly property bool stoppable: root.own && root.live && !!(root.svc && root.svc.liveSharing)
  /// Somewhere worth going: a dropped pin, or someone else's running share.
  readonly property bool openable: root.pinMarker || (root.liveKind && root.live && !root.own)
  /// Own position draws as your face; a dropped pin stays a pin.
  readonly property bool selfMarker: root.liveKind || !!(root.loc && root.loc.asset === "m.self")
  readonly property bool pinMarker: !root.selfMarker && root.haveFix
  readonly property string markerAvatar: root.selfMarker ? ((item && item.senderAvatarPath) || "") : ""

  readonly property string statusText: {
    if (root.ended) return "Live location ended"
    if (root.live) return "Sharing until " + Qt.formatTime(new Date(root.expiresAt), "h:mm AP")
    if (item && item.ts) return "Shared " + Qt.formatTime(new Date(item.ts), "h:mm AP")
    return "Shared location"
  }

  /// Test hook: recentre the way the button does.
  function debugReset() {
    if (mapLoader.item) mapLoader.item.resetView()
    return mapLoader.item ? mapLoader.item.debugInput() : "no map"
  }

  function debugZoom(d) {
    if (mapLoader.item) { if (d > 0) mapLoader.item.zoomIn(); else mapLoader.item.zoomOut() }
    return mapLoader.item ? mapLoader.item.debugInput() : "no map"
  }

  function debugPan(dx, dy) {
    if (mapLoader.item) mapLoader.item.debugPan(dx, dy)
    return mapLoader.item ? mapLoader.item.debugInput() : "no map"
  }

  // Page-level log: the MapView itself may be the thing being torn down.
  property var pageLog: []
  property real plogT0: 0
  function pnote(what) {
    if (root.plogT0 === 0) root.plogT0 = Date.now()
    if (root.pageLog.length < 200) root.pageLog.push([Date.now() - root.plogT0, what])
  }
  onMapReadyChanged: root.pnote("mapReady=" + root.mapReady)
  onHaveFixChanged: root.pnote("haveFix=" + root.haveFix)
  onItemChanged: root.pnote("timeline item replaced")
  onVisibleChanged: root.pnote("page.visible=" + root.visible)
  onEnabledChanged: root.pnote("page.enabled=" + root.enabled)
  Connections {
    target: mapLoader
    function onItemChanged() { root.pnote("loader.item=" + (mapLoader.item ? "built#" + mapLoader.builds : "DESTROYED")) }
    function onActiveChanged() { root.pnote("loader.active=" + mapLoader.active) }
  }
  Connections {
    target: sheet
    function onHeightChanged() { root.pnote("sheetH=" + Math.round(sheet.height)) }
  }
  // A Connections with a null target logs nothing; record which case it is.
  Component.onCompleted: root.pnote("watcher: window=" + (Window.window
      ? "attached active=" + Window.window.active + " vis=" + Window.window.visibility
      : "NULL (window events are NOT being observed)"))
  Connections {
    target: Window.window
    ignoreUnknownSignals: true
    function onActiveChanged() { root.pnote("window.active=" + (Window.window ? Window.window.active : "?")) }
    function onVisibilityChanged() { root.pnote("window.visibility=" + (Window.window ? Window.window.visibility : "?")) }
  }

  /// Test switch: override the gesture inset. -1 = follow the sheet.
  property real debugInsetOverride: -1
  function debugPanMode(m) {
    if (mapLoader.item) mapLoader.item.panMode = m
    return mapLoader.item ? ("panMode=" + mapLoader.item.panMode) : "no map"
  }
  function debugIsolate(n) {
    if (mapLoader.item) mapLoader.item.isolate = n
    return mapLoader.item ? ("isolate=" + mapLoader.item.isolate) : "no map"
  }
  function debugInset(px) {
    root.debugInsetOverride = px
    return mapLoader.item ? String(mapLoader.item.gestureBottomInset) : "no map"
  }

  function debugEvents() {
    var page = root.pageLog
    root.pageLog = []
    root.plogT0 = 0
    return JSON.stringify({ window: (Window.window
                              ? "attached active=" + Window.window.active + " vis=" + Window.window.visibility
                              : "NULL — window events NOT observed"),
                            page: page,
                            view: mapLoader.item ? JSON.parse(mapLoader.item.debugEvents()) : "no map" })
  }

  function debugTrace(on) {
    return mapLoader.item ? mapLoader.item.debugTrace(on) : "no map"
  }

  function debugTraceRead() {
    return mapLoader.item ? mapLoader.item.debugTraceRead() : "no map"
  }

  function debugMap() {
    return JSON.stringify({
      pageLat: root.lat, pageLon: root.lon, haveFix: root.haveFix,
      pin: root.pinMarker, openable: root.openable,
      who: root.who, live: root.live, ended: root.ended, stoppable: root.stoppable,
      status: root.statusText,
      sheetH: Math.round(sheet.height),
      loc: root.loc ? { lat: root.loc.lat, lon: root.loc.lon, geo: root.loc.geoUri } : null,
      builds: mapLoader.builds,
      map: mapLoader.active ? JSON.parse(mapLoader.probe()) : "loader inactive"
    })
  }

  property string toast: ""
  Timer { id: toastTimer; interval: 2200; onTriggered: root.toast = "" }
  function note(t) { root.toast = t; toastTimer.restart() }

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
      text: "Location"
      color: root.fg; font.family: Fonts.ui; font.pixelSize: Style.font.subtitle; font.bold: true
    }
  }

  // Map

  Item {
    id: mapArea
    anchors.top: header.bottom
    anchors.left: parent.left; anchors.right: parent.right
    anchors.bottom: parent.bottom

    // Layer mask, not a parent clip: a map draws through its own scene-graph node
    // and ignores a parent's rounded clip.
    Item {
      id: mapMask
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
      id: mapClip
      anchors.fill: parent
      layer.enabled: true
      layer.smooth: true
      layer.effect: MultiEffect {
        maskEnabled: true
        maskSource: mapMask
        // A hard cutoff aliases the curve; these soften the edge.
        maskThresholdMin: 0.5
        maskSpreadAtMin: 1.0
      }

      Rectangle { anchors.fill: parent; color: Util.alpha(root.fg, 0.08) }

      Loader {
        id: mapLoader
        anchors.fill: parent
        // See LocationBody: the style has to be in place before the Map is
        // constructed, so it goes in through setSource, not onLoaded.
        active: root.mapReady
        property int builds: 0
        function build() {
          builds++
          builtStyle = root.styleUrl
          setSource(Qt.resolvedUrl("MapView.qml"),
                    { styleUrl: root.styleUrl, lat: root.lat, lon: root.lon,
                      zoom: 16, interactive: true, pinColor: root.accent })
        }
        function probe() {
          if (!item) return "no map item"
          return JSON.stringify({ viewLat: item.lat, viewLon: item.lon, viewStyle: item.styleUrl !== "",
                                  w: Math.round(item.width), h: Math.round(item.height),
                                  input: JSON.parse(item.debugInput()) })
        }
        // Everything but the style is bound after construction: setSource's
        // property map is a one-time snapshot, and a live share moves.
        onLoaded: {
          item.lat = Qt.binding(function () { return root.lat })
          item.lon = Qt.binding(function () { return root.lon })
          item.pinColor = Qt.binding(function () { return root.accent })
          item.markerAvatar = Qt.binding(function () { return root.markerAvatar })
          item.markerRipple = Qt.binding(function () { return root.live })
          // The sheet resizes with its contents, so bind rather than pass a number.
          item.gestureBottomInset = Qt.binding(function () {
            return root.debugInsetOverride >= 0 ? root.debugInsetOverride : sheet.height
          })
        }
        // Via Qt.callLater, never straight from a change handler: `onLatChanged` reads a stale `root.lon`.
        function rebuild() { if (active) build() }
        onActiveChanged: { if (active) Qt.callLater(rebuild); else setSource("", {}) }
        Component.onCompleted: if (active) Qt.callLater(rebuild)

        // Rebuild ONLY on a style change: a live share replaces `root.item` on every
        // beacon, and rebuilding on that tears down the map and any in-flight drag.
        property string builtStyle: ""
        Connections {
          target: root
          function onStyleUrlChanged() {
            if (root.styleUrl !== mapLoader.builtStyle) Qt.callLater(mapLoader.rebuild)
          }
        }
      }

      // Say which is missing, rather than showing an empty rectangle.
      Column {
        anchors.centerIn: parent
        anchors.verticalCenterOffset: -sheet.height / 2
        width: parent.width - Style.space(48)
        spacing: Style.space(10)
        visible: !root.mapReady
        Text {
          anchors.horizontalCenter: parent.horizontalCenter
          text: Icons.location
          color: Util.alpha(root.accent, 0.85)
          font.family: Fonts.iconFilled; renderType: Text.NativeRendering
          font.pixelSize: Style.space(46)
        }
        Text {
          width: parent.width
          horizontalAlignment: Text.AlignHCenter
          wrapMode: Text.Wrap
          text: root.styleUrl === ""
            ? "This homeserver publishes no map style, so there is nothing to draw. Point Sigil at a tile server and the pin becomes a map."
            : "The MapLibre plugin is not installed. Install qt6-location and maplibre-native-qt, then restart the shell."
          color: Util.alpha(root.fg, 0.6)
          font.family: Fonts.ui
          font.pixelSize: Style.font.caption
        }
      }

    }

    // Time remaining, top left. The same chip the bubble carries.
    Rectangle {
      id: countdown
      z: 3
      visible: root.live || root.ended
      anchors.left: parent.left; anchors.top: parent.top
      anchors.margins: Style.space(12)
      width: chipRow.width + Style.space(22)
      height: Style.space(28)
      radius: height / 2
      color: root.chip

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
        spacing: Style.space(6)
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
          font.pixelSize: Style.font.caption
          font.bold: true
        }
      }
    }

    Rectangle {
      z: 3
      visible: root.mapReady
      anchors.right: parent.right; anchors.top: parent.top
      anchors.margins: Style.space(12)
      width: Style.space(38); height: width; radius: width / 2
      color: root.chip
      IconLabel { filled: true; anchors.centerIn: parent
        icon: Icons.recentre
        color: root.accent; size: Style.font.icon }
      MouseArea {
        anchors.fill: parent
        cursorShape: Qt.PointingHandCursor
        onClicked: if (mapLoader.item) mapLoader.item.resetView()
      }
    }
  }

  // Sheet

  Rectangle {
    id: sheet
    z: 4
    anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom
    height: sheetCol.implicitHeight + Style.space(32)
    topLeftRadius: Style.space(24)
    topRightRadius: Style.space(24)
    antialiasing: true
    color: root.sheetTone

    // The sheet floats over the map; a drag that starts here must not pan it.
    MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons }

    Column {
      id: sheetCol
      anchors.left: parent.left; anchors.right: parent.right
      anchors.top: parent.top; anchors.topMargin: Style.space(14)
      anchors.leftMargin: Style.space(14); anchors.rightMargin: Style.space(14)
      spacing: Style.space(10)

      Text {
        width: parent.width
        text: "On the map"
        color: root.fg
        font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: true
      }

      Rectangle {
        width: parent.width
        height: Style.space(60)
        radius: Style.space(18)
        antialiasing: true
        color: Util.alpha(root.fg, 0.10)

        Avatar {
          id: face
          anchors.left: parent.left; anchors.leftMargin: Style.space(11)
          anchors.verticalCenter: parent.verticalCenter
          size: Style.space(38)
          source: (root.item && root.item.senderAvatarPath) || ""
          name: (root.item && root.item.senderName) || ""
          userId: (root.item && root.item.sender) || ""
          // A finished share drains of colour, matching its marker.
          opacity: root.ended ? 0.55 : 1.0
          Behavior on opacity { NumberAnimation { duration: 300 } }
        }

        Column {
          anchors.left: face.right; anchors.leftMargin: Style.space(11)
          anchors.right: action.left; anchors.rightMargin: Style.space(10)
          anchors.verticalCenter: parent.verticalCenter
          spacing: Style.space(2)
          Text {
            width: parent.width; elide: Text.ElideRight
            text: root.who
            color: root.fg
            font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: true
          }
          // An Item, not a Row: a Row's children may not anchor, and this must elide.
          Item {
            width: parent.width
            height: statusLine.implicitHeight
            IconLabel { filled: true; id: statusGlyph
              anchors.left: parent.left; anchors.verticalCenter: parent.verticalCenter
              icon: Icons.recentre
              color: Util.alpha(root.fg, 0.5); size: Style.font.caption }
            Text {
              id: statusLine
              anchors.left: statusGlyph.right; anchors.leftMargin: Style.space(5)
              anchors.right: parent.right
              anchors.verticalCenter: parent.verticalCenter
              elide: Text.ElideRight
              text: root.statusText
              color: Util.alpha(root.fg, 0.6)
              font.family: Fonts.ui; font.pixelSize: Style.font.caption
            }
          }
        }

        // One action: Stop while we are broadcasting, Open for a destination.
        Rectangle {
          id: action
          visible: root.stoppable || root.openable
          anchors.right: parent.right; anchors.rightMargin: visible ? Style.space(10) : 0
          anchors.verticalCenter: parent.verticalCenter
          width: visible ? actionLabel.implicitWidth + Style.space(26) : 0
          height: Style.space(34)
          radius: height / 2
          antialiasing: true
          color: root.stoppable ? Util.alpha(root.fg, 0.90) : Util.alpha(root.accent, 0.90)
          Text {
            id: actionLabel
            anchors.centerIn: parent
            text: root.stoppable ? "Stop" : "Open"
            color: "#141414"
            font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall; font.bold: true
          }
          MouseArea {
            anchors.fill: parent
            enabled: root.stoppable || root.openable
            cursorShape: Qt.PointingHandCursor
            onClicked: {
              if (root.stoppable) {
                if (root.svc) root.svc.stopLiveLocation()
                root.note("Stopped sharing")
                return
              }
              // `geo:` usually has no registered handler on desktop, so an https map link is used there.
              Qt.openUrlExternally("https://www.google.com/maps/search/?api=1&query="
                                   + root.lat + "%2C" + root.lon)
              root.note("Opened in your browser")
            }
          }
        }
      }
    }
  }

  Rectangle {
    z: 5
    visible: root.toast !== ""
    anchors.horizontalCenter: parent.horizontalCenter
    anchors.bottom: sheet.top; anchors.bottomMargin: Style.space(12)
    width: tt.implicitWidth + Style.space(22); height: Style.space(28); radius: height / 2
    color: Util.alpha(Color.background, 0.85)
    Text { id: tt; anchors.centerIn: parent; text: root.toast; color: "#ececec"; font.family: Fonts.ui; font.pixelSize: Style.font.caption }
  }
}
