package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.input.pointer.*
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.layout.positionInWindow
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import kotlin.math.roundToInt

private class MenuEntry(val position: State<Offset>, val dismiss: State<() -> Unit>, val content: State<@Composable () -> Unit>)
private class MenuHostState { var entry by mutableStateOf<MenuEntry?>(null) }
private val LocalMenuHost = staticCompositionLocalOf<MenuHostState> { error("Missing menu host") }

@Composable
internal fun AdminMenuHost(content: @Composable () -> Unit) {
    val host = remember { MenuHostState() }
    var origin by remember { mutableStateOf(Offset.Zero) }
    var size by remember { mutableStateOf(IntSize.Zero) }
    CompositionLocalProvider(LocalMenuHost provides host) {
        Box(Modifier.fillMaxSize().onGloballyPositioned { origin = it.positionInWindow(); size = it.size }) {
            content()
            host.entry?.let { entry ->
                var menuSize by remember(entry) { mutableStateOf(IntSize.Zero) }
                val relative = entry.position.value - origin
                val offset = IntOffset(relative.x.roundToInt().coerceIn(0, (size.width - menuSize.width).coerceAtLeast(0)),
                    relative.y.roundToInt().coerceIn(0, (size.height - menuSize.height).coerceAtLeast(0)))
                val bounds = Rect(offset.x.toFloat(), offset.y.toFloat(), (offset.x + menuSize.width).toFloat(), (offset.y + menuSize.height).toFloat())
                Box(Modifier.matchParentSize().pointerInput(entry, bounds) {
                    awaitPointerEventScope {
                        while (true) {
                            val event = awaitPointerEvent(PointerEventPass.Initial)
                            if (event.type == PointerEventType.Press && event.changes.any { !bounds.contains(it.position) }) {
                                entry.dismiss.value(); event.changes.forEach { it.consume() }
                            }
                        }
                    }
                }) {
                    Surface(Modifier.offset { offset }.onSizeChanged { menuSize = it }.semantics { paneTitle = "Editing menu" },
                        shape = RoundedCornerShape(10.dp), color = MaterialTheme.colorScheme.surfaceContainer, shadowElevation = 6.dp) {
                        Column(Modifier.width(IntrinsicSize.Max).widthIn(min = 112.dp, max = 280.dp).padding(vertical = 8.dp)) { entry.content.value() }
                    }
                }
            }
        }
    }
}

@Composable
internal fun AdminMenu(position: Offset, dismiss: () -> Unit, content: @Composable () -> Unit) {
    val host = LocalMenuHost.current
    val positionState = rememberUpdatedState(position)
    val dismissState = rememberUpdatedState(dismiss)
    val contentState = rememberUpdatedState(content)
    DisposableEffect(host) {
        val entry = MenuEntry(positionState, dismissState, contentState)
        host.entry = entry
        onDispose { if (host.entry === entry) host.entry = null }
    }
}
