package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlin.math.floor
import kotlin.math.pow

// Drum timing: the bubble rises, the figures decelerate through the range, the result drops in last.
internal const val NumberPickDelay = MotionQuick
internal const val NumberPickTicks = 1800
internal const val NumberPickMillis = NumberPickDelay + NumberPickTicks + MotionInline
private const val NumberPickSteps = 27

// "-1234" -> "−1,234"; anything else passes through.
internal fun numberFigure(raw: String): String {
    val body = raw.trim().removePrefix("-")
    if (body.isEmpty() || !body.all { it.isDigit() }) return raw
    val grouped = if (body.length > 3) body.reversed().chunked(3).joinToString(",").reversed() else body
    return if (raw.trim().startsWith("-") && body.any { it != '0' }) "−$grouped" else grouped
}

internal data class NumberRange(val min: String, val max: String)

internal fun numberRange(alternate: String): NumberRange? =
    Regex("""Between (-?\d+) and (-?\d+)""").matchEntire(alternate.trim())?.let { NumberRange(it.groupValues[1], it.groupValues[2]) }

// En dash between positives; words once a minus sign would sit beside the dash.
internal fun NumberRange.label() = if (min.startsWith("-") || max.startsWith("-")) "${numberFigure(min)} to ${numberFigure(max)}" else "${numberFigure(min)}–${numberFigure(max)}"

@Composable
internal fun NumberPickCard(value: UtilityContent) {
    val range = numberRange(value.alternate)
    val motion = value.motion ?: RandomizerMotion("number", result = value.display)
    val figure = numberFigure(motion.result.ifEmpty { value.display })
    val spoken = if (range != null) "Number pick. $figure, from ${numberFigure(range.min)} to ${numberFigure(range.max)}" else "Number pick. $figure"
    Column(Modifier.widthIn(min = 140.dp, max = MessageCardMaxWidth).padding(vertical = 4.dp).clearAndSetSemantics { contentDescription = spoken },
        horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(4.dp)) {
        NumberDrum(motion.copy(result = motion.result.ifEmpty { value.display }), value.motion != null, listOfNotNull(range?.min, range?.max))
        Text(range?.label() ?: value.alternate, style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum, lnum"),
            color = LocalContentColor.current.copy(alpha = .68f), maxLines = 2, textAlign = TextAlign.Center)
    }
}

// One figure per step; a step never repeats the one before it, and the last never pre-empts the result.
internal fun drumSequence(frames: List<String>, result: String, seed: Int): List<String> {
    val distinct = frames.toSet().size
    val others = (frames.toSet() - result).isNotEmpty()
    var previous: String? = null
    return List(NumberPickSteps) { step ->
        var i = step * 5 + seed.mod(frames.size)
        fun avoid(f: String) = f == previous || step == NumberPickSteps - 1 && f == result && others
        repeat(frames.size) { if (distinct >= 2 && avoid(frames[i.mod(frames.size)])) i++ }
        frames[i.mod(frames.size)].also { previous = it }
    }
}

// The figure alone: decelerating through the stored frames, landing on the stored result.
@Composable
internal fun NumberDrum(value: RandomizerMotion, animate: Boolean, bounds: List<String> = emptyList()) {
    val motion = LocalMotion.current
    val playback = LocalTextMotion.current?.clock.takeIf { animate && !motion.reduced && LocalAppearance.current.messageEffects }
    val visible = LocalMotionVisible.current
    // Scrolled away mid-turn: the card comes back settled, never replaying unasked.
    LaunchedEffect(visible, playback) { if (!visible) playback?.elapsed = TextMotionCap.toFloat() }
    val elapsed = playback?.elapsed ?: TextMotionCap.toFloat()
    val figure = numberFigure(value.result)
    val sequence = remember(value) { drumSequence(value.frames.map(::numberFigure).ifEmpty { listOf(figure) }, figure, value.result.hashCode()) }
    val p = ((elapsed - NumberPickDelay) / NumberPickTicks).coerceIn(0f, 1f)
    val step = if (p >= 1f) NumberPickSteps else floor((1f - (1f - p).pow(3)) * NumberPickSteps).toInt()
    val shown = if (step >= NumberPickSteps) figure else sequence[step]
    // Time since this step began: the inverse of the cubic ease-out that paces the steps.
    val stepStart = NumberPickDelay + (1f - (1f - step / NumberPickSteps.toFloat()).pow(1f / 3f)) * NumberPickTicks
    val enter = if (playback == null) 1f else MotionEnterEasing.transform(((elapsed - stepStart) / (if (step >= NumberPickSteps) MotionInline.toFloat() else minOf(120f, 45f + p * 100f))).coerceIn(0f, 1f))
    val drop = with(LocalDensity.current) { 9.dp.toPx() }
    val widest = (listOf(figure) + sequence + bounds.map(::numberFigure)).maxBy { it.length }
    // Steps down with length so a long range still fits the 340dp cap; past that the figure wraps rather than clips.
    val hero = (if (widest.length <= 7) MaterialTheme.typography.displayMedium else if (widest.length <= 11) MaterialTheme.typography.displaySmall else MaterialTheme.typography.headlineSmall)
        .copy(fontFeatureSettings = "tnum, lnum", textAlign = TextAlign.Center)
    val lines = if (widest.length <= 11) 1 else Int.MAX_VALUE
    // The widest candidate holds the width so the bubble never breathes while the drum turns.
    Box(Modifier.clipToBounds(), contentAlignment = Alignment.Center) {
        Text(widest, Modifier.graphicsLayer { alpha = 0f }, style = hero, maxLines = lines)
        Text(shown, Modifier.graphicsLayer { translationY = -drop * (1f - enter); alpha = .4f + .6f * enter }, style = hero, maxLines = lines)
    }
}
