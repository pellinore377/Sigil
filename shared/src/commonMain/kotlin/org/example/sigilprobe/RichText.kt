package org.sigil

import androidx.compose.animation.core.Animatable
import androidx.compose.foundation.gestures.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.*
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.*
import androidx.compose.ui.text.font.*
import androidx.compose.ui.text.style.*
import androidx.compose.ui.unit.*
import kotlin.math.abs
import kotlin.math.floor
import kotlin.math.sqrt

val LocalMessageSurface = staticCompositionLocalOf { Color.Unspecified }
// Further backgrounds a glyph can land on (page gradient, tinted rows); ink must clear all of them.
internal val LocalMessageSurfaceAlternates = staticCompositionLocalOf { emptyList<Color>() }
private const val PaintUnits = 192

internal fun revealPlaceholder(kind: String) = if (kind == "scratch") "Scratch to reveal" else "Hidden text"

internal fun textColor(name: String, surface: Color, alternates: List<Color> = emptyList()): Color {
    val hue = when (name.dropLast(1)) { "red" -> 5f; "orange" -> 28f; "yellow" -> 52f; "green" -> 135f; "cyan" -> 185f; "blue" -> 220f; "purple" -> 275f; "pink" -> 330f; else -> 0f }
    val shade = name.lastOrNull()?.digitToIntOrNull() ?: 2
    val saturation = if (name.startsWith("gray")) 0f else .62f
    val grounds = (alternates + surface).map { it.luminance() }
    fun contrast(light: Float): Float {
        val a = Color.hsl(hue, saturation, light).luminance()
        return grounds.minOf { b -> (maxOf(a, b) + .05f) / (minOf(a, b) + .05f) }
    }
    // Correct toward whichever side of the background has headroom; a fixed direction muddies mid-tone bubbles.
    fun walk(step: Float): Pair<Float, Float> {
        var light = .66f - shade * .08f
        while (contrast(light) < 4.5f && light + step in 0f..1f) light += step
        return light to contrast(light)
    }
    val nominal = .66f - shade * .08f
    val up = walk(.02f); val down = walk(-.02f)
    return Color.hsl(hue, saturation, when {
        up.second >= 4.5f && down.second >= 4.5f -> if (up.first - nominal <= nominal - down.first) up.first else down.first
        up.second >= 4.5f -> up.first
        down.second >= 4.5f -> down.first
        else -> if (up.second >= down.second) up.first else down.first
    })
}

internal fun gradientStop(colors: List<Color>, index: Int, count: Int): Color {
    if (colors.size < 2) return colors.firstOrNull() ?: Color.Unspecified
    val position = if (count <= 1) 0f else index.toFloat() / (count - 1) * (colors.size - 1)
    val left = floor(position).toInt().coerceIn(0, colors.size - 1)
    return lerp(colors[left], colors[(left + 1).coerceAtMost(colors.size - 1)], position - left)
}

private fun codePointAt(text: String, index: Int) =
    if (text[index].isHighSurrogate() && index + 1 < text.length && text[index + 1].isLowSurrogate())
        0x10000 + ((text[index].code - 0xd800) shl 10) + (text[index + 1].code - 0xdc00) else text[index].code
private fun extending(point: Int) = point == 0x200d || point == 0x20e3 || point in 0x300..0x36f || point in 0x1ab0..0x1aff ||
    point in 0x1dc0..0x1dff || point in 0x20d0..0x20ff || point in 0xfe00..0xfe0f || point in 0xfe20..0xfe2f ||
    point in 0x1f3fb..0x1f3ff || point in 0xe0020..0xe007f

/** Cluster boundaries within [text]; a gradient or a particle colours whole graphemes, never half an emoji. */
internal fun graphemeCuts(text: String): List<Int> {
    val cuts = ArrayList<Int>(text.length + 1)
    var index = 0; var join = false; var pending = false
    while (index < text.length) {
        val point = codePointAt(text, index)
        val regional = point in 0x1f1e6..0x1f1ff
        if (cuts.isEmpty() || !(join || extending(point) || (regional && pending))) cuts.add(index)
        pending = regional && !pending
        join = point == 0x200d
        index += if (point > 0xffff) 2 else 1
    }
    cuts.add(text.length)
    return cuts
}

internal fun richPresentation(value: RichText, revealed: Set<Int>, codeFont: FontFamily, surface: Color, foreground: Color,
    alternates: List<Color> = emptyList(), reveal: (Int) -> Unit): AnnotatedString = buildAnnotatedString {
    val offsets = IntArray(value.text.length + 1)
    var at = 0
    fun plain(end: Int) {
        while (at < end) { offsets[at] = length; append(value.text[at++]) }
        offsets[at] = length
    }
    value.spans.forEach { span ->
        plain(span.start)
        val start = length
        val from = at
        if (span.reveal.isNotEmpty() && span.start !in revealed) {
            append(revealPlaceholder(span.reveal))
            addStyle(SpanStyle(color = Color.Transparent), start, length)
            addLink(LinkAnnotation.Clickable("reveal:${span.start}", null) { reveal(span.start) }, start, length)
            if (span.reveal == "scratch") addStringAnnotation("scratch", span.start.toString(), start, length)
            while (at < span.end) offsets[at++] = start
            offsets[at] = length
        } else {
            plain(span.end)
            val flags = span.flags
            val decoration = buildList { if ("underline" in flags) add(TextDecoration.Underline); if ("strike" in flags) add(TextDecoration.LineThrough) }
            val background = "background" in flags
            val ground = if (!background && "code" in flags) lerp(surface, foreground, .08f) else surface
            val colors = span.colors.map { textColor(it, ground, alternates) }
            addStyle(SpanStyle(
                fontWeight = if ("bold" in flags) FontWeight.Bold else null,
                fontStyle = if ("italic" in flags) FontStyle.Italic else null,
                fontFamily = if ("monospace" in flags || "code" in flags) codeFont else null,
                textDecoration = if (decoration.isEmpty()) null else TextDecoration.combine(decoration),
                fontSize = if (span.size == 0) TextUnit.Unspecified else (1f + span.size * .12f).em,
                color = if (background || colors.isEmpty()) Color.Unspecified else colors.first(),
                background = if (background) (colors.firstOrNull() ?: foreground).copy(alpha = .16f) else if ("code" in flags) foreground.copy(alpha = .08f) else Color.Unspecified,
            ), start, length)
            if (!background && colors.size > 1) {
                // A span brush is sized to the whole layout, so a short span would show one slice of the ramp.
                val cuts = graphemeCuts(value.text.substring(from.coerceAtMost(span.end), span.end))
                val count = cuts.size - 1
                if (count in 1..PaintUnits) for (index in 0 until count) addStyle(SpanStyle(color = gradientStop(colors, index, count)), start + cuts[index], start + cuts[index + 1])
                else addStyle(SpanStyle(brush = Brush.horizontalGradient(colors)), start, length)
            }
            span.link?.let { addLink(LinkAnnotation.Url(it, TextLinkStyles(SpanStyle(textDecoration = TextDecoration.Underline))), start, length) }
        }
    }
    plain(value.text.length)
    value.blocks.forEach { block ->
        val start = offsets[block.start]; val end = offsets[block.end]
        if (start < end) when (block.kind) {
            "heading" -> addStyle(SpanStyle(fontSize = (1.28f - block.level * .075f).em, fontWeight = FontWeight.Bold), start, end)
            "code" -> addStyle(SpanStyle(fontFamily = codeFont, background = foreground.copy(alpha = .08f)), start, end)
            "quote" -> addStyle(SpanStyle(fontStyle = FontStyle.Italic), start, end)
        }
    }
}

@Composable
fun RichMessageText(value: RichText, modifier: Modifier = Modifier, style: TextStyle = MaterialTheme.typography.bodyLarge) {
    val code = remember(value) { visibleCodeBlocks(value) }
    if (code.isEmpty()) { RichInlineText(value, modifier, style); return }
    Column(modifier, verticalArrangement = Arrangement.spacedBy(12.dp)) {
        var start = 0
        code.forEach { block ->
            if (block.start > start) richSlice(value, start, block.start).takeIf { it.text.isNotBlank() }?.let { RichInlineText(it, style = style) }
            CodeBlock(richSlice(value, block.start, block.end), block.language)
            start = block.end
        }
        if (start < value.text.length) richSlice(value, start, value.text.length).takeIf { it.text.isNotBlank() }?.let { RichInlineText(it, style = style) }
    }
}

@Composable
private fun RichInlineText(value: RichText, modifier: Modifier = Modifier, style: TextStyle = MaterialTheme.typography.bodyLarge) {
    var revealed by remember(value) { mutableStateOf(emptySet<Int>()) }
    var layout by remember(value) { mutableStateOf<TextLayoutResult?>(null) }
    var brushed by remember(value) { mutableStateOf(emptyList<Offset>()) }
    var wiping by remember(value) { mutableStateOf<Int?>(null) }
    val wipe = remember(value) { Animatable(1f) }
    val motion = LocalMotion.current
    val haptic = LocalHapticFeedback.current
    val font = LocalCodeFont.current
    val foreground = LocalContentColor.current
    val surface = LocalMessageSurface.current.takeOrElse { MaterialTheme.colorScheme.surface }
    val alternates = LocalMessageSurfaceAlternates.current
    fun uncover(start: Int) { if (start !in revealed) { revealed = revealed + start; wiping = start } }
    val text = remember(value, revealed, font, surface, foreground, alternates) { richPresentation(value, revealed, font, surface, foreground, alternates, ::uncover) }
    LaunchedEffect(wiping) {
        val active = wiping ?: return@LaunchedEffect
        wipe.snapTo(0f)
        wipe.animateTo(1f, motion.tween(MotionInline))
        if (wiping == active) wiping = null
    }
    Text(text, modifier
        .then(revealVeil(value, revealed, layout, foreground, surface, brushed, wiping, wipe.value))
        .then(textMotion(value, revealed, layout, foreground, surface))
        .pointerInput(text) {
            awaitEachGesture {
                val down = awaitFirstDown(requireUnconsumed = false)
                val placed = layout ?: return@awaitEachGesture
                val offset = placed.getOffsetForPosition(down.position)
                val tag = text.getStringAnnotations("scratch", offset, offset).firstOrNull() ?: return@awaitEachGesture
                val span = value.spans.firstOrNull { it.start == tag.item.toIntOrNull() } ?: return@awaitEachGesture
                val change = awaitHorizontalTouchSlopOrCancellation(down.id) { move, _ -> move.consume() } ?: return@awaitEachGesture
                val radius = InkBrush.toPx()
                val cell = InkCell.toPx()
                val eligible = inkCells(revealRects(value, revealed, placed, span, 0f), cell)
                val covered = HashSet<Long>()
                var distance = abs(change.position.x - down.position.x)
                val threshold = 24.dp.toPx()
                var last = down.position
                fun paint(to: Offset) {
                    val delta = to - last
                    val steps = (sqrt(delta.x * delta.x + delta.y * delta.y) / (radius * .5f)).toInt().coerceIn(1, 24)
                    val marks = (1..steps).map { last + delta * (it.toFloat() / steps) }
                    brushed = (brushed + marks).takeLast(96)
                    marks.forEach { covered += brushedCells(it, radius * .72f, cell, eligible) }
                    last = to
                }
                paint(change.position)
                horizontalDrag(change.id) { move ->
                    val before = distance
                    distance += abs(move.position.x - move.previousPosition.x)
                    move.consume()
                    paint(move.position)
                    if (distance >= threshold && before < threshold) haptic.performHapticFeedback(HapticFeedbackType.LongPress)
                    if (motion.reduced || (eligible.isNotEmpty() && covered.size >= eligible.size * .67f)) uncover(span.start)
                }
            }
        }, style = style, onTextLayout = { layout = it })
}
