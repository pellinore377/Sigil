import QtQuick
import qs.Commons
import qs.Ui
import "../components"

// A shared contact, as a card. Two sources: a Matrix contact in the message's
// `com.sigil.contact` field, and a `.vcf` file (possibly several entries). The
// card takes a list and lets each entry answer for itself.
Item {
  id: root
  /// `[{name, phones:[{kind,value}], emails:[…], org, title, address, note,
  /// matrixId, hasPhoto, photoPath, avatarPath, userId}]`
  property var cards: []
  property color fg: Color.menu.text
  property color accent: Color.accent
  property color surface: Util.alpha(Color.menu.text, 0.10)
  property real topLeftRadius: Style.space(16)
  property real topRightRadius: Style.space(16)
  property real bottomLeftRadius: Style.space(16)
  property real bottomRightRadius: Style.space(16)

  signal messageRequested(string userId)
  signal copyRequested(string value)
  signal openRequested(string url)
  signal shareVcfRequested(string userId, string displayName)
  signal saveRequested(string userId, string displayName, bool saved)
  /// The saved address book, so a card can say whether this person is in it.
  property var svc: null

  implicitHeight: col.implicitHeight

  Column {
    id: col
    width: parent.width
    spacing: 0

    Repeater {
      model: root.cards

      delegate: Item {
        id: cardItem
        required property var modelData
        required property int index
        width: col.width
        implicitHeight: inner.implicitHeight + Style.space(20)

        readonly property bool isMatrix: (modelData.userId || modelData.matrixId || "") !== ""
        readonly property string mxid: modelData.userId || modelData.matrixId || ""

        Rectangle {
          anchors.fill: parent
          // Only the outermost corners round, so several cards read as one stack.
          topLeftRadius: cardItem.index === 0 ? root.topLeftRadius : 0
          topRightRadius: cardItem.index === 0 ? root.topRightRadius : 0
          bottomLeftRadius: cardItem.index === root.cards.length - 1 ? root.bottomLeftRadius : 0
          bottomRightRadius: cardItem.index === root.cards.length - 1 ? root.bottomRightRadius : 0
          antialiasing: true
          color: "transparent"
          Rectangle {
            anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
            anchors.leftMargin: Style.space(14); anchors.rightMargin: Style.space(14)
            height: 1
            visible: cardItem.index > 0
            color: Util.alpha(root.fg, 0.12)
          }
        }

        Column {
          id: inner
          anchors.left: parent.left; anchors.right: parent.right
          anchors.top: parent.top; anchors.topMargin: Style.space(10)
          anchors.leftMargin: Style.space(14); anchors.rightMargin: Style.space(14)
          spacing: Style.space(8)

          // Identity
          Item {
            width: parent.width
            height: Style.space(46)
            Avatar {
              id: face
              anchors.left: parent.left; anchors.verticalCenter: parent.verticalCenter
              size: Style.space(42)
              // A vCard photo is a file the engine wrote out; a Matrix contact uses
              // the avatar cache. Either way it is a path.
              source: cardItem.modelData.photoPath || cardItem.modelData.avatarPath || ""
              name: cardItem.modelData.name || ""
              userId: cardItem.mxid
            }
            Column {
              anchors.left: face.right; anchors.leftMargin: Style.space(10)
              anchors.right: parent.right
              anchors.verticalCenter: parent.verticalCenter
              spacing: Style.space(1)
              Text {
                width: parent.width; elide: Text.ElideRight
                text: cardItem.modelData.name || cardItem.mxid || "Contact"
                color: root.fg
                font.family: Fonts.ui; font.pixelSize: Style.font.body; font.bold: true
              }
              Text {
                width: parent.width; elide: Text.ElideMiddle
                visible: text !== ""
                text: {
                  var m = cardItem.modelData
                  if (m.title && m.org) return m.title + " · " + m.org
                  return m.title || m.org || cardItem.mxid
                }
                color: Util.alpha(root.fg, 0.6)
                font.family: Fonts.ui; font.pixelSize: Style.font.caption
              }
            }
          }

          // Fields
          Repeater {
            model: {
              var m = cardItem.modelData, out = []
              var ph = m.phones || []
              for (var i = 0; i < ph.length; i++)
                out.push({ icon: Icons.phone, label: ph[i].kind || "phone", value: ph[i].value, url: "tel:" + ph[i].value })
              var em = m.emails || []
              for (var j = 0; j < em.length; j++)
                out.push({ icon: Icons.email, label: em[j].kind || "email", value: em[j].value, url: "mailto:" + em[j].value })
              if (m.address) out.push({ icon: Icons.location, label: "address", value: m.address, url: "" })
              if (m.note) out.push({ icon: Icons.note, label: "note", value: m.note, url: "" })
              return out
            }
            delegate: Item {
              required property var modelData
              width: inner.width
              height: Style.space(34)
              IconLabel { id: fIcon
                anchors.left: parent.left; anchors.verticalCenter: parent.verticalCenter
                icon: modelData.icon
                color: Util.alpha(root.fg, 0.45); size: Style.font.bodySmall }
              Column {
                anchors.left: fIcon.right; anchors.leftMargin: Style.space(9)
                anchors.right: copyBtn.left; anchors.rightMargin: Style.space(8)
                anchors.verticalCenter: parent.verticalCenter
                Text {
                  width: parent.width; elide: Text.ElideRight
                  text: modelData.value
                  color: modelData.url !== "" ? root.accent : root.fg
                  font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
                }
                Text {
                  width: parent.width; elide: Text.ElideRight
                  text: modelData.label
                  color: Util.alpha(root.fg, 0.45)
                  font.family: Fonts.ui; font.pixelSize: Style.space(8)
                }
              }
              Rectangle {
                id: copyBtn
                anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter
                width: Style.space(26); height: width; radius: width / 2
                color: cpArea.containsMouse ? Util.alpha(root.fg, 0.12) : "transparent"
                IconLabel { anchors.centerIn: parent
                  icon: Icons.copy
                  color: Util.alpha(root.fg, 0.6); size: Style.font.caption }
                MouseArea {
                  id: cpArea
                  anchors.fill: parent
                  hoverEnabled: true
                  cursorShape: Qt.PointingHandCursor
                  onClicked: root.copyRequested(modelData.value)
                }
              }
              MouseArea {
                anchors.left: parent.left; anchors.right: copyBtn.left
                anchors.top: parent.top; anchors.bottom: parent.bottom
                enabled: modelData.url !== ""
                cursorShape: Qt.PointingHandCursor
                onClicked: root.openRequested(modelData.url)
              }
            }
          }

          // Actions
          Row {
            width: parent.width
            spacing: Style.space(8)

            component CardButton: Rectangle {
              id: btn
              property alias label: btnText.text
              property bool primary: false
              property bool marked: false      // a done/steady state, e.g. Saved
              signal tapped()
              visible: cardItem.isMatrix
              width: visible ? btnText.implicitWidth + Style.space(26) : 0
              height: Style.space(32); radius: height / 2
              antialiasing: true
              readonly property color base: btn.primary ? Util.alpha(root.accent, 0.9)
                                          : (btn.marked ? Util.alpha(root.accent, 0.22)
                                                        : Util.alpha(root.fg, 0.12))
              color: btnArea.pressed ? Qt.darker(btn.base, 1.18)
                   : (btnArea.containsMouse ? Qt.lighter(btn.base, btn.primary ? 1.08 : 1.6) : btn.base)
              Behavior on color { ColorAnimation { duration: 110 } }
              scale: btnArea.pressed ? 0.97 : 1
              Behavior on scale { NumberAnimation { duration: 90; easing.type: Easing.OutCubic } }
              Text {
                id: btnText
                anchors.centerIn: parent
                color: btn.primary ? "#141414" : (btn.marked ? root.accent : root.fg)
                font.family: Fonts.ui; font.pixelSize: Style.font.bodySmall
                font.bold: btn.primary
              }
              MouseArea {
                id: btnArea
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: btn.tapped()
              }
            }

            CardButton {
              label: "Message"
              primary: true
              onTapped: root.messageRequested(cardItem.mxid)
            }

            CardButton {
              id: saveBtn
              readonly property bool inBook: !!(root.svc && root.svc.isSavedContact(cardItem.mxid))
              label: saveBtn.inBook ? "Saved" : "Save"
              marked: saveBtn.inBook
              onTapped: root.saveRequested(cardItem.mxid, cardItem.modelData.name || "", saveBtn.inBook)
            }

            CardButton {
              label: "Share"
              onTapped: root.shareVcfRequested(cardItem.mxid, cardItem.modelData.name || "")
            }
          }
        }
      }
    }
  }
}
