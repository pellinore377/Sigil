package org.sigil

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.draw.dropShadow
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.layer.GraphicsLayer
import androidx.compose.ui.graphics.layer.drawLayer
import androidx.compose.ui.graphics.shadow.Shadow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInWindow
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.DpOffset
import androidx.compose.ui.unit.dp

internal class ChromeBackdrop(val layer: GraphicsLayer) {
    var origin = Offset.Zero
}

@Composable internal fun rememberChromeBackdrop(): ChromeBackdrop {
    val layer = rememberGraphicsLayer()
    return remember(layer) { ChromeBackdrop(layer) }
}

internal fun Modifier.captureBackdrop(backdrop: ChromeBackdrop) = onGloballyPositioned {
    backdrop.origin = it.positionInWindow()
}.drawWithContent {
    backdrop.layer.record { this@drawWithContent.drawContent() }
    drawLayer(backdrop.layer)
}

@Composable internal fun FloatingChrome(backdrop: ChromeBackdrop?, modifier: Modifier, shape: Shape, content: @Composable () -> Unit) {
    var origin by remember { mutableStateOf(Offset.Zero) }
    val radius = with(LocalDensity.current) { 14.dp.toPx() }
    val effect = remember(radius) { BlurEffect(radius, radius, TileMode.Clamp) }
    val glass = backdrop != null && effect.isSupported()
    val blurred = rememberGraphicsLayer()
    SideEffect { blurred.renderEffect = if (glass) effect else null }
    val color = if (backdrop == null) MaterialTheme.colorScheme.background else MaterialTheme.colorScheme.surfaceContainerHigh
    // Elevation shadows assume opaque content and expose descendant-shaped gaps through glass.
    val shadow = if (backdrop == null) Modifier else Modifier.dropShadow(shape,
        Shadow(radius = 8.dp, color = Color.Black.copy(alpha = .15f), offset = DpOffset(0.dp, 3.dp)))
    Surface(modifier.then(shadow).clip(shape).onGloballyPositioned { origin = it.positionInWindow() }.drawWithContent {
        if (glass) {
            val offset = origin - backdrop!!.origin
            blurred.record { translate(-offset.x, -offset.y) { drawLayer(backdrop.layer) } }
            drawLayer(blurred)
        }
        drawContent()
    }, shape = shape, color = if (glass) color.copy(alpha = .82f) else color, shadowElevation = 0.dp) {
        content()
    }
}
