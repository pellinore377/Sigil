import QtQuick
import Quickshell
import Quickshell.Hyprland
import qs.Commons

// A popup rendered as a real xdg-popup of the panel window.
//
// In-window popups (QtQuick.Controls Popup / Menu) composite *over* the panel content,
// and the compositor cannot blur pixels of the same surface. A separate surface gets
// Hyprland's blur (`blur_popups` on the panel's layer rule). Size with popupWidth/Height.
PopupWindow {
  id: root

  property Item anchorItem: null
  property real gap: Style.spacing.xxs
  property color tint: Color.popups.background
  property real radius: Style.cornerRadius
  property real padding: Style.space(4)
  property real popupWidth: anchorItem ? anchorItem.width : Style.space(200)
  property real popupHeight: Style.space(200)
  property bool shown: false
  // true: the panel stays in the focus grab, so typing continues there and only clicks
  // outside the panel dismiss. false: any click outside the popup dismisses (context menus).
  property bool keepPanelFocus: true
  property bool placeAtPoint: false
  property real pointX: 0
  property real pointY: 0
  default property alias content: contentHolder.data
  // Hovering moves Hyprland's keyboard focus onto this separate surface. Owners use
  // `hovered` to ignore that focus loss; keys landing here go to keyForwardTo.
  readonly property bool hovered: hover.hovered
  property list<Item> keyForwardTo

  signal dismissed()

  readonly property var anchorWindow: anchorItem ? anchorItem.QsWindow.window : null

  function open() { placeAtPoint = false; shown = true }
  function openAt(sceneX, sceneY) { placeAtPoint = true; pointX = sceneX; pointY = sceneY; shown = true }
  function close() { shown = false }

  visible: shown && anchorWindow !== null
  color: "transparent"
  implicitWidth: Math.max(1, Math.round(popupWidth))
  implicitHeight: Math.max(1, Math.round(popupHeight))

  HyprlandFocusGrab {
    active: root.shown
    windows: root.keepPanelFocus && root.anchorWindow ? [root, root.anchorWindow] : [root]
    onCleared: if (root.shown) { root.shown = false; root.dismissed() }
  }

  anchor {
    id: panchor
    window: root.anchorWindow
    // Anchor rect = the trigger (plus gap); the popup hangs below its bottom edge,
    // flipping above when there is no room and sliding sideways to stay on screen.
    edges: Edges.Bottom | Edges.Left
    gravity: Edges.Bottom | Edges.Right
    adjustment: PopupAdjustment.SlideX | PopupAdjustment.FlipY
    onAnchoring: root.place()
  }

  function place() {
    var win = root.anchorWindow
    if (!win) return
    if (root.placeAtPoint) {
      panchor.rect.x = Math.round(root.pointX)
      panchor.rect.y = Math.round(root.pointY)
      panchor.rect.width = 1
      panchor.rect.height = 1
      return
    }
    if (!root.anchorItem) return
    // Scene coordinates == window coordinates; Omarchy's panels override the
    // window's contentItem, so don't map through it.
    var p = root.anchorItem.mapToItem(null, 0, 0)
    panchor.rect.x = Math.round(p.x)
    panchor.rect.y = Math.round(p.y - root.gap)
    panchor.rect.width = Math.max(1, Math.round(root.anchorItem.width))
    panchor.rect.height = Math.max(1, Math.round(root.anchorItem.height + 2 * root.gap))
  }

  Rectangle {
    anchors.fill: parent
    radius: root.radius
    color: root.tint
    opacity: root.shown ? 1 : 0
    Behavior on opacity { NumberAnimation { duration: 120; easing.type: Easing.OutCubic } }

    HoverHandler { id: hover }

    Item {
      id: keySink
      anchors.fill: parent
      focus: true
      Keys.forwardTo: root.keyForwardTo
    }

    Item {
      id: contentHolder
      anchors.fill: parent
      anchors.margins: root.padding
    }
  }
}
