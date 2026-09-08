@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)

package org.sigil

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import kotlinx.browser.document
import org.w3c.dom.MutationObserver
import org.w3c.dom.MutationObserverInit
import kotlin.js.JsArray
import kotlin.js.JsString

@Composable
internal fun WebFieldInput(label: String, secret: Boolean) {
    DisposableEffect(label, secret) {
        val root = checkNotNull(document.body!!.shadowRoot)
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
            }
            editing(input != null)
        }
        val observer = MutationObserver { _, _ -> update() }
        observer.observe(root, MutationObserverInit(childList = true, subtree = true,
            attributes = true, attributeFilter = JsArray<JsString>()))
        update()
        onDispose { observer.disconnect(); editing(false) }
    }
}
