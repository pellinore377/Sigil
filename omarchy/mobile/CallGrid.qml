import QtQuick
import qs.Commons
import qs.Ui
import "../calls"
import "../components"

// Group-call layout for the narrow panel: portrait cells, share on top.
// calls/ParticipantGrid.qml is the wide-window equivalent.
Item {
  id: root
  /// `[{participant, track, isLocal}]`, self included.
  property var tiles: []
  property color fg: Color.menu.text
  property color accent: Color.accent

  readonly property real gap: Style.space(8)

  readonly property int shareIdx: {
    for (var i = 0; i < root.tiles.length; i++) {
      var t = root.tiles[i].track
      if (t && t.kind === "screen") return i
    }
    return -1
  }
  /// A ListModel, not a `var` array: call state is replaced wholesale on every
  /// speaker update, and an array model cannot diff, so tiles were rebuilt.
  ListModel { id: peopleModel; dynamicRoles: true }
  readonly property int peopleCount: peopleModel.count

  function syncPeople() {
    var want = []
    for (var i = 0; i < root.tiles.length; i++) if (i !== root.shareIdx) want.push(root.tiles[i])

    // Drop rows that are gone, back to front so indices stay valid.
    for (var r = peopleModel.count - 1; r >= 0; r--) {
      var pid = peopleModel.get(r).pid
      var still = false
      for (var w = 0; w < want.length; w++) if (root.pidOf(want[w]) === pid) { still = true; break }
      if (!still) peopleModel.remove(r)
    }
    for (var k = 0; k < want.length; k++) {
      var t = want[k]
      var id = root.pidOf(t)
      var at = -1
      for (var j = 0; j < peopleModel.count; j++) if (peopleModel.get(j).pid === id) { at = j; break }
      if (at < 0) {
        peopleModel.insert(Math.min(k, peopleModel.count), { pid: id, pPart: t.participant, pTrack: t.track || null, pLocal: !!t.isLocal })
      } else {
        if (at !== k) peopleModel.move(at, k, 1)
        peopleModel.setProperty(k, "pPart", t.participant)
        peopleModel.setProperty(k, "pTrack", t.track || null)
        peopleModel.setProperty(k, "pLocal", !!t.isLocal)
      }
    }
  }
  function pidOf(t) {
    if (t.isLocal) return "@local"
    return (t.participant && (t.participant.participantId || t.participant.userId)) || "?"
  }
  onTilesChanged: root.syncPeople()
  Component.onCompleted: root.syncPeople()
  readonly property bool sharing: root.shareIdx >= 0

  /// Two columns; three across leaves faces too small to read at this width.
  readonly property int cols: root.peopleCount <= 2 ? 1 : 2
  readonly property int rows: Math.max(1, Math.ceil(root.peopleCount / root.cols))

  // With a share on top, people become one scrolling row; none if alone.
  readonly property real stripH: (root.sharing && root.peopleCount > 0) ? Style.space(96) : 0
  readonly property real shareH: root.sharing ? root.height - root.stripH - root.gap : 0

  // Spotlight
  ParticipantTile {
    visible: root.sharing
    x: root.gap; y: root.gap
    width: root.width - root.gap * 2
    height: Math.max(0, root.shareH - root.gap)
    tileRadius: Style.space(20)
    // Never crop a shared screen; the content is the whole point.
    fitVideo: true
    participant: root.sharing ? root.tiles[root.shareIdx].participant : null
    track: root.sharing ? root.tiles[root.shareIdx].track : null
    isLocal: root.sharing ? !!root.tiles[root.shareIdx].isLocal : false
    fg: root.fg; accent: root.accent
  }

  // Thumb strip
  Flickable {
    visible: root.sharing
    y: root.shareH + root.gap
    x: 0
    width: root.width
    height: Math.max(0, root.stripH - root.gap)
    contentWidth: strip.width
    flickableDirection: Flickable.HorizontalFlick
    boundsBehavior: Flickable.StopAtBounds
    clip: true
    Row {
      id: strip
      height: parent.height
      spacing: root.gap
      leftPadding: root.gap
      rightPadding: root.gap
      // Centred with `x`, not padding: deriving leftPadding from implicitWidth
      // is a binding loop. `x` is not part of the Row's own size.
      x: Math.max(0, (root.width - strip.width) / 2)
      Repeater {
        // Guarded: unguarded, this builds a full tile per person while hidden.
        model: root.sharing ? peopleModel : null
        delegate: ParticipantTile {
          // Prefixed roles: an item's own properties shadow model roles.
          required property var pPart
          required property var pTrack
          required property bool pLocal
          // Wide enough for a name plus the mute glyph at this text size.
          width: Style.space(96); height: parent.height
          participant: pPart; track: pTrack; isLocal: pLocal
          fg: root.fg; accent: root.accent
        }
      }
    }
  }

  // Grid
  Repeater {
    model: root.sharing ? null : peopleModel
    delegate: ParticipantTile {
      required property var pPart
      required property var pTrack
      required property bool pLocal
      required property int index
      readonly property int c: index % root.cols
      readonly property int r: Math.floor(index / root.cols)
      // Last row centres when short of a full row. `r`/`c` are the delegate's
      // own row and column; root's yield NaN here.
      readonly property int inRow: Math.min(root.cols, root.peopleCount - r * root.cols)
      readonly property real cellW: Math.max(0, (root.width - root.gap * (root.cols + 1)) / root.cols)
      // Clamped: the tile crops rather than fits; a tall cell would cut a 16:9 frame.
      readonly property real cellH: Math.max(0, Math.min((root.height - root.gap * (root.rows + 1)) / root.rows,
                                                         cellW * 1.4))
      readonly property real rowW: inRow * cellW + (inRow - 1) * root.gap
      x: (root.width - rowW) / 2 + c * (cellW + root.gap)
      // Block centred vertically once cells are clamped.
      y: root.gap + Math.max(0, (root.height - root.gap * (root.rows + 1) - root.rows * cellH) / 2)
         + r * (cellH + root.gap)
      width: cellW; height: cellH
      tileRadius: Style.space(16)
      participant: pPart; track: pTrack; isLocal: pLocal
      fg: root.fg; accent: root.accent
    }
  }

  Text {
    anchors.centerIn: parent
    // `tiles` always contains you, so the empty case is "nobody else".
    visible: !root.sharing && root.peopleCount === 0
    text: "Waiting for others to join…"
    color: Util.alpha(root.fg, 0.5)
    font.family: Fonts.ui; font.pixelSize: Style.font.body
  }
}
