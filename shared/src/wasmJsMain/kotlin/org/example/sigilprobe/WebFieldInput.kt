@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)

package org.sigil

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.rememberUpdatedState
import kotlinx.browser.document
import org.w3c.dom.MutationObserver
import org.w3c.dom.MutationObserverInit
import org.w3c.dom.events.Event
import org.w3c.dom.events.KeyboardEvent
import kotlin.js.JsArray
import kotlin.js.JsString

@Composable
internal fun WebFieldInput(label: String, secret: Boolean, done: Boolean, onEnter: () -> Unit, onMenu: () -> Unit, menuKey: ((String) -> Boolean)?) {
    val enter = rememberUpdatedState(onEnter)
    val menu = rememberUpdatedState(onMenu)
    val menuKeys = rememberUpdatedState(menuKey)
    DisposableEffect(label, secret, done) {
        val root = checkNotNull(document.body!!.shadowRoot)
        val keydown: (Event) -> Unit = { event ->
            val original = event as? KeyboardEvent
            if (original != null && !original.isComposing && menuKeys.value != null &&
                ((original.repeat && original.key == "Enter") || menuKeys.value?.invoke(original.key) == true)) {
                event.preventDefault(); event.stopPropagation()
            }
            val input = root.querySelector("input")
            val key = original?.takeIf { (input == null || it.target == input) && menuKeys.value == null }
            if (key?.key == "Enter" && !key.isComposing && !key.shiftKey && !key.ctrlKey && !key.altKey && !key.metaKey) {
                event.preventDefault(); event.stopPropagation()
                if (!key.repeat) enter.value()
            } else if (key?.key == "ContextMenu" || (key?.key == "F10" && key.shiftKey)) {
                event.preventDefault(); event.stopPropagation()
                if (!key.repeat) menu.value()
            }
        }
        root.addEventListener("keydown", keydown, true)
        fun editing(active: Boolean) {
            val fields = root.querySelectorAll("[role='textbox']")
            for (i in 0 until fields.length) {
                val field = fields.item(i) as org.w3c.dom.Element
                if (field.getAttribute("aria-label") == label) {
                    if (active) field.setAttribute("aria-hidden", "true")
                    else field.removeAttribute("aria-hidden")
                }
            }
        }
        fun update() {
            val input = root.querySelector("input")
            input?.let {
                input.setAttribute("type", if (secret) "password" else "text")
                input.setAttribute("aria-label", label)
                input.setAttribute("spellcheck", "false")
                input.setAttribute("autocapitalize", "none")
                input.setAttribute("enterkeyhint", if (done) "done" else "next")
            }
            editing(input != null)
        }
        val observer = MutationObserver { _, _ -> update() }
        observer.observe(root, MutationObserverInit(childList = true, subtree = true,
            attributes = true, attributeFilter = JsArray<JsString>()))
        update()
        onDispose { observer.disconnect(); root.removeEventListener("keydown", keydown, true); editing(false) }
    }
}
