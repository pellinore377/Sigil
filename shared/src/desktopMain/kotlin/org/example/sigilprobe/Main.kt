package org.sigil

import androidx.compose.ui.window.Window
import androidx.compose.ui.window.application
import java.util.prefs.Preferences
import androidx.compose.runtime.CompositionLocalProvider

fun main() {
    val preferences = Preferences.userRoot().node("org/sigil/appearance")
    application {
        Window(onCloseRequest = ::exitApplication, title = "Sigil") {
            CompositionLocalProvider(LocalBuilderSource provides NativeCore::builderSource, LocalHelpCatalog provides NativeCore::helpCatalog, LocalTemporalPreview provides ::nativeTemporalPreview, LocalTextMotionSeeds provides NativeCore::motionSeeds, LocalEditorAnalysis provides NativeCore::editor) {
            SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "unavailable"),
                command = { _, _ -> },
                read = { preferences.get(it, null) }, write = { key, value -> preferences.put(key, value) })
            }
        }
    }
}
