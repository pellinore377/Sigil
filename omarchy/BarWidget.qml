import QtQuick
import qs.Commons
import qs.Ui
import "components"

// Bar icon: toggles the panel over the same IPC route a keybinding uses, and
// badges highlights (count), unread (dot) and an active call (pulsing dot).
BarWidget {
  id: root
  moduleName: "pellinore.sigil"

  readonly property string pluginId: "pellinore.sigil"
  readonly property var service: (bar && bar.shell && typeof bar.shell.serviceFor === "function")
                                 ? bar.shell.serviceFor(pluginId) : null
  readonly property bool engineUp: service ? (service.connected && service.authState === "loggedIn") : false
  readonly property int unread: service ? service.unreadTotal : 0
  readonly property int highlights: service ? service.highlightTotal : 0
  readonly property bool inCall: service ? service.inCall : false
  readonly property bool ringing: service ? !!(service.call && service.call.incoming) : false
  readonly property bool showCount: root.setting("showCount", true) !== false

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  WidgetButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    // WidgetButton draws `text` with the bar's own font, which has no Material
    // Symbols glyphs; it exposes `fontFamily` for exactly this.
    fontFamily: Fonts.iconFilled
    text: Icons.chat
    tooltipText: {
      if (!root.service || !root.service.connected) return "Sigil — engine not running"
      if (root.service.authState !== "loggedIn") return "Sigil — not signed in"
      var bits = []
      if (root.highlights > 0) bits.push(root.highlights + " mention" + (root.highlights === 1 ? "" : "s"))
      if (root.unread > 0) bits.push(root.unread + " unread")
      if (root.inCall) bits.push("in a call")
      if (root.ringing) bits.push("incoming call")
      return bits.length ? "Sigil — " + bits.join(", ") : "Sigil"
    }
    foreground: root.engineUp ? (root.bar ? root.bar.barForeground : Color.foreground) : Color.muted
    fixedWidth: root.bar && root.bar.vertical ? -1 : Style.space(27)
    fixedHeight: root.bar && root.bar.vertical ? Style.space(26) : -1
    onPressed: function(b) {
      if (!root.bar) return
      if (b === Qt.MiddleButton) { if (root.service) root.service.markAllRead(); return }
      root.bar.run("omarchy-shell shell toggle pellinore.sigil")
    }
  }

  // Highlight count pill / unread dot.
  Rectangle {
    id: badge
    visible: root.highlights > 0 || root.unread > 0
    readonly property bool pill: root.highlights > 0 && root.showCount
    width: pill ? Math.max(Style.space(12), countText.implicitWidth + Style.space(5)) : Style.space(7)
    height: pill ? Style.space(11) : Style.space(7)
    radius: height / 2
    color: root.highlights > 0 ? Color.urgent : Util.alpha(root.bar ? root.bar.barForeground : Color.foreground, 0.55)
    anchors.top: parent.top
    anchors.right: parent.right
    anchors.topMargin: Style.space(3)
    anchors.rightMargin: Style.space(2)
    Text {
      id: countText
      anchors.centerIn: parent
      visible: badge.pill
      text: root.highlights > 99 ? "99+" : String(root.highlights)
      font.family: Fonts.ui
      font.pixelSize: Style.space(8)
      font.bold: true
      color: Color.background
    }
  }

  // Active call: pulsing accent dot bottom-right.
  Rectangle {
    visible: root.inCall || root.ringing
    width: Style.space(7); height: width; radius: width / 2
    color: root.ringing ? Color.urgent : Color.accent
    anchors.bottom: parent.bottom
    anchors.right: parent.right
    anchors.bottomMargin: Style.space(3)
    anchors.rightMargin: Style.space(2)
    SequentialAnimation on opacity {
      running: root.inCall || root.ringing
      loops: Animation.Infinite
      NumberAnimation { to: 0.25; duration: 600 }
      NumberAnimation { to: 1.0; duration: 600 }
    }
  }
}
