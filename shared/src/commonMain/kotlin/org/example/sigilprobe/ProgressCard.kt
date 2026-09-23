package org.sigil

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.*
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlin.math.roundToInt

// The fill waits one stagger for the bubble's rise, then grows over a settle.
internal const val ProgressMotionMillis = MotionStagger + MotionSettle

@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun ProgressCard(value: UtilityContent) {
    val ink = LocalContentColor.current
    val motion = LocalMotion.current
    val target = (value.ratio ?: 0f).coerceIn(0f, 1f)
    val playback = LocalTextMotion.current?.clock.takeIf { !motion.reduced && LocalAppearance.current.messageEffects }
    val grow = MotionStandardEasing.transform((((playback?.elapsed ?: TextMotionCap.toFloat()) - MotionStagger) / MotionSettle).coerceIn(0f, 1f))
    val settled by animateFloatAsState(target, motion.tween(MotionMillis), label = "Progress value")
    val shown = settled * grow
    val figure = if (grow < 1f) "${(shown * 100).roundToInt()}%" else value.display
    val numerals = MaterialTheme.typography.displaySmall.copy(fontFeatureSettings = "tnum, lnum")
    val title = value.rich?.takeIf { it.text.isNotBlank() }
    Column(Modifier.widthIn(min = MessageCardMinWidth, max = MessageCardMaxWidth).fillMaxWidth().padding(vertical = 4.dp)
        .clearAndSetSemantics {
            contentDescription = listOfNotNull("Progress", title?.text, value.display).joinToString(". ")
            progressBarRangeInfo = ProgressBarRangeInfo(target, 0f..1f)
        }, verticalArrangement = Arrangement.spacedBy(12.dp)) {
        // A title that leaves no room for the figure stacks above it instead of wrapping beside it.
        FlowRow(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalArrangement = Arrangement.spacedBy(12.dp)) {
            val heading = MaterialTheme.typography.titleMedium
            val cap = with(LocalDensity.current) { heading.lineHeight.toDp() } * 3
            if (title != null) RichMessageText(title, Modifier.alignByBaseline().padding(end = 12.dp).heightIn(max = cap).clipToBounds(), heading)
            // The settled figure holds the width, so the count-up never shifts the row.
            Box(Modifier.alignByBaseline(), contentAlignment = if (title == null) Alignment.CenterStart else Alignment.CenterEnd) {
                Text(value.display, Modifier.alpha(0f), style = numerals, maxLines = 1)
                Text(figure, style = numerals, maxLines = 1, overflow = TextOverflow.Clip)
            }
        }
        Canvas(Modifier.fillMaxWidth().height(4.dp)) {
            val round = CornerRadius(size.height / 2)
            drawRoundRect(ink.copy(alpha = .12f), cornerRadius = round)
            val fill = maxOf(size.height, size.width * shown)
            if (shown > 0f) drawRoundRect(ink, Offset(if (layoutDirection == LayoutDirection.Rtl) size.width - fill else 0f, 0f), size.copy(width = fill), round)
        }
    }
}
