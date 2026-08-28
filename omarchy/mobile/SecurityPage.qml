import QtQuick
import qs.Commons
import qs.Ui
import "../components"

// Security & privacy: who can join, whether messages are encrypted, and how
// much history a new member sees. Writes are deferred to Save rather than
// applied per tap — each is a state event others see land, and undo cannot
// take back a room opened to the world.
Item {
  id: root
  property var svc: null
  property string roomId: ""
  property color fg: Color.menu.text

  signal closed()

  property var settings: ({})
  property bool busy: false
  property string error: ""

  // Pending edits, empty until something is touched.
  property string joinRule: ""
  property string history: ""
  property bool wantEncrypted: false

  readonly property var can: root.settings.can || ({})
  readonly property bool isEncrypted: !!root.settings.isEncrypted
  /// Restricted join ("space members can join") needs a parent space to name.
  readonly property var parentSpaces: (root.svc && root.roomId) ? root.svc.spacesContaining(root.roomId) : []
  readonly property var parentSpace: root.parentSpaces.length > 0 ? root.parentSpaces[0] : null

  readonly property bool dirty: (root.joinRule !== "" && root.joinRule !== root.settings.joinRule)
                             || (root.history !== "" && root.history !== root.settings.historyVisibility)
                             || (root.wantEncrypted && !root.isEncrypted)

  function reset() { root.error = ""; root.joinRule = ""; root.history = ""; root.wantEncrypted = false; root.load() }
  function load() {
    if (!root.svc || !root.roomId) return
    root.svc.roomSettings(root.roomId, function (r, e) {
      if (!r) { root.error = "Could not read settings"; return }
      root.settings = r
      root.joinRule = r.joinRule || "invite"
      root.history = r.historyVisibility || "shared"
      root.wantEncrypted = !!r.isEncrypted
    })
  }
  onRoomIdChanged: root.load()

  function save() {
    if (!root.dirty || root.busy || !root.svc) return
    root.busy = true
    var fields = {}
    if (root.joinRule !== root.settings.joinRule) {
      fields.joinRule = root.joinRule
      if (root.joinRule === "restricted" && root.parentSpace) fields.restrictedTo = root.parentSpace.id
    }
    if (root.history !== root.settings.historyVisibility) fields.historyVisibility = root.history
    if (root.wantEncrypted && !root.isEncrypted) fields.encrypted = true
    root.svc.setRoomSettings(root.roomId, fields, function (r, e) {
      root.busy = false
      if (e) { root.error = (e.message || "Could not save"); return }
      root.error = ""
      root.load()
    })
  }

  Rectangle { anchors.fill: parent; color: Qt.lighter(Color.menu.background, 1.35) }

  Flickable {
    anchors.fill: parent
    contentWidth: width
    contentHeight: col.implicitHeight + Style.space(24)
    clip: true
    boundsBehavior: Flickable.StopAtBounds

    Column {
      id: col
      width: parent.width
      spacing: 0

      SettingsHeader {
        fg: root.fg
        title: "Security & privacy"
        action: root.busy ? "Saving…" : "Save"
        actionEnabled: root.dirty && !root.busy
        onBack: root.closed()
        onActioned: root.save()
      }

      SettingsGroup {
        fg: root.fg
        title: "Access"
        divided: false
        SettingsRow {
          fg: root.fg
          icon: Icons.globe; label: "Anyone"; sublabel: "Anyone can join."
          trailing: "radio"; on: root.joinRule === "public"
          enabled: !!root.can.setJoinRule
          onClicked: root.joinRule = "public"
        }
        SettingsRow {
          fg: root.fg
          // Only offered when the room sits in a space; the server rejects it otherwise.
          visible: !!root.parentSpace
          icon: Icons.space
          label: "Space members"
          sublabel: root.parentSpace ? ("Anyone in " + (root.parentSpace.name || "the space") + " can join.") : ""
          trailing: "radio"; on: root.joinRule === "restricted"
          enabled: !!root.can.setJoinRule
          onClicked: root.joinRule = "restricted"
        }
        SettingsRow {
          fg: root.fg
          icon: Icons.lock; label: "Invite only"; sublabel: "Only invited people can join."
          trailing: "radio"; on: root.joinRule === "invite"
          enabled: !!root.can.setJoinRule
          onClicked: root.joinRule = "invite"
        }
      }

      SettingsGroup {
        fg: root.fg
        title: "Encryption"
        // A space carries no messages, so encrypting one only hides the tree. Not offered.
        visible: !root.settings.isSpace
        SettingsRow {
          fg: root.fg
          label: "Enable end-to-end encryption"
          sublabel: "Once enabled, encryption cannot be disabled."
          trailing: "toggle"
          on: root.wantEncrypted
          // One-way in the spec: it cannot be turned back off, so the control locks.
          enabled: !root.isEncrypted && !!root.can.setEncryption
          onClicked: root.wantEncrypted = !root.wantEncrypted
        }
      }

      SettingsGroup {
        fg: root.fg
        title: "Who can read history"
        Text {
          x: Style.space(22)
          width: parent.width - Style.space(44)
          wrapMode: Text.Wrap
          text: "Changes won't affect past messages, only new ones."
          color: Util.alpha(root.fg, 0.55)
          font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
          bottomPadding: Style.space(8)
        }
        SettingsRow {
          fg: root.fg
          // Only meaningful when anyone can join.
          visible: root.joinRule === "public"
          label: "Anyone"; sublabel: "Including people who never joined."
          trailing: "radio"; on: root.history === "world_readable"
          enabled: !!root.can.setHistoryVisibility
          onClicked: root.history = "world_readable"
        }
        SettingsRow {
          fg: root.fg
          label: "Members since they were invited"
          trailing: "radio"; on: root.history === "invited"
          enabled: !!root.can.setHistoryVisibility
          onClicked: root.history = "invited"
        }
        SettingsRow {
          fg: root.fg
          label: "Members since they joined"
          trailing: "radio"; on: root.history === "joined"
          enabled: !!root.can.setHistoryVisibility
          onClicked: root.history = "joined"
        }
        SettingsRow {
          fg: root.fg
          label: "Members (full history)"
          trailing: "radio"; on: root.history === "shared"
          enabled: !!root.can.setHistoryVisibility
          onClicked: root.history = "shared"
        }
      }

      Item { width: parent.width; height: Style.space(10) }
      Text {
        x: Style.space(22)
        width: parent.width - Style.space(44)
        visible: root.error !== "" || !root.can.setJoinRule
        text: root.error !== "" ? root.error : "You do not have permission to change these."
        color: root.error !== "" ? Color.urgent : Util.alpha(root.fg, 0.5)
        wrapMode: Text.Wrap
        font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
      }
    }
  }
}
