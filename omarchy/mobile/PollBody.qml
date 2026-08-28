import QtQuick
import qs.Commons
import qs.Ui
import "../components"

// An MSC3381 poll in a bubble: question, one row per answer with a fill for
// its share of the vote, and a tally footer.
//
// An *undisclosed* poll withholds responses until it ends, so the tally is
// genuinely empty rather than zero and rows say "hidden", not 0%. A response
// replaces the sender's previous one; an empty answer list retracts.
Item {
  id: root
  property var poll: null
  property color fg: Color.menu.text
  property color accent: Color.accent
  property color surface: Util.alpha(Color.background, 0.55)
  property bool own: false
  /// False once the poll has ended, or while the send is still in flight.
  property bool interactive: true
  signal voteRequested(var answerIds)

  readonly property var answers: (poll && poll.answers) ? poll.answers : []
  readonly property bool ended: !!(poll && poll.ended)
  readonly property bool disclosed: !!(poll && poll.disclosed)
  readonly property int voters: poll ? (poll.voters || 0) : 0
  readonly property int maxSelections: poll ? Math.max(1, poll.maxSelections || 1) : 1
  // Counts only exist once the server has sent them.
  readonly property bool showCounts: root.ended || root.disclosed
  readonly property bool canVote: root.interactive && !root.ended

  readonly property int totalVotes: {
    var n = 0
    for (var i = 0; i < root.answers.length; i++) n += (root.answers[i].votes || 0)
    return n
  }
  readonly property int leaderVotes: {
    var n = 0
    for (var i = 0; i < root.answers.length; i++) n = Math.max(n, root.answers[i].votes || 0)
    return n
  }
  function shareOf(a) {
    if (!root.showCounts || root.totalVotes <= 0) return 0
    return (a.votes || 0) / root.totalVotes
  }
  function mineIds() {
    var out = []
    for (var i = 0; i < root.answers.length; i++) if (root.answers[i].mine) out.push(root.answers[i].id)
    return out
  }
  function pick(a) {
    if (!root.canVote) return
    if (root.maxSelections <= 1) {
      // Tapping your own answer again takes the vote back.
      root.voteRequested(a.mine ? [] : [a.id])
      return
    }
    var sel = root.mineIds()
    var at = sel.indexOf(a.id)
    if (at >= 0) sel.splice(at, 1)
    else if (sel.length < root.maxSelections) sel.push(a.id)
    else return
    root.voteRequested(sel)
  }

  implicitWidth: Style.space(252)
  implicitHeight: col.implicitHeight + Style.space(24)

  Rectangle {
    anchors.fill: parent
    radius: Style.space(16)
    antialiasing: true
    color: root.surface
  }

  Column {
    id: col
    anchors.left: parent.left; anchors.right: parent.right
    anchors.leftMargin: Style.space(12); anchors.rightMargin: Style.space(12)
    anchors.verticalCenter: parent.verticalCenter
    spacing: Style.space(7)

    Text {
      width: parent.width
      text: (root.poll && root.poll.question) ? root.poll.question : ""
      color: root.fg
      font.family: Fonts.ui
      font.pixelSize: Style.font.body
      font.bold: true
      wrapMode: Text.Wrap
    }

    Repeater {
      model: root.answers
      delegate: Item {
        id: answerRow
        required property var modelData
        width: col.width
        height: Style.space(34)

        readonly property real share: root.shareOf(modelData)
        readonly property bool leading: root.showCounts && root.leaderVotes > 0
                                        && (modelData.votes || 0) === root.leaderVotes

        Rectangle {
          anchors.fill: parent
          radius: Style.space(11)
          antialiasing: true
          color: Util.alpha("#000000", ah.containsMouse && root.canVote ? 0.30 : 0.22)

          Rectangle {
            anchors.left: parent.left
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            width: parent.width * Math.max(0, Math.min(1, answerRow.share))
            radius: parent.radius
            antialiasing: true
            color: Util.alpha(root.accent, answerRow.leading ? 0.55 : 0.28)
            Behavior on width { NumberAnimation { duration: 320; easing.type: Easing.OutCubic } }
          }
        }

        // Your pick, as a filled dot; empty ring while the poll is open.
        Rectangle {
          id: pickDot
          anchors.left: parent.left
          anchors.leftMargin: Style.space(10)
          anchors.verticalCenter: parent.verticalCenter
          width: Style.space(15); height: width; radius: width / 2
          antialiasing: true
          visible: root.canVote || modelData.mine
          color: modelData.mine ? root.accent : "transparent"
          border.width: modelData.mine ? 0 : Math.max(1, Style.space(1.5))
          border.color: Util.alpha(root.fg, 0.4)
          Text {
            anchors.centerIn: parent
            visible: answerRow.modelData.mine
            text: Icons.check
            color: Color.background
            font.family: Fonts.icon; renderType: Text.NativeRendering
            font.pixelSize: Style.space(9)
          }
        }

        Text {
          anchors.left: pickDot.visible ? pickDot.right : parent.left
          anchors.leftMargin: Style.space(pickDot.visible ? 8 : 12)
          anchors.right: tally.left
          anchors.rightMargin: Style.space(8)
          anchors.verticalCenter: parent.verticalCenter
          text: answerRow.modelData.text
          color: root.fg
          font.family: Fonts.ui
          font.pixelSize: Style.font.bodySmall
          elide: Text.ElideRight
        }

        Text {
          id: tally
          anchors.right: parent.right
          anchors.rightMargin: Style.space(11)
          anchors.verticalCenter: parent.verticalCenter
          visible: root.showCounts
          text: {
            var v = answerRow.modelData.votes || 0
            if (root.totalVotes <= 0) return "0"
            return v + "  " + Math.round(answerRow.share * 100) + "%"
          }
          color: Util.alpha(root.fg, 0.7)
          font.family: Fonts.ui
          font.pixelSize: Style.space(10)
          font.bold: answerRow.leading
        }

        MouseArea {
          id: ah
          anchors.fill: parent
          hoverEnabled: true
          enabled: root.canVote
          cursorShape: root.canVote ? Qt.PointingHandCursor : Qt.ArrowCursor
          onClicked: root.pick(answerRow.modelData)
        }
      }
    }

    Text {
      width: parent.width
      text: {
        var who = root.voters === 1 ? "1 vote" : root.voters + " votes"
        if (root.ended) return "Final results · " + who
        if (!root.disclosed) return "Results hidden until the poll ends · " + who
        if (root.maxSelections > 1) return "Pick up to " + root.maxSelections + " · " + who
        return who
      }
      color: Util.alpha(root.fg, 0.5)
      font.family: Fonts.ui
      font.pixelSize: Style.space(10)
      wrapMode: Text.Wrap
    }
  }
}
