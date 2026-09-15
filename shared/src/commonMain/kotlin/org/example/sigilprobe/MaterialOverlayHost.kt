package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInWindow
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.IntOffset
import kotlin.math.roundToInt

internal class MaterialOverlayHost {
    var scene by mutableStateOf<MaterialTimeline?>(null)
    var render by mutableStateOf<(@Composable (MaterialTimeline, Modifier) -> Unit)?>(null)
}
internal val LocalMaterialOverlayHost = staticCompositionLocalOf<MaterialOverlayHost?> { null }

@Composable internal fun MaterialRootOverlay(scene: MaterialTimeline, render: (@Composable (MaterialTimeline, Modifier) -> Unit)?, enabled: Boolean) {
    val host = LocalMaterialOverlayHost.current ?: return
    val current by rememberUpdatedState(render)
    val content = remember(scene) { @Composable { timeline: MaterialTimeline, modifier: Modifier -> current?.invoke(timeline, modifier); Unit } }
    DisposableEffect(host, scene, enabled, render != null) {
        if (enabled && render != null) { host.scene = scene; host.render = content }
        onDispose { if (host.render === content) { host.render = null; host.scene = null } }
    }
}

@Composable internal fun MaterialOverlayViewport(host: MaterialOverlayHost, modifier: Modifier) {
    var origin by remember { mutableStateOf(Offset.Zero) }
    val density = LocalDensity.current
    Box(modifier.onGloballyPositioned { origin = it.positionInWindow() }) {
        host.scene?.let { scene ->
            val viewport = scene.viewport
            if (viewport.width > 0 && viewport.height > 0) {
                Box(Modifier.offset { IntOffset((viewport.left - origin.x).roundToInt(), (viewport.top - origin.y).roundToInt()) }
                    .size(with(density) { viewport.width.toDp() }, with(density) { viewport.height.toDp() }).clipToBounds()) {
                    CompositionLocalProvider(LocalMaterialHandoffPass provides true) { host.render?.invoke(scene, Modifier.matchParentSize()) }
                }
            }
        }
    }
}
