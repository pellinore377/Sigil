@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)

package org.sigil

import kotlinx.browser.window
import kotlinx.coroutines.suspendCancellableCoroutine
import org.w3c.dom.clipboard.Clipboard
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlin.js.*

private external interface ClipboardNavigator : JsAny { val clipboard: Clipboard }
private val clipboard get() = window.navigator.unsafeCast<ClipboardNavigator>().clipboard

// This coroutine version's await cannot resume non-Kotlin promise rejections.
private suspend fun Promise<JsAny?>.clipboardResult(): JsAny? = suspendCancellableCoroutine { continuation ->
    then<JsAny?>(onFulfilled = {
        if (continuation.isActive) continuation.resume(it)
        null
    }, onRejected = {
        if (continuation.isActive) continuation.resumeWithException(IllegalStateException("Clipboard access was not granted."))
        null
    })
}

internal suspend fun readClipboard(): String = checkNotNull(clipboard.readText().clipboardResult()).unsafeCast<JsString>().toString()
internal suspend fun writeClipboard(text: String) { clipboard.writeText(text).clipboardResult() }
