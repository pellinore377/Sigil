import QtQuick
import qs.Commons
import "components"

// Themed replacement for Qt's stock text-editing context menu (Qt 6.9+ attaches a
// Basic-style Cut/Copy/Paste menu to every TextField and TextArea). As a child of
// the editor: `ContextMenu.menu: null` to suppress Qt's, then `TextContextMenu {
// editor: parent }`. Built on FrostPopup, so Hyprland frosts it like the panel.
Item {
  id: root

  property Item editor: parent
  property color foreground: Color.menu.text
  property color accent: Color.accent

  readonly property bool editable: editor !== null && !editor.readOnly
  readonly property bool hasSelection: editor !== null && String(editor.selectedText || "").length > 0
  readonly property bool hasText: editor !== null && String(editor.text || "").length > 0
  readonly property var rows: [
    { text: "Cut", icon: Icons.cut, shortcut: "Ctrl+X", enabled: root.editable && root.hasSelection, action: "cut" },
    { text: "Copy", icon: Icons.copy, shortcut: "Ctrl+C", enabled: root.hasSelection, action: "copy" },
    { text: "Paste", icon: Icons.paste, shortcut: "Ctrl+V", enabled: root.editable && editor !== null && editor.canPaste, action: "paste" },
    { text: "Delete", icon: Icons.trash, shortcut: "", enabled: root.editable && root.hasSelection, action: "delete" },
    { separator: true },
    { text: "Select all", icon: Icons.selectAll, shortcut: "Ctrl+A", enabled: root.hasText, action: "selectAll" }
  ]

  function run(action) {
    var e = root.editor
    if (!e) return
    if (action === "cut") e.cut()
    else if (action === "copy") e.copy()
    else if (action === "paste") e.paste()
    else if (action === "delete") e.remove(e.selectionStart, e.selectionEnd)
    else if (action === "selectAll") e.selectAll()
  }

  // Fills the editor; only right-clicks are taken, the rest passes through.
  anchors.fill: parent

  // Host the menu in the window's own content item: an xdg-popup escaped the app and dismissed on hover-out.
  function findHost() {
    // Prefer the panel card, so the menu is clipped to the app.
    var it = root.parent
    while (it) {
      if (it.objectName === "sigilCard") return it
      it = it.parent
    }
    return root.Window.contentItem
  }
  property Item menuItem: null

  TapHandler {
    acceptedButtons: Qt.RightButton
    gesturePolicy: TapHandler.ReleaseWithinBounds
    onTapped: function(point) {
      if (root.editor) root.editor.forceActiveFocus()
      root.openMenu(point.scenePosition.x, point.scenePosition.y)
    }
  }

  function closeMenu() {
    if (root.menuItem) { root.menuItem.destroy(); root.menuItem = null }
  }

  function openMenu(sx, sy) {
    root.closeMenu()
    var host = root.findHost()
    if (!host) return
    // Scene coords are window-relative; map into the host or the menu lands far from the pointer.
    var local = host.mapFromItem(null, sx, sy)
    root.menuItem = menuComponent.createObject(host, { "ctx": root, "sceneX": local.x, "sceneY": local.y })
  }

  Component.onDestruction: root.closeMenu()

  // The panel closing (or focus moving on) takes the menu with it.
  Connections {
    target: root.editor
    function onActiveFocusChanged() { if (!root.editor.activeFocus) root.closeMenu() }
    function onVisibleChanged() { if (!root.editor.visible) root.closeMenu() }
  }

  Component {
    id: menuComponent
    Item {
      id: menuRoot
      property var ctx: null           // the TextContextMenu; outer ids do not
                                       // resolve inside a dynamically created object
      property real sceneX: 0
      property real sceneY: 0
      readonly property real padding: Style.space(4)
      property bool shown: false
      anchors.fill: parent
      z: 9999
      Component.onCompleted: menuRoot.shown = true

      // click-away
      MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; onPressed: menuRoot.ctx.closeMenu() }

      Rectangle {
        id: menu
        readonly property real popupWidth: Style.space(196)
        readonly property real popupHeight: column.implicitHeight + menuRoot.padding * 2
        width: popupWidth
        height: popupHeight
        // Below the pointer when there is room, else above: clamping a bottom-edge click parked it far away.
        readonly property bool up: menuRoot.sceneY + height + Style.space(8) > menuRoot.height
        x: Math.max(Style.space(6), Math.min(menuRoot.sceneX, menuRoot.width - width - Style.space(6)))
        y: up ? Math.max(Style.space(6), menuRoot.sceneY - height)
              : Math.min(menuRoot.sceneY, menuRoot.height - height - Style.space(6))
        transformOrigin: up ? Item.BottomLeft : Item.TopLeft
        scale: menuRoot.shown ? 1 : 0.8
        opacity: menuRoot.shown ? 1 : 0
        Behavior on scale { NumberAnimation { duration: 130; easing.type: Easing.OutCubic } }
        Behavior on opacity { NumberAnimation { duration: 110 } }
        radius: Style.space(12)
        antialiasing: true
        color: Util.alpha(Qt.lighter(Color.menu.background, 1.35), 0.99)
        border.width: 1
        border.color: Util.alpha(menuRoot.ctx.foreground, 0.12)
        MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons }

    Column {
      id: column
      x: menuRoot.padding
      y: menuRoot.padding
      width: parent.width - menuRoot.padding * 2

      Repeater {
        model: menuRoot.ctx.rows

        delegate: Item {
          id: row
          required property var modelData
          readonly property bool separator: modelData.separator === true
          readonly property bool rowEnabled: !separator && modelData.enabled === true
          width: column.width
          height: separator ? Style.space(7) : Style.spacing.popupRowHeight

          Rectangle {
            visible: row.separator
            anchors.verticalCenter: parent.verticalCenter
            width: parent.width
            height: 1
            color: Util.alpha(menuRoot.ctx.foreground, 0.12)
          }

          Rectangle {
            visible: !row.separator
            anchors.fill: parent
            radius: Math.max(0, Style.cornerRadius - 3)
            color: hover.hovered && row.rowEnabled ? Style.hoverFillFor(menuRoot.ctx.foreground, menuRoot.ctx.accent) : "transparent"
          }

          Text {
            visible: !row.separator
            anchors.left: parent.left
            anchors.leftMargin: Style.spacing.controlPaddingX
            anchors.verticalCenter: parent.verticalCenter
            text: row.separator ? "" : String(row.modelData.icon || "")
            color: hover.hovered && row.rowEnabled ? Style.hoverStateColor(menuRoot.ctx.foreground, menuRoot.ctx.accent) : menuRoot.ctx.foreground
            opacity: row.rowEnabled ? 0.85 : 0.35
            font.family: Fonts.icon; renderType: Text.NativeRendering
            font.pixelSize: Style.font.icon
          }

          Text {
            visible: !row.separator
            anchors.left: parent.left
            anchors.leftMargin: Style.spacing.controlPaddingX + Style.space(26)
            anchors.verticalCenter: parent.verticalCenter
            text: row.separator ? "" : String(row.modelData.text)
            color: hover.hovered && row.rowEnabled ? Style.hoverStateColor(menuRoot.ctx.foreground, menuRoot.ctx.accent) : menuRoot.ctx.foreground
            opacity: row.rowEnabled ? 1 : 0.4
            font.family: Fonts.ui
            font.pixelSize: Style.font.body
          }

          Text {
            visible: !row.separator
            anchors.right: parent.right
            anchors.rightMargin: Style.spacing.controlPaddingX
            anchors.verticalCenter: parent.verticalCenter
            text: row.separator ? "" : String(row.modelData.shortcut || "")
            color: Util.alpha(menuRoot.ctx.foreground, 0.5)
            opacity: row.rowEnabled ? 1 : 0.4
            font.family: Fonts.ui
            font.pixelSize: Style.font.caption
          }

          HoverHandler { id: hover; enabled: row.rowEnabled; cursorShape: Qt.PointingHandCursor }

          TapHandler {
            enabled: row.rowEnabled
            onTapped: {
              menuRoot.ctx.closeMenu()
              menuRoot.ctx.run(row.modelData.action)
              if (menuRoot.ctx.editor) menuRoot.ctx.editor.forceActiveFocus()
            }
          }
        }
      }
    }
      }
    }
  }
}
