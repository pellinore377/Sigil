@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)

package org.sigil

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import kotlinx.browser.document
import kotlinx.browser.window
import org.w3c.dom.HTMLInputElement
import org.w3c.dom.events.Event
import org.w3c.dom.events.MouseEvent

/**
 * Right-clicking a hovered field opens the browser's own editing menu on a hidden input that
 * mirrors it, so Paste inserts directly instead of asking for clipboard permission.
 */
@Composable
internal fun NativeFieldMenu(hovered: () -> Boolean, value: TextFieldValue, secret: Boolean, enabled: Boolean, apply: (TextFieldValue) -> Unit, restore: () -> Unit) {
    val current = rememberUpdatedState(value)
    val over = rememberUpdatedState(hovered)
    val active = rememberUpdatedState(enabled)
    val change = rememberUpdatedState(apply)
    val back = rememberUpdatedState(restore)
    DisposableEffect(secret) {
        var mirror: HTMLInputElement? = null
        var shown = false
        fun close(refocus: Boolean) {
            val input = mirror ?: return
            mirror = null
            input.remove()
            if (refocus) back.value()
        }
        val press: (Event) -> Unit = press@{ event ->
            val mouse = event as? MouseEvent ?: return@press
            if (mouse.button.toInt() != 2 || !active.value || !over.value()) return@press
            close(false)
            shown = false
            val input = document.createElement("input") as HTMLInputElement
            input.type = if (secret) "password" else "text"
            input.setAttribute("aria-hidden", "true")
            input.setAttribute("autocomplete", "off")
            input.setAttribute("style", "position:fixed;left:${mouse.clientX - 4}px;top:${mouse.clientY - 4}px;width:8px;height:8px;" +
                "opacity:0;border:0;padding:0;margin:0;font-size:16px;z-index:2147483647")
            val value = current.value
            input.value = value.text
            // The page renders inside the body's shadow root.
            (document.body!!.shadowRoot ?: document.body!!).appendChild(input)
            input.addEventListener("input", { _ ->
                val start = input.selectionStart ?: input.value.length
                val end = input.selectionEnd ?: start
                change.value(TextFieldValue(input.value.replace('\n', ' ').replace('\r', ' '), TextRange(start, end)))
                close(true)
            })
            input.addEventListener("select", { _ ->
                change.value(TextFieldValue(input.value, TextRange(input.selectionStart ?: 0, input.selectionEnd ?: 0)))
            })
            input.addEventListener("copy", { _ -> window.setTimeout({ close(true); null }, 0) })
            input.addEventListener("keydown", { _ -> close(true) })
            input.addEventListener("blur", { _ -> if (shown) close(false) })
            // Focus only once the menu targets the mirror; the canvas takes focus on press.
            input.addEventListener("contextmenu", { _ ->
                shown = true
                input.focus()
                input.setSelectionRange(value.selection.min, value.selection.max)
            })
            // The pointer moves again only after the menu closes without a choice.
            input.addEventListener("mousemove", { _ -> if (shown) close(true) })
            mirror = input
        }
        window.addEventListener("mousedown", press, true)
        onDispose { window.removeEventListener("mousedown", press, true); close(false) }
    }
}
