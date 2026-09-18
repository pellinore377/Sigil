package org.sigil

import androidx.compose.runtime.*
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.clipPath
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.onGloballyPositioned

internal class MaterialOcclusion {
    var header by mutableStateOf(Rect.Zero)
    var footer by mutableStateOf(Rect.Zero)
    var input by mutableStateOf(Rect.Zero)
    var notice by mutableStateOf(Rect.Zero)
    fun launch(viewport: Rect, source: Rect?): Rect? {
        if (source == null || !input.overlaps(viewport)) return null
        val normal = band(viewport)
        val left = maxOf(viewport.left, source.left)
        val right = minOf(viewport.right, source.right)
        val top = maxOf(normal.top, minOf(normal.bottom, source.top))
        val bottom = minOf(source.bottom, input.top, viewport.bottom)
        return if (left < right && top < bottom) Rect(left, top, right, bottom) else null
    }
    fun visible(viewport: Rect): Rect = viewport
    /// The corridor is defined against the chrome, so it keeps the truncated band.
    private fun band(viewport: Rect): Rect {
        val top = if (header.overlaps(viewport)) maxOf(viewport.top, header.bottom) else viewport.top
        val bottom = if (footer.overlaps(viewport)) minOf(viewport.bottom, footer.top) else viewport.bottom
        return Rect(viewport.left, top, viewport.right, maxOf(top, bottom))
    }
    /// The chrome floats inset with rounded corners, so it hides only its own rectangle.
    /// Truncating the viewport at its edge sliced objects along an invisible full-width line.
    fun covered(viewport: Rect): List<Rect> =
        listOf(header, footer, notice).filter { it.overlaps(viewport) }
}
internal val LocalMaterialOcclusion = staticCompositionLocalOf<MaterialOcclusion?> { null }

internal fun materialClipInsets(viewport: Rect, left: Float, top: Float, width: Float, height: Float): List<Float> {
    if (width <= 0 || height <= 0 || !width.isFinite() || !height.isFinite()) return listOf(100f, 0f, 0f, 0f)
    return listOf(
        ((viewport.top - top) / height * 100).coerceIn(0f, 100f),
        ((left + width - viewport.right) / width * 100).coerceIn(0f, 100f),
        ((top + height - viewport.bottom) / height * 100).coerceIn(0f, 100f),
        ((viewport.left - left) / width * 100).coerceIn(0f, 100f),
    )
}

internal val LocalMaterialLaunchWindow = staticCompositionLocalOf<Rect?> { null }

@Composable internal fun Modifier.materialOcclusion(): Modifier {
    val timeline = LocalMaterialTimeline.current
    if (timeline == null || LocalObjectMenu.current) return this
    val occlusion = LocalMaterialOcclusion.current
    val source = LocalMaterialLaunchWindow.current
    var coordinates by remember { mutableStateOf<LayoutCoordinates?>(null) }
    // A scrolled row is translated, not redrawn; counting placements makes the recorded clip follow it.
    var placed by remember { mutableIntStateOf(0) }
    val path = remember { Path() }
    return onGloballyPositioned { coordinates = it; placed++ }.drawWithContent {
        placed
        val layout = coordinates?.takeIf { it.isAttached } ?: return@drawWithContent
        path.reset()
        fun add(rect: Rect) {
            if (rect.width <= 0 || rect.height <= 0) return
            val a = layout.windowToLocal(rect.topLeft)
            val b = layout.windowToLocal(Offset(rect.right, rect.top))
            val c = layout.windowToLocal(rect.bottomRight)
            val d = layout.windowToLocal(Offset(rect.left, rect.bottom))
            path.moveTo(a.x, a.y); path.lineTo(b.x, b.y); path.lineTo(c.x, c.y); path.lineTo(d.x, d.y)
            path.close()
        }
        materialClipRegions(timeline.viewport, occlusion?.launch(timeline.viewport, source), occlusion?.covered(timeline.viewport).orEmpty()).forEach(::add)
        clipPath(path) { this@drawWithContent.drawContent() }
    }
}

private fun subtract(bounds: Rect, exclusion: Rect): List<Rect> {
    if(!bounds.overlaps(exclusion)) return listOf(bounds)
    val hole=bounds.intersect(exclusion)
    return listOf(Rect(bounds.left,bounds.top,bounds.right,hole.top),Rect(bounds.left,hole.bottom,bounds.right,bounds.bottom),
        Rect(bounds.left,hole.top,hole.left,hole.bottom),Rect(hole.right,hole.top,bounds.right,hole.bottom))
}
internal fun materialClipRegions(visible: Rect, launch: Rect?, exclusions: List<Rect>): List<Rect> =
    exclusions.fold(listOfNotNull(visible,launch)) { regions,exclusion -> regions.flatMap {subtract(it,exclusion)} }
        .filter {it.width>0 && it.height>0}
internal fun materialClipRegions(visible: Rect, launch: Rect?, exclusion: Rect?): List<Rect> =
    materialClipRegions(visible,launch,listOfNotNull(exclusion))

internal fun materialClipPath(visible: Rect, launch: Rect?, left: Float, top: Float, exclusions: List<Rect>): String {
    val regions=materialClipRegions(visible,launch,exclusions)
    if(regions.isEmpty())return "inset(100%)"
    return "path('"+regions.joinToString(" "){rect->"M ${rect.left-left} ${rect.top-top} H ${rect.right-left} V ${rect.bottom-top} H ${rect.left-left} Z"}+"')"
}
internal fun materialClipPath(visible: Rect, launch: Rect?, left: Float, top: Float, exclusion: Rect? = null): String =
    materialClipPath(visible,launch,left,top,listOfNotNull(exclusion))

/** Keeps GPU surfaces only for objects that can contribute pixels, including the launch corridor. */
@Composable fun materialViewportVisible(bounds: () -> Rect): Boolean {
    val timeline=LocalMaterialTimeline.current ?: return true
    if(LocalObjectMenu.current)return true
    val occlusion=LocalMaterialOcclusion.current
    val source=LocalMaterialLaunchWindow.current
    val currentBounds=rememberUpdatedState(bounds)
    return remember(timeline,occlusion,source) { derivedStateOf {
        val viewport=timeline.viewport
        val margin=viewport.height.coerceAtLeast(1f)
        val retained=Rect(viewport.left,viewport.top-margin,viewport.right,viewport.bottom+margin)
        val bounds=currentBounds.value()
        bounds==Rect.Zero || retained.overlaps(bounds) ||
            occlusion?.launch(viewport,source)?.overlaps(currentBounds.value())==true
    } }.value
}
