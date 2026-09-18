package org.sigil

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties

// A full-screen page shown over the app in its own window: the glass headers blur what is really beneath them.
internal class PresentationHost {
    var content by mutableStateOf<(@Composable () -> Unit)?>(null)
}
internal val LocalPresentationHost = staticCompositionLocalOf<PresentationHost?> { null }

// Shows the content over everything for as long as this stays composed; back closes it. Without a host it falls back to a dialog.
@Composable fun Presented(close: () -> Unit, content: @Composable () -> Unit) {
    val host = LocalPresentationHost.current
    val current by rememberUpdatedState(content)
    BackAction(true, close)
    if (host == null) {
        Dialog(close, DialogProperties(usePlatformDefaultWidth = false)) { content() }
        return
    }
    val slot = remember { @Composable { current() } }
    DisposableEffect(host, slot) {
        host.content = slot
        onDispose { if (host.content === slot) host.content = null }
    }
}

// The page covers the app: touches that its content does not take stop here rather than reaching what lies beneath.
@Composable internal fun PresentationViewport(host: PresentationHost, modifier: Modifier) {
    host.content?.let { Box(modifier.fillMaxSize().pointerInput(Unit) { awaitPointerEventScope { while (true) awaitPointerEvent() } }) { it() } }
}
