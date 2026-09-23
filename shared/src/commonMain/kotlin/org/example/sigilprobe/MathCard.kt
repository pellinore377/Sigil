package org.sigil

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.isSpecified
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.drawscope.withTransform
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.layout.FirstBaseline
import androidx.compose.ui.layout.LastBaseline
import androidx.compose.ui.layout.layout
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import kotlin.math.ceil

// Rust's layout in em: glyph outlines in font units (y up) placed on a y-down baseline grid.
data class MathTypeset(val units: Float, val width: Float, val ascent: Float, val descent: Float, val glyphs: Map<Int, String>, val runs: List<MathRun>, val rules: List<MathRule>)
data class MathRun(val glyph: Int, val x: Float, val y: Float, val scale: Float, val rgb: Long?)
data class MathRule(val x: Float, val y: Float, val width: Float, val height: Float, val rgb: Long?)

// Text and inline formulas only: the formulas join the sentence instead of stacking under it.
internal fun ChatMessage.inlineMathLine() = parts.any { it.utility?.kind == "math" && !it.utility.block } &&
    parts.all { it.kind == "text" || (it.utility?.kind == "math" && !it.utility.block) }

// A formula wider than its room shrinks to this, then scrolls.
private const val MinFit = .6f

private fun contrast(a: Color, b: Color): Float {
    val x = a.luminance(); val y = b.luminance()
    return (maxOf(x, y) + .05f) / (minOf(x, y) + .05f)
}

// A sender's \color stays legible on the reader's bubble: blended toward ink until it clears 3:1.
internal fun legibleMathColor(color: Color, surface: Color, ink: Color): Color {
    if (!surface.isSpecified) return color
    var t = 0f
    var out = color
    while (t < 1f && contrast(out, surface) < 3f) { t += .1f; out = lerp(color, ink, t.coerceAtMost(1f)) }
    return out
}

@Composable
internal fun TypesetMath(value: MathTypeset, size: TextUnit, modifier: Modifier = Modifier) {
    val ink = LocalContentColor.current
    val surface = LocalMessageSurface.current
    val paths = remember(value) { value.glyphs.mapValues { PathParser().parsePathString(it.value).toPath() } }
    val fit = remember { floatArrayOf(1f) }
    val scroll = rememberScrollState()
    // Scrolling only when it overflows, so a bubble's drag-to-reply still reaches it.
    var wide by remember(value) { mutableStateOf(false) }
    // The outer pass fits the scale to the room; past MinFit the scroll takes the rest.
    Spacer(modifier.layout { measurable, constraints ->
        val em = size.toPx()
        val natural = value.width * em
        val k = if (constraints.hasBoundedWidth && natural > constraints.maxWidth) (constraints.maxWidth / natural).coerceAtLeast(MinFit) else 1f
        fit[0] = k
        wide = natural * k > constraints.maxWidth
        val placeable = measurable.measure(constraints)
        val baseline = (ceil(em * .08f) + value.ascent * em * k).toInt()
        layout(placeable.width, placeable.height, mapOf(FirstBaseline to baseline, LastBaseline to baseline)) { placeable.place(0, 0) }
    }.graphicsLayer { compositingStrategy = CompositingStrategy.Offscreen }.drawWithContent {
        drawContent()
        // Clipped sides fade out, so the reader sees there is more to scroll.
        val fade = 16.dp.toPx().coerceAtMost(this.size.width / 4)
        if (wide && scroll.canScrollBackward) drawRect(Brush.horizontalGradient(listOf(Color.Transparent, Color.Black), 0f, fade), size = Size(fade, this.size.height), blendMode = BlendMode.DstIn)
        if (wide && scroll.canScrollForward) drawRect(Brush.horizontalGradient(listOf(Color.Black, Color.Transparent), this.size.width - fade, this.size.width), Offset(this.size.width - fade, 0f), Size(fade, this.size.height), blendMode = BlendMode.DstIn)
    }.horizontalScroll(scroll, enabled = wide).layout { measurable, constraints ->
        val em = size.toPx() * fit[0]
        val width = ceil(value.width * em).toInt().coerceAtLeast(constraints.minWidth)
        val height = (ceil((value.ascent + value.descent) * em) + 2 * ceil(size.toPx() * .08f)).toInt()
        val placeable = measurable.measure(Constraints.fixed(width, height))
        layout(width, height) { placeable.place(0, 0) }
    }.clipToBounds().drawBehind {
        val em = size.toPx() * fit[0]
        val left = (this.size.width - value.width * em).coerceAtLeast(0f) / 2
        val top = ceil(size.toPx() * .08f) + value.ascent * em
        fun paint(rgb: Long?) = rgb?.let { legibleMathColor(Color(0xff000000 or it), surface, ink) } ?: ink
        value.rules.forEach { drawRect(paint(it.rgb), Offset(left + it.x * em, top + it.y * em), Size(it.width * em, it.height * em)) }
        value.runs.forEach { run ->
            val path = paths[run.glyph] ?: return@forEach
            val k = run.scale * em / value.units
            withTransform({ translate(left + run.x * em, top + run.y * em); scale(k, -k, Offset.Zero) }) { drawPath(path, paint(run.rgb)) }
        }
    })
}

@Composable
private fun Formula(value: UtilityContent, modifier: Modifier = Modifier) {
    val size = (if (value.block) MaterialTheme.typography.headlineSmall else MaterialTheme.typography.bodyLarge).fontSize
    Box(modifier.clearAndSetSemantics { contentDescription = "Formula. ${value.display}" }, contentAlignment = Alignment.Center) {
        val typeset = value.math
        if (typeset != null) TypesetMath(typeset, size)
        // Not typeset: the TeX source, whole and wrapping, in quiet code.
        else Text(value.display, style = MaterialTheme.typography.bodyMedium.copy(fontFamily = LocalCodeFont.current))
    }
}

@Composable
internal fun MathCard(value: UtilityContent) {
    if (!value.block) { Formula(value, Modifier.widthIn(max = MessageCardMaxWidth)); return }
    Formula(value, Modifier.widthIn(min = MessageCardMinWidth, max = MessageCardMaxWidth).padding(vertical = 8.dp))
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun InlineMathMessage(message: ChatMessage, analyze: (String) -> String) {
    FlowRow(Modifier.widthIn(max = MessageCardMaxWidth), horizontalArrangement = Arrangement.spacedBy(4.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        message.parts.forEach { part ->
            val formula = part.utility
            // Formulas sit on the sentence's baseline, as typeset math does.
            if (formula != null) Formula(formula, Modifier.alignByBaseline())
            else Box(Modifier.alignByBaseline()) { if (part.rich != null) RichMessageText(part.rich) else MessageText(part.text, analyze) }
        }
    }
}
