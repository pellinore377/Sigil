@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)

package org.sigil

import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.window.ComposeViewport
import androidx.compose.runtime.*
import androidx.compose.ui.platform.LocalFontFamilyResolver
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.platform.Font
import org.jetbrains.compose.resources.ExperimentalResourceApi
import sigil.shared.generated.resources.Res
import kotlinx.browser.document
import kotlinx.browser.window
import androidx.compose.material3.Text

@OptIn(ExperimentalComposeUiApi::class, ExperimentalResourceApi::class)
fun main() {
    initializeRust().then {
        document.title = "Sigil · Administration"
        ComposeViewport(document.body!!) {
            val resolver = LocalFontFamilyResolver.current
            var ready by remember { mutableStateOf(false) }
            var failed by remember { mutableStateOf(false) }
            LaunchedEffect(resolver) {
                try {
                for (name in listOf("arabic", "hebrew", "emoji")) {
                    resolver.preload(FontFamily(Font("Sigil-$name", Res.readBytes("files/$name.ttf"))))
                }
                ready = true
                } catch (_: Exception) { failed = true }
            }
            if (ready) {
                AdminApp()
                SideEffect {
                    if (document.documentElement!!.getAttribute("data-ready-ms") == null) {
                        document.documentElement!!.setAttribute("data-ready-ms", window.performance.now().toString())
                    }
                }
            } else Text(if (failed) "Couldn't load display fonts. Reload to retry." else "Loading…")
        }
        null
    }
}
