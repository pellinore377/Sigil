package org.sigil

import androidx.compose.ui.window.Window
import androidx.compose.ui.window.application
import java.util.prefs.Preferences

fun main() {
    val preferences = Preferences.userRoot().node("org/sigil/appearance")
    application {
        Window(onCloseRequest = ::exitApplication, title = "Sigil") {
            SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "unavailable"),
                command = { _, _ -> },
                read = { preferences.get(it, null) }, write = { key, value -> preferences.put(key, value) })
        }
    }
}
