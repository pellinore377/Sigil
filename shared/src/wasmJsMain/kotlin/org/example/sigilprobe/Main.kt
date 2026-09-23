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
        val preview=window.location.pathname=="/preview"
        document.title = if(preview)"Sigil · Design workbench" else if(window.location.pathname=="/admin")"Sigil · Administration" else "Sigil"
        ComposeViewport(document.body!!) {
            val resolver = LocalFontFamilyResolver.current
            var ready by remember { mutableStateOf(false) }
            var failed by remember { mutableStateOf(false) }
            LaunchedEffect(resolver) {
                try {
                for (name in listOf("arabic", "hebrew", "ipa", "emoji")) {
                    resolver.preload(FontFamily(Font("Sigil-$name", Res.readBytes("files/$name.ttf"))))
                }
                ready = true
                } catch (_: Exception) { failed = true }
            }
            if (ready) {
                WebNativeMenus { if(preview)WebPreview() else if(window.location.pathname=="/admin")AdminApp() else if(window.location.pathname=="/messenger")WebMessenger() else MessengerEntry() }
                SideEffect {
                    (document.body?.shadowRoot?.querySelector("canvas") as? org.w3c.dom.HTMLCanvasElement)?.style?.display="block"
                    if (document.documentElement!!.getAttribute("data-ready-ms") == null) {
                        document.documentElement!!.setAttribute("data-ready-ms", window.performance.now().toString())
                    }
                }
            } else Text(if (failed) "Couldn't load display fonts. Reload to retry." else "Loading…")
        }
        null
    }
}

@Composable private fun MessengerEntry() {
    var complete by remember {mutableStateOf<Boolean?>(null)}
    var failed by remember {mutableStateOf(false)}
    LaunchedEffect(Unit) {
        try {complete=(api("/setup/v0/status") as kotlinx.serialization.json.JsonObject).bool("complete")}
        catch(_:Exception){failed=true}
    }
    when(complete) {
        true->WebMessenger()
        false->AdminApp()
        null->Text(if(failed)"Could not reach your server. Reload to retry." else "Opening Sigil…")
    }
}
