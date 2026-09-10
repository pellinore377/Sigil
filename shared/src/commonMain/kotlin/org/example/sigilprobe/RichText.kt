package org.sigil

import androidx.compose.foundation.gestures.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.*
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.*
import androidx.compose.ui.text.font.*
import androidx.compose.ui.text.style.*
import androidx.compose.ui.unit.*
import kotlin.math.abs

internal val LocalMessageSurface = staticCompositionLocalOf { Color.Unspecified }

internal fun textColor(name: String, surface: Color): Color {
    val hue = when (name.dropLast(1)) { "red" -> 5f; "orange" -> 28f; "yellow" -> 52f; "green" -> 135f; "cyan" -> 185f; "blue" -> 220f; "purple" -> 275f; "pink" -> 330f; else -> 0f }
    val shade = name.lastOrNull()?.digitToIntOrNull() ?: 2
    val dark = surface.luminance() < .18f
    var light = if (dark) .85f - shade * .06f else .44f - shade * .05f
    fun color() = Color.hsl(hue, if (name.startsWith("gray")) 0f else .62f, light)
    fun contrast(c: Color): Float { val a = c.luminance(); val b = surface.luminance(); return (maxOf(a, b) + .05f) / (minOf(a, b) + .05f) }
    while (contrast(color()) < 4.5f && light in .02f.. .98f) light = (light + if (dark) .02f else -.02f).coerceIn(0f, 1f)
    return color()
}

internal fun richPresentation(value: RichText, revealed: Set<Int>, codeFont: FontFamily, surface: Color, foreground: Color, reveal: (Int) -> Unit): AnnotatedString = buildAnnotatedString {
    val offsets = IntArray(value.text.length + 1)
    var at = 0
    fun plain(end: Int) {
        while (at < end) { offsets[at] = length; append(value.text[at++]) }
        offsets[at] = length
    }
    value.spans.forEach { span ->
        plain(span.start)
        val start = length
        if (span.reveal.isNotEmpty() && span.start !in revealed) {
            append(if (span.reveal == "scratch") "Scratch to reveal" else "Hidden text")
            addStyle(SpanStyle(color = foreground, background = foreground.copy(alpha = .16f)), start, length)
            addLink(LinkAnnotation.Clickable("reveal:${span.start}", TextLinkStyles(SpanStyle(textDecoration = TextDecoration.Underline))) { reveal(span.start) }, start, length)
            if (span.reveal == "scratch") addStringAnnotation("scratch", span.start.toString(), start, length)
            while (at < span.end) offsets[at++] = start
            offsets[at] = length
        } else {
            plain(span.end)
            val flags = span.flags
            val decoration = buildList { if ("underline" in flags) add(TextDecoration.Underline); if ("strike" in flags) add(TextDecoration.LineThrough) }
            val colors = span.colors.map { textColor(it, surface) }
            val background = "background" in flags
            addStyle(SpanStyle(
                fontWeight = if ("bold" in flags) FontWeight.Bold else null,
                fontStyle = if ("italic" in flags) FontStyle.Italic else null,
                fontFamily = if ("monospace" in flags || "code" in flags) codeFont else null,
                textDecoration = if (decoration.isEmpty()) null else TextDecoration.combine(decoration),
                fontSize = if (span.size == 0) TextUnit.Unspecified else (1f + span.size * .12f).em,
                color = if (background || colors.isEmpty()) Color.Unspecified else colors.first(),
                background = if (background) (colors.firstOrNull() ?: foreground).copy(alpha = .16f) else if ("code" in flags) foreground.copy(alpha = .08f) else Color.Unspecified,
            ), start, length)
            if (!background && colors.size > 1) addStyle(SpanStyle(brush = Brush.horizontalGradient(colors)), start, length)
            span.link?.let { addLink(LinkAnnotation.Url(it, TextLinkStyles(SpanStyle(textDecoration = TextDecoration.Underline))), start, length) }
        }
    }
    plain(value.text.length)
    value.blocks.forEach { block ->
        val start = offsets[block.start]; val end = offsets[block.end]
        if (start < end) when (block.kind) {
            "heading" -> addStyle(SpanStyle(fontSize = (1.55f - block.level * .075f).em, fontWeight = FontWeight.Bold), start, end)
            "code" -> addStyle(SpanStyle(fontFamily = codeFont, background = foreground.copy(alpha = .08f)), start, end)
            "quote" -> addStyle(SpanStyle(fontStyle = FontStyle.Italic), start, end)
        }
    }
}

@Composable
fun RichMessageText(value: RichText, modifier: Modifier = Modifier, style: TextStyle = MaterialTheme.typography.bodyLarge) {
    var revealed by remember(value) { mutableStateOf(emptySet<Int>()) }
    var layout by remember { mutableStateOf<TextLayoutResult?>(null) }
    val font = LocalCodeFont.current
    val foreground = LocalContentColor.current
    val surface = LocalMessageSurface.current.takeOrElse { MaterialTheme.colorScheme.surface }
    val text = remember(value, revealed, font, surface, foreground) { richPresentation(value, revealed, font, surface, foreground) { revealed = revealed + it } }
    Text(text, modifier.pointerInput(text) {
        awaitEachGesture {
            val down = awaitFirstDown(requireUnconsumed = false)
            val offset = layout?.getOffsetForPosition(down.position) ?: return@awaitEachGesture
            val scratch = text.getStringAnnotations("scratch", offset, offset).firstOrNull() ?: return@awaitEachGesture
            val change = awaitHorizontalTouchSlopOrCancellation(down.id) { move, _ -> move.consume() } ?: return@awaitEachGesture
            var distance = abs(change.position.x - down.position.x)
            horizontalDrag(change.id) { move ->
                distance += abs(move.position.x - move.previousPosition.x)
                move.consume()
                if (distance >= 24.dp.toPx()) revealed = revealed + scratch.item.toInt()
            }
        }
    }, style = style, onTextLayout = { layout = it })
}
