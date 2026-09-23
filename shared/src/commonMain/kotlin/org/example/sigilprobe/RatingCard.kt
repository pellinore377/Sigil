package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.drawscope.clipRect
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.unit.dp

// Stars fill one after another over at most five stagger steps, after the bubble's rise.
internal const val RatingMotionMillis = MotionStagger * 5 + MotionInline

// "4/5" or "3.5/5"; a scale past ten stars falls back to five scaled by the ratio.
internal data class RatingScale(val value: String, val max: String, val stars: Int, val filled: Float)

internal fun ratingScale(display: String, ratio: Float?): RatingScale {
    val share = (ratio ?: 0f).coerceIn(0f, 1f)
    val parts = display.split('/').map { it.trim() }
    val max = parts.getOrNull(1)?.toFloatOrNull()
    val whole = max != null && max == kotlin.math.floor(max) && max in 1f..10f
    val stars = if (whole) max!!.toInt() else 5
    // Halves are the finest step a star shows; anything finer lives in the figure.
    val filled = kotlin.math.round(share * stars * 2) / 2
    return RatingScale(parts.getOrElse(0) { display }, parts.getOrElse(1) { "" }, stars, filled)
}

// Speaks "stars" only when the stars drawn match the scale, so 70/100 is not read as 100 stars.
internal fun ratingSpoken(display: String, scale: RatingScale): String = when {
    scale.max.isEmpty() -> display
    scale.max.toFloatOrNull() == scale.stars.toFloat() -> "${scale.value} out of ${scale.max} stars"
    else -> "${scale.value} out of ${scale.max}"
}

@Composable
internal fun RatingCard(value: UtilityContent) {
    val scale = ratingScale(value.display, value.ratio)
    val ink = LocalContentColor.current
    val motion = LocalMotion.current
    val playback = LocalTextMotion.current?.clock.takeIf { !motion.reduced && LocalAppearance.current.messageEffects }
    val elapsed = playback?.elapsed ?: TextMotionCap.toFloat()
    val glyph = if (scale.stars > 5) 20 else 24
    val lit = kotlin.math.ceil(scale.filled).toInt().coerceAtLeast(1)
    val step = MotionStagger * minOf(lit, 5) / lit.toFloat()
    val spoken = ratingSpoken(value.display, scale)
    Column(Modifier.widthIn(min = MessageCardMinWidth, max = MessageCardMaxWidth).padding(vertical = 4.dp)
        .clearAndSetSemantics { contentDescription = "Rating. $spoken" }, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(if (scale.stars > 5) 2.dp else 4.dp), verticalAlignment = Alignment.CenterVertically) {
            repeat(scale.stars) { star ->
                val amount = (scale.filled - star).coerceIn(0f, 1f)
                val shown = if (amount <= 0f) 0f else MotionStandardEasing.transform(((elapsed - MotionStagger - star * step) / MotionInline).coerceIn(0f, 1f))
                Box(contentAlignment = Alignment.Center) {
                    CompositionLocalProvider(LocalContentColor provides ink.copy(alpha = .56f)) { Glyph("star", glyph) }
                    // A half star is the full star clipped at its middle, from the start edge.
                    if (shown > 0f) Box(Modifier.graphicsLayer { alpha = shown; scaleX = .8f + .2f * shown; scaleY = scaleX }.drawWithContent {
                        val w = size.width * amount
                        if (layoutDirection == LayoutDirection.Rtl) clipRect(left = size.width - w) { this@drawWithContent.drawContent() }
                        else clipRect(right = w) { this@drawWithContent.drawContent() }
                    }) { Glyph("star", glyph, filled = true) }
                }
            }
        }
        Text(value.display, style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum, lnum"), color = ink.copy(alpha = .68f), maxLines = 1)
    }
}
