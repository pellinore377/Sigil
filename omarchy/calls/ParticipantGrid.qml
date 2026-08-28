import QtQuick
import qs.Commons
import "../components"

// N-tile grid (cols = ceil(sqrt n)); a screen-share tile takes a spotlight.
Item {
  id: root
  property var tiles: []   // [{participant, track, isLocal}]
  property color fg: Color.menu.text
  readonly property int gap: Style.space(8)
  readonly property var spotlight: { for (var i = 0; i < tiles.length; i++) if (tiles[i].track && tiles[i].track.kind === "screen") return i; return -1 }
  readonly property var gridTiles: { var out = []; for (var i = 0; i < tiles.length; i++) if (i !== spotlight) out.push(tiles[i]); return out }
  readonly property int n: gridTiles.length
  readonly property int cols: n <= 1 ? 1 : Math.ceil(Math.sqrt(n))
  readonly property int rows: n <= 1 ? 1 : Math.ceil(n / cols)
  readonly property real gridW: spotlight >= 0 ? Math.round(width * 0.25) : width
  readonly property real gridX: spotlight >= 0 ? width - gridW : 0
  readonly property int gcols: spotlight >= 0 ? 1 : cols
  readonly property int grows: spotlight >= 0 ? Math.max(1, n) : rows
  readonly property real cellW: (gridW - gap * (gcols + 1)) / gcols
  readonly property real cellH: (height - gap * (grows + 1)) / grows
  readonly property real tileW: Math.min(cellW, cellH * 16 / 9)
  readonly property real tileH: tileW * 9 / 16

  ParticipantTile {
    visible: root.spotlight >= 0
    x: root.gap; y: root.gap; width: root.spotlight >= 0 ? root.width - root.gridW - root.gap * 2 : 0; height: root.height - root.gap * 2
    participant: root.spotlight >= 0 ? root.tiles[root.spotlight].participant : null
    track: root.spotlight >= 0 ? root.tiles[root.spotlight].track : null
    isLocal: root.spotlight >= 0 ? root.tiles[root.spotlight].isLocal : false
    fitVideo: true
    fg: root.fg
  }
  Repeater {
    model: root.gridTiles
    delegate: ParticipantTile {
      required property var modelData
      required property int index
      readonly property int c: index % root.gcols
      readonly property int r: Math.floor(index / root.gcols)
      readonly property real rowCount: Math.min(root.gcols, root.n - r * root.gcols)
      readonly property real rowW: rowCount * root.tileW + (rowCount - 1) * root.gap
      x: root.gridX + (root.gridW - rowW) / 2 + c * (root.tileW + root.gap)
      y: root.gap + (root.height - root.gap * 2 - (root.grows * root.tileH + (root.grows - 1) * root.gap)) / 2 + r * (root.tileH + root.gap)
      width: root.tileW; height: root.tileH
      participant: modelData.participant; track: modelData.track; isLocal: modelData.isLocal
      fg: root.fg
    }
  }
  Text { anchors.centerIn: parent; visible: root.tiles.length === 0; text: "Waiting for others to join…"; color: Util.alpha(root.fg, 0.5); font.family: Fonts.ui; font.pixelSize: Style.font.body }
}
