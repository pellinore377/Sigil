import QtQuick
import QtLocation as QL
import QtPositioning
import QtQuick.Effects
import "../components"
import QtQuick.Shapes

// The MapLibre map, isolated here so nothing else imports QtLocation:
// `Service.mapsRenderable()` probes this with Qt.createComponent and degrades to
// a pin card when the `maplibre-native-qt` plugin is not installed.
//
// `styleUrl` must point at a style JSON (MSC3488 `m.tile_server`, or a local
// override): MapLibre is a renderer, not a map. Interaction is hand-rolled —
// Qt 6 removed `MapGestureArea`, and MapView's own DragHandler lets other
// handlers steal the grab mid-pan.
Item {
  id: root
  property string styleUrl: ""
  property real lat: 0
  property real lon: 0
  property real zoom: 15
  /// Bubbles show a still map; the full page drags, flicks and pinches.
  property bool interactive: false
  onInteractiveChanged: root.note("interactive=" + root.interactive)
  /// Colour of the marker, so it can follow the chat theme.
  property color pinColor: "#e0a370"
  /// Radio waves from the marker, for a live share that is still running.
  property bool markerRipple: false
  /// Avatar used as the marker instead of a pin, for your own position.
  property string markerAvatar: ""
  property bool markerVisible: true
  /// Emitted with the coordinate under the pointer. Used by the pin picker.
  signal mapTapped(real lat, real lon)
  /// Off unless taps are wanted: an unlistened TapHandler still competes for the press.
  property bool tappable: false
  /// Bottom strip owned by something else (the detail sheet). The handlers must
  /// be confined to their own item: `CanTakeOverFromItems` defeats any blocking
  /// MouseArea laid over them, so a drag there would pan the map.
  property real gestureBottomInset: 0
  /// Handler bisection switch: 0 all on, 1 no pinch, 2 no wheel, 3 neither.
  property int isolate: 0
  /// "point" — `PointHandler`: passive grab only, sampled once a frame, so
  /// there is no exclusive grab for a touchscreen, tablet, gesture touchpad or
  /// compositor focus change to cancel. "drag" — `DragHandler`, for comparison.
  property string panMode: "point"

  /// Live pointer tracking for `panMode: "point"`.
  property bool tracking: false
  property real lastPx: 0
  property real lastPy: 0

  // Diagnostics.
  property int dragEvents: 0
  /// Timestamped pointer log, newest last. Capped: unbounded, it would leak.
  property var events: []
  property real t0: 0
  /// QPointingDevice::GrabTransition, which QML hands over as a bare int.
  function grabName(t) {
    switch (t) {
      case 0x01: return "GrabPassive"
      case 0x02: return "UngrabPassive"
      case 0x03: return "CancelGrabPassive"
      case 0x04: return "OverrideGrabPassive"
      case 0x10: return "GrabExclusive"
      case 0x20: return "UngrabExclusive"       // we let it go, normal end
      case 0x30: return "CancelGrabExclusive"   // someone took it off us
      default: return "transition" + t
    }
  }
  /// Scene coordinates plus the device: a second device delivering a point cancels the grab.
  function where(point) {
    var d = "?"
    try { d = point.device ? point.device.name : "no-device" } catch (e) { d = "unreadable" }
    return "@scene " + Math.round(point.scenePosition.x) + "," + Math.round(point.scenePosition.y)
           + " dev=" + d
  }
  function note(what) {
    if (root.t0 === 0) root.t0 = Date.now()
    if (root.events.length < 400) root.events.push([Date.now() - root.t0, what, root.dragEvents])
  }
  function debugEvents() { var e = root.events; root.events = []; root.t0 = 0; return JSON.stringify(e) }

  /// Per-frame pan trace, measured as input latency: from the first translation
  /// event that finds the accumulator empty to the frame that hands those pixels
  /// to `map.pan()`. One row per frame that had input pending:
  ///   [latency, dt, dEvents, dxIn, dyIn, dxApplied, dyApplied, active]
  property bool traceOn: false
  property var panTrace: []
  property real lastFrameAt: 0
  property real inDx: 0
  property real inDy: 0
  property int inEvents: 0
  /// When the currently-pending pixels first arrived. 0 = nothing waiting.
  property real pendingSince: 0
  function debugTrace(on) {
    root.traceOn = on
    if (on) { root.panTrace = []; root.lastFrameAt = 0; root.pendingSince = 0
              root.inDx = 0; root.inDy = 0; root.inEvents = 0 }
    return String(root.traceOn)
  }
  function debugTraceRead() {
    var t = root.panTrace
    root.panTrace = []
    if (t.length === 0) return JSON.stringify({ frames: 0, note: "no input was pending at any frame" })
    var lat = t.map(function (r) { return r[0] })
    var sorted = lat.slice().sort(function (a, b) { return a - b })
    function pct(p) { return sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * p))] }
    return JSON.stringify({
      frames: t.length,
      // One frame of latency is normal (vsync); past ~50ms it visibly lags.
      laggy: lat.filter(function (v) { return v > 50 }).length,
      stuck: lat.filter(function (v) { return v > 150 }).length,
      medianMs: pct(0.5), p95Ms: pct(0.95), maxMs: sorted[sorted.length - 1],
      worst: t.slice().sort(function (a, b) { return b[0] - a[0] }).slice(0, 10),
      sample: t.slice(0, 30)
    })
  }

  // Pending pan, in pixels, drained once per frame.
  property real panDx: 0
  property real panDy: 0
  property int recentres: 0
  property int wheelEvents: 0
  function debugInput() {
    return JSON.stringify({ interactive: root.interactive, userMoved: root.userMoved,
                            panMode: root.panMode,
                            dragEnabled: pan.enabled, dragActive: pan.active,
                            tracking: root.tracking, trackerActive: tracker.active,
                            drags: root.dragEvents, wheels: root.wheelEvents,
                            recentres: root.recentres,
                            centreLat: Math.round(map.center.latitude * 1e5) / 1e5,
                            centreLon: Math.round(map.center.longitude * 1e5) / 1e5,
                            pinLat: Math.round(root.lat * 1e5) / 1e5,
                            zoom: Math.round(map.zoomLevel * 10) / 10 })
  }

  QL.Plugin {
    id: mapPlugin
    name: "maplibre"
    QL.PluginParameter { name: "maplibre.map.styles"; value: root.styleUrl }
  }

  QL.Map {
    id: map
    anchors.fill: parent
    plugin: mapPlugin
    zoomLevel: root.zoom
    copyrightsVisible: false
    onZoomLevelChanged: if (pan.active) root.note("zoom=" + (Math.round(map.zoomLevel * 100) / 100) + " DURING-DRAG")
    onWidthChanged: root.note("map.w=" + Math.round(width))
    onHeightChanged: root.note("map.h=" + Math.round(height))
    onVisibleChanged: root.note("map.visible=" + visible)
    onEnabledChanged: root.note("map.enabled=" + enabled)

    // A child of the map, not an overlay, so it stays on its coordinate.
    QL.MapQuickItem {
      visible: root.markerVisible && root.markerAvatar === ""
      coordinate: QtPositioning.coordinate(root.lat, root.lon)
      // A pin points with its tip: bottom centre of the 48px glyph.
      anchorPoint.x: 24
      anchorPoint.y: 48
      sourceItem: IconLabel {
        id: pin
        icon: Icons.location
        filled: true
        color: root.pinColor
        size: 48
      }
    }

    // Your own position: the face sits inside the pin head.
    QL.MapQuickItem {
      visible: root.markerVisible && root.markerAvatar !== ""
      coordinate: QtPositioning.coordinate(root.lat, root.lon)
      // A pin points with its tip, so the anchor is the bottom centre.
      anchorPoint.x: 24
      anchorPoint.y: 48
      // sourceItem renders into a texture, which a ClippingRectangle's custom node does not survive.
      sourceItem: Item {
        id: selfMark
        width: 48; height: 48

        // Head of the filled `place` glyph, as fractions of the 48px box: its
        // hole centres at 0.4896 across, 0.4062 down.
        readonly property real headCx: width * 0.4896
        readonly property real headCy: height * 0.4062
        /// Waves radiate from the face, not from the pin's tip.
        readonly property real waveCy: headCy
        readonly property real headR: width * 0.26

        // Broadcast arcs, staggered so one is always in flight. Geometry is addressed
        // by id: a ShapePath is not an Item, so `parent` does not resolve from a PathAngleArc.
        Repeater {
          model: root.markerRipple ? 2 : 0
          delegate: Shape {
            id: waves
            required property int index
            anchors.fill: parent
            opacity: 0
            transformOrigin: Item.Center
            ShapePath {
              strokeColor: root.pinColor
              strokeWidth: 2
              fillColor: "transparent"
              capStyle: ShapePath.RoundCap
              PathAngleArc {
                centerX: selfMark.headCx; centerY: selfMark.waveCy
                radiusX: selfMark.headR * 1.7; radiusY: selfMark.headR * 1.7
                startAngle: -34; sweepAngle: 68
              }
            }
            ShapePath {
              strokeColor: root.pinColor
              strokeWidth: 2
              fillColor: "transparent"
              capStyle: ShapePath.RoundCap
              PathAngleArc {
                centerX: selfMark.headCx; centerY: selfMark.waveCy
                radiusX: selfMark.headR * 1.7; radiusY: selfMark.headR * 1.7
                startAngle: 146; sweepAngle: 68
              }
            }
            SequentialAnimation on opacity {
              running: root.markerRipple
              loops: Animation.Infinite
              PauseAnimation { duration: waves.index * 850 }
              NumberAnimation { from: 0.8; to: 0; duration: 1700; easing.type: Easing.OutCubic }
            }
            SequentialAnimation on scale {
              running: root.markerRipple
              loops: Animation.Infinite
              PauseAnimation { duration: waves.index * 850 }
              NumberAnimation { from: 0.8; to: 2.2; duration: 1700; easing.type: Easing.OutCubic }
            }
          }
        }

        IconLabel {
          anchors.fill: parent
          icon: Icons.location
          filled: true
          size: 48
          color: root.pinColor
          renderMode: Text.QtRendering
        }

        Item {
          id: faceMask
          x: selfMark.headCx - selfMark.headR; y: selfMark.headCy - selfMark.headR
          width: selfMark.headR * 2; height: width
          visible: false
          layer.enabled: true
          Rectangle { anchors.fill: parent; radius: width / 2; antialiasing: true; color: "black" }
        }
        Item {
          x: selfMark.headCx - selfMark.headR; y: selfMark.headCy - selfMark.headR
          width: selfMark.headR * 2; height: width
          layer.enabled: true
          layer.smooth: true
          layer.effect: MultiEffect {
            maskEnabled: true
            maskSource: faceMask
            maskThresholdMin: 0.5
            maskSpreadAtMin: 1.0
          }
          Image {
            anchors.fill: parent
            source: root.markerAvatar !== "" ? "file://" + root.markerAvatar : ""
            fillMode: Image.PreserveAspectCrop
            sourceSize.width: 76
            asynchronous: true
          }
        }
      }
    }

    TapHandler {
      enabled: root.interactive && root.tappable
      acceptedButtons: Qt.LeftButton
      onSingleTapped: function (point) {
        var c = map.toCoordinate(point.position)
        if (c.isValid) root.mapTapped(c.latitude, c.longitude)
      }
      onGrabChanged: function (transition, point) { root.note("tap-grab:" + root.grabName(transition)) }
    }

    DragHandler {
      id: pan
      enabled: root.interactive && root.panMode === "drag"
      target: null
      // Not `TakeOverForbidden`: that also blocks *acquiring* the grab from the MouseAreas below.
      grabPermissions: PointerHandler.CanTakeOverFromItems
                     | PointerHandler.CanTakeOverFromHandlersOfDifferentType
      // `translation` is measured from the press point, so the default ~10px
      // threshold arrives as a 10px first delta and the map jumps.
      dragThreshold: 0
      cursorShape: Qt.ClosedHandCursor
      // Accumulate, apply once a frame: `map.pan()` takes whole pixels, and a
      // high-rate pointer delivers sub-pixel deltas that each round to nothing.
      onTranslationChanged: function (delta) {
        root.dragEvents++
        root.userMoved = true
        root.panDx -= delta.x
        root.panDy -= delta.y
        if (root.traceOn) {
          // Stamped only when the accumulator was empty: the map owes pixels here.
          if (root.pendingSince === 0) root.pendingSince = Date.now()
          root.inDx -= delta.x; root.inDy -= delta.y; root.inEvents++
        }
        panDrive.running = true
      }
      onActiveChanged: root.note(active ? "DRAG-START" : "drag-end")
      // scenePosition, not position: `position` is relative to the item the
      // event was delivered to, which differs between grab and cancel.
      onGrabChanged: function (transition, point) {
        root.note("grab:" + root.grabName(transition) + " " + root.where(point))
      }
      onCanceled: function (point) { root.note("CANCELED " + root.where(point)) }
    }

    // Passive by construction: PointHandler never asks to own the point.
    PointHandler {
      id: tracker
      enabled: root.interactive && root.panMode === "point"
      acceptedButtons: Qt.LeftButton
      target: null
      onActiveChanged: {
        root.note(active ? "TRACK-START" : "track-end")
        if (tracker.active) {
          root.lastPx = tracker.point.position.x
          root.lastPy = tracker.point.position.y
          root.tracking = true
          root.userMoved = true
          panDrive.running = true
        } else {
          root.tracking = false
        }
      }
      onGrabChanged: function (transition, point) {
        root.note("point-grab:" + root.grabName(transition) + " " + root.where(point))
      }
      onCanceled: function (point) { root.note("POINT-CANCELED " + root.where(point)) }
    }

    PinchHandler {
      enabled: root.interactive && root.isolate !== 1 && root.isolate !== 3
      target: null
      grabPermissions: PointerHandler.TakeOverForbidden
      onScaleChanged: function (delta) {
        root.userMoved = true
        map.zoomLevel = clampZoom(map.zoomLevel + Math.log2(delta))
      }
      onActiveChanged: root.note(active ? "PINCH-START" : "pinch-end")
      onGrabChanged: function (transition, point) { root.note("pinch-grab:" + root.grabName(transition)) }
    }

    WheelHandler {
      enabled: root.interactive && root.isolate !== 2 && root.isolate !== 3
      acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
      onWheel: function (event) {
        root.wheelEvents++
        root.userMoved = true
        root.note("WHEEL dy=" + event.angleDelta.y
                  + (pan.active ? " DURING-DRAG" : "")
                  + " dev=" + (event.device ? event.device.name : "?"))
        map.zoomLevel = clampZoom(map.zoomLevel + event.angleDelta.y / 120 * 0.5)
        event.accepted = true
      }
    }
  }

  FrameAnimation {
    id: panDrive
    running: false
    onTriggered: {
      // Sample once per frame rather than once per pointer event.
      if (root.tracking && tracker.active) {
        var px = tracker.point.position.x
        var py = tracker.point.position.y
        root.panDx -= (px - root.lastPx)
        root.panDy -= (py - root.lastPy)
        root.lastPx = px
        root.lastPy = py
        if (root.traceOn && root.pendingSince === 0
            && (Math.abs(root.panDx) >= 1 || Math.abs(root.panDy) >= 1)) {
          root.pendingSince = Date.now()
        }
      }
      // Keep the sub-pixel remainder, or a slow drag loses everything under 1px.
      var ix = root.panDx >= 0 ? Math.floor(root.panDx) : Math.ceil(root.panDx)
      var iy = root.panDy >= 0 ? Math.floor(root.panDy) : Math.ceil(root.panDy)
      if (ix !== 0 || iy !== 0) {
        map.pan(ix, iy)
        root.panDx -= ix
        root.panDy -= iy
      }
      if (root.traceOn && root.pendingSince !== 0 && root.panTrace.length < 900) {
        var now = Date.now()
        var dt = root.lastFrameAt === 0 ? 0 : now - root.lastFrameAt
        root.lastFrameAt = now
        root.panTrace.push([now - root.pendingSince, dt, root.inEvents,
                            Math.round(root.inDx * 100) / 100, Math.round(root.inDy * 100) / 100,
                            ix, iy, pan.active ? 1 : 0])
        root.inDx = 0
        root.inDy = 0
        root.inEvents = 0
        // Leftovers are still owed: only a frame that drained it clears the clock.
        root.pendingSince = (ix !== 0 || iy !== 0) ? 0 : root.pendingSince
      }
      if (!pan.active && !root.tracking && Math.abs(root.panDx) < 1 && Math.abs(root.panDy) < 1) running = false
    }
  }

  function clampZoom(z) {
    return Math.max(map.minimumZoomLevel, Math.min(map.maximumZoomLevel, z))
  }

  // Centre is assigned, not bound: a binding evaluates mid-setSource while `lon` is still 0.
  /// True once the map has been dragged, pinched or zoomed. Until then the view
  /// re-snaps to the pin: a centre assigned before the item has a size lands elsewhere.
  property bool userMoved: false
  function recentre() { root.recentres++; map.center = QtPositioning.coordinate(root.lat, root.lon) }
  // All four gated on `userMoved`, or a live share yanks the view back after a pan.
  onLatChanged: if (!root.userMoved) recentre()
  onLonChanged: if (!root.userMoved) recentre()
  onWidthChanged: if (!root.userMoved) Qt.callLater(recentre)
  onHeightChanged: if (!root.userMoved) Qt.callLater(recentre)
  Component.onCompleted: Qt.callLater(recentre)

  /// Test hook: pan with no pointer involved.
  function debugPan(dx, dy) { root.userMoved = true; map.pan(dx, dy) }

  function zoomIn() { root.userMoved = true; map.zoomLevel = clampZoom(map.zoomLevel + 1) }
  function zoomOut() { root.userMoved = true; map.zoomLevel = clampZoom(map.zoomLevel - 1) }

  /// Back to the pin, for a "recentre" control.
  function resetView() { root.userMoved = false; recentre(); map.zoomLevel = root.zoom }
}
