@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)
package org.sigil

import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.Composable
import kotlinx.browser.document
import org.w3c.dom.MutationObserver
import org.w3c.dom.MutationObserverInit
import org.w3c.dom.HTMLElement
import kotlin.js.JsArray
import kotlin.js.JsString

@Composable
fun WebAccessibility() {
    DisposableEffect(Unit) {
        val root = checkNotNull(document.body!!.shadowRoot)
        var hadInput = false
        fun update() {
            val input = root.querySelector("textarea, input")
            input?.setAttribute("aria-label", "Message")
            // The backing input owns editing accessibility while it exists.
            root.querySelector("[role='textbox']")?.let {
                if (input != null) it.setAttribute("aria-hidden", "true")
                else it.removeAttribute("aria-hidden")
            }
            // Removing the backing textarea otherwise drops browser keyboard focus.
            if (hadInput && input == null && root.querySelector(":focus") == null && document.activeElement == document.body) {
                (root.querySelector("canvas") as? HTMLElement)?.focus()
            }
            hadInput = input != null
        }
        val observer = MutationObserver { _, _ -> update() }
        observer.observe(root, MutationObserverInit(childList = true, subtree = true,
            attributes = true, attributeFilter = JsArray<JsString>()))
        update()
        onDispose { observer.disconnect() }
    }
}
