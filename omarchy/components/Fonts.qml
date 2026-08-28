pragma Singleton
import QtQuick

// The app's own fonts, loaded from `fonts/` rather than asked of the system.
//
// QML's `font` grouped property has `family` and no `families`, so there is no
// per-character fallback: a Text draws either icons or words, never both. Getting it
// wrong does not show as tofu — an icon codepoint falls back to whatever font holds
// that private-use range, which may draw a completely different icon.
QtObject {
  id: root

  readonly property FontLoader uiRegular: FontLoader { source: Qt.resolvedUrl("../fonts/Roboto-Regular.ttf") }
  readonly property FontLoader uiMedium:  FontLoader { source: Qt.resolvedUrl("../fonts/Roboto-Medium.ttf") }
  readonly property FontLoader uiBold:    FontLoader { source: Qt.resolvedUrl("../fonts/Roboto-Bold.ttf") }
  readonly property FontLoader monoFace:  FontLoader { source: Qt.resolvedUrl("../fonts/RobotoMono-Regular.ttf") }
  readonly property FontLoader iconFace:  FontLoader { source: Qt.resolvedUrl("../fonts/MaterialSymbolsRounded.ttf") }
  // The FILL axis lives in the 30 MB variable font; Google serves a static instance per
  // axis value, so the filled set ships as a second family rather than an axis setting.
  readonly property FontLoader iconFilledFace: FontLoader { source: Qt.resolvedUrl("../fonts/MaterialSymbolsRounded-Filled.ttf") }

  /// Family names, as the loaded files declare them.
  readonly property string ui: root.uiRegular.status === FontLoader.Ready ? root.uiRegular.name : "sans-serif"
  readonly property string mono: root.monoFace.status === FontLoader.Ready ? root.monoFace.name : "monospace"
  readonly property string icon: root.iconFace.status === FontLoader.Ready ? root.iconFace.name : "sans-serif"
  /// Solid rather than outlined. Same codepoints — only the family differs.
  readonly property string iconFilled: root.iconFilledFace.status === FontLoader.Ready ? root.iconFilledFace.name : root.icon


  readonly property bool ready: root.uiRegular.status === FontLoader.Ready
                             && root.iconFace.status === FontLoader.Ready
                             && root.iconFilledFace.status === FontLoader.Ready
  function debugState() {
    return JSON.stringify({ ui: root.ui, mono: root.mono, icon: root.icon,
                            iconFilled: root.iconFilled, ready: root.ready,
                            status: [root.uiRegular.status, root.monoFace.status, root.iconFace.status] })
  }
}
