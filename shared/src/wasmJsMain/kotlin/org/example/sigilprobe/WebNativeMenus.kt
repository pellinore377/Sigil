@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class, androidx.compose.foundation.ExperimentalFoundationApi::class)

package org.sigil

import androidx.compose.foundation.text.contextmenu.provider.LocalTextContextMenuDropdownProvider
import androidx.compose.foundation.text.contextmenu.provider.TextContextMenuDataProvider
import androidx.compose.foundation.text.contextmenu.provider.TextContextMenuProvider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import kotlinx.browser.document
import kotlinx.browser.window
import org.w3c.dom.Element
import org.w3c.dom.HTMLElement
import org.w3c.dom.HTMLInputElement
import org.w3c.dom.HTMLTextAreaElement
import org.w3c.dom.ShadowRoot
import org.w3c.dom.events.Event
import org.w3c.dom.events.KeyboardEvent
import org.w3c.dom.events.KeyboardEventInit
import org.w3c.dom.events.MouseEvent
import org.w3c.dom.events.MouseEventInit

// Over a field the browser's menu shows instead of Compose's; read-only selections keep Compose's.
private class FieldMenu(private val menus: NativeMenus, private val fallback: TextContextMenuProvider?) : TextContextMenuProvider {
    override suspend fun showTextContextMenu(dataProvider: TextContextMenuDataProvider) { if (!menus.native) fallback?.showTextContextMenu(dataProvider) }
}

/** Right-clicks on a field reach the browser's own menu on Compose's backing input; message bubbles keep Sigil's actions. */
private class NativeMenus(private val root: ShadowRoot) {
    private var x = 0; private var y = 0
    private var raised: HTMLElement? = null
    private var saved: String? = null
    private var spelling: String? = null
    private var shown = false
    val native get() = raised != null
    // Only where Enter sends does the browser's own line break need blocking.
    var enterSends = false
    private var handled = false
    private var keyComposing = false
    private var composition = false
    val composing get() = keyComposing || composition

    private fun backing() = root.querySelector("textarea,input") as? HTMLElement
    private fun Element.contains(px: Int, py: Int) = getBoundingClientRect().let { px >= it.left && px <= it.right && py >= it.top && py <= it.bottom }
    // The textbox rect is only its text lines; the field's padding counts too.
    private fun Element.near(px: Int, py: Int) = getBoundingClientRect().let { px >= it.left - 16 && px <= it.right + 16 && py >= it.top - 16 && py <= it.bottom + 16 }
    private fun textbox(): Element? {
        val boxes = root.querySelectorAll("[role='textbox']")
        return (0 until boxes.length).map { boxes.item(it) as Element }.lastOrNull { it.getAttribute("aria-disabled") != "true" && it.near(x, y) }
    }
    private fun HTMLElement.range() = when (this) {
        is HTMLTextAreaElement -> (selectionStart ?: 0) to (selectionEnd ?: 0)
        is HTMLInputElement -> (selectionStart ?: 0) to (selectionEnd ?: 0)
        else -> 0 to 0
    }
    private fun HTMLElement.length() = (this as? HTMLTextAreaElement)?.value?.length ?: (this as? HTMLInputElement)?.value?.length ?: 0
    private fun cover() =
        "position:fixed;left:${x - 4}px;top:${y - 4}px;width:8px;height:8px;opacity:0;border:0;padding:0;margin:0;resize:none;" +
            "color:transparent;background:transparent;caret-color:transparent;font-size:16px;z-index:2147483647;pointer-events:auto"

    fun close() {
        raised?.let { element ->
            element.setAttribute("style", saved.orEmpty())
            spelling?.let { element.setAttribute("spellcheck", it) } ?: element.removeAttribute("spellcheck")
        }
        raised = null; saved = null; spelling = null
        shown = false
    }

    // A press at the pointer focuses the field and places the caret there, as a browser does on right-click.
    private fun press() {
        val canvas = root.querySelector("canvas") ?: return
        for (type in listOf("mousedown", "mouseup")) canvas.dispatchEvent(MouseEvent(type, MouseEventInit(clientX = x, clientY = y, button = 0,
            buttons = if (type == "mousedown") 1 else 0, bubbles = true, cancelable = true, composed = true)))
    }

    private fun field(event: MouseEvent): Boolean {
        val box = textbox() ?: return false
        val current = backing()
        val here = current != null && current.getBoundingClientRect().let { box.contains((it.left + it.right).toInt() / 2, (it.top + it.bottom).toInt() / 2) }
        // A selection in the field under the pointer stays, so Cut and Copy act on it.
        if (!here || current!!.range().let { it.first == it.second }) press()
        val element = backing() ?: return false
        event.stopPropagation()
        saved = element.getAttribute("style"); spelling = element.getAttribute("spellcheck")
        element.setAttribute("style", cover())
        if ((element as? HTMLInputElement)?.type != "password") element.setAttribute("spellcheck", "true")
        raised = element
        return true
    }

    private val mac = window.navigator.platform.startsWith("Mac")
    // Ctrl+click is the Mac's right-click.
    private fun MouseEvent.secondary() = button.toInt() == 2 || mac && button.toInt() == 0 && ctrlKey

    private val down: (Event) -> Unit = down@{ event ->
        val mouse = event as? MouseEvent ?: return@down
        if (!mouse.isTrusted) return@down
        close()
        if (!mouse.secondary()) return@down
        x = mouse.clientX; y = mouse.clientY
        handled = field(mouse)
    }
    // Where the menu follows the release, it comes at once; a cover left without one never blocks the next click.
    private val up: (Event) -> Unit = up@{ event ->
        val mouse = event as? MouseEvent ?: return@up
        if (!mouse.isTrusted || !handled && mouse.button.toInt() != 2) return@up
        if (handled) event.stopPropagation()
        handled = false
        window.setTimeout({ if (!shown) close(); null }, 100)
    }
    private val menu: (Event) -> Unit = menu@{ event ->
        val target = event.composedPath().toList().firstOrNull() ?: return@menu
        // The menu key or Shift+F10 opens on the focused field where it stands.
        if (raised == null && target == backing()) (target as HTMLElement).let { raised = it; saved = it.getAttribute("style"); spelling = it.getAttribute("spellcheck") }
        if (target != raised) return@menu
        // Compose cancels every context menu that reaches its listeners.
        event.stopPropagation()
        shown = true
    }
    // The page sees the pointer move again only once the menu has closed.
    private val move: (Event) -> Unit = { event -> val mouse = event as MouseEvent; if (shown && kotlin.math.abs(mouse.clientX - x) + kotlin.math.abs(mouse.clientY - y) > 2) close() }
    private val key: (Event) -> Unit = { event ->
        val key = event as KeyboardEvent
        keyComposing = key.isComposing || key.keyCode == 229
        if (key.isTrusted) close()
    }
    private val compose: (Event) -> Unit = { event -> composition = event.type != "compositionend" }
    // The native Select all changes only the DOM selection; Compose learns of it as its own shortcut.
    private val select: (Event) -> Unit = select@{ event ->
        val element = raised ?: return@select
        if (event.target != element || element.length() == 0 || element.range() != 0 to element.length()) return@select
        element.dispatchEvent(KeyboardEvent("keydown", KeyboardEventInit(key = "a", code = "KeyA", ctrlKey = !mac, metaKey = mac, bubbles = true, cancelable = true, composed = true)))
    }
    // Compose inserts line breaks from the key itself; the browser's own would linger in the backing input after a send.
    private val lineBreak: (Event) -> Unit = { event -> if (enterSends && event.unsafeCast<TypedInput>().inputType in listOf("insertLineBreak", "insertParagraph")) event.preventDefault() }
    private val edited: (Event) -> Unit = { event -> if (event.composedPath().toList().firstOrNull() == raised) window.setTimeout({ close(); null }, 0) }
    private val listeners = listOf("mousedown" to down, "mouseup" to up, "contextmenu" to menu, "mousemove" to move, "keydown" to key, "compositionstart" to compose,
        "compositionend" to compose, "beforeinput" to lineBreak, "select" to select, "input" to edited, "copy" to edited, "cut" to edited, "paste" to edited)

    // On the shadow root, ahead of Compose's canvas and window listeners, and seeing uncomposed events such as select.
    fun attach() { listeners.forEach { (type, listener) -> root.addEventListener(type, listener, true) } }
    fun detach() { listeners.forEach { (type, listener) -> root.removeEventListener(type, listener, true) }; close() }
}

private external interface TypedInput : kotlin.js.JsAny { val inputType: String }
private fun <T : kotlin.js.JsAny?> kotlin.js.JsArray<T>.toList() = (0 until length).mapNotNull { get(it) }

@Composable
internal fun WebNativeMenus(content: @Composable () -> Unit) {
    val root = document.body!!.shadowRoot
    if (root == null) { content(); return }
    val menus = remember(root) { NativeMenus(root) }
    DisposableEffect(menus) { menus.attach(); onDispose { menus.detach() } }
    // Enter sends unless the primary pointer is touch; a touch keyboard's Enter breaks the line, as on Android.
    val query = remember { runCatching { window.matchMedia("(pointer: coarse)") }.getOrNull() }
    var sends by remember { mutableStateOf(query?.matches != true) }
    DisposableEffect(query) {
        val listener: (Event) -> Unit = { sends = query?.matches != true }
        query?.addEventListener("change", listener)
        onDispose { query?.removeEventListener("change", listener) }
    }
    menus.enterSends = sends
    val fallback = LocalTextContextMenuDropdownProvider.current
    val provider = remember(menus, fallback) { FieldMenu(menus, fallback) }
    CompositionLocalProvider(LocalTextContextMenuDropdownProvider provides provider,
        LocalImeComposing provides if (sends) { { menus.composing } } else null) { content() }
}
