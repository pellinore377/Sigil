package org.sigil

import androidx.compose.animation.core.Animatable
import androidx.compose.foundation.gestures.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.foundation.background
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.Modifier
import kotlinx.coroutines.launch
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.layout
import androidx.compose.ui.draw.drawWithCache
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.*
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.*
import androidx.compose.ui.text.font.*
import androidx.compose.ui.text.style.*
import androidx.compose.ui.unit.*
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.absoluteValue
import kotlin.math.cos
import kotlin.math.floor
import kotlin.math.pow
import kotlin.math.sin
import kotlin.math.sqrt

val LocalMessageSurface = staticCompositionLocalOf { Color.Unspecified }
// Further backgrounds a glyph can land on (page gradient, tinted rows); ink must clear all of them.
internal val LocalMessageSurfaceAlternates = staticCompositionLocalOf { emptyList<Color>() }
private const val PaintUnits = 192


internal fun textColor(name: String, surface: Color, alternates: List<Color> = emptyList()): Color {
    val hue = when (name.dropLast(1)) { "red" -> 5f; "orange" -> 28f; "yellow" -> 52f; "green" -> 135f; "cyan" -> 185f; "blue" -> 220f; "purple" -> 275f; "pink" -> 330f; else -> 0f }
    val shade = name.lastOrNull()?.digitToIntOrNull() ?: 2
    val saturation = if (name.startsWith("gray")) 0f else .80f
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
            // Laid out as written; the veil drawn over it does the concealing until it is uncovered.
            plain(span.end)
            if (span.reveal == "scratch") addStringAnnotation("scratch", span.start.toString(), start, length)
            else addLink(LinkAnnotation.Clickable("reveal:${span.start}", null) { reveal(span.start) }, start, length)
        } else if (span.redaction > 0) {
            // The body keeps one placeholder; the drawn run is as long as what was removed, and the bar covers it.
            append("x".repeat(span.redaction))
            addStyle(SpanStyle(color = Color.Transparent), start, length)
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
                background = if ("code" in flags && !background) foreground.copy(alpha = .08f) else Color.Unspecified,
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
    redactionRanges(value).forEach { (from, to) -> addStyle(SpanStyle(color = Color.Transparent), offsets[from], offsets[to]) }
    value.blocks.forEach { block ->
        val start = offsets[block.start]; val end = offsets[block.end]
        if (start < end) when (block.kind) {
            "heading" -> addStyle(SpanStyle(fontSize = (1.28f - block.level * .075f).em, fontWeight = FontWeight.Bold), start, end)
            "code" -> addStyle(SpanStyle(fontFamily = codeFont, background = foreground.copy(alpha = .08f)), start, end)
            "quote" -> addStyle(SpanStyle(fontStyle = FontStyle.Italic), start, end)
            else -> {}
        }
    }
}

/** A code panel sits closer to the bubble edge than body text does. */
internal val CodeBleed = 14.dp
private fun Modifier.bleed(amount: Dp) = layout { measurable, constraints ->
    val extra = amount.roundToPx() * 2
    val placeable = measurable.measure(constraints.copy(minWidth = 0, maxWidth = constraints.maxWidth + extra))
    layout((placeable.width - extra).coerceAtLeast(0), placeable.height) { placeable.place(-amount.roundToPx(), 0) }
}

/** Blocks that cannot live inside one paragraph: fenced code, and quotations with their rule. */
private fun standaloneBlocks(value: RichText): List<RichBlock> {
    val code = visibleCodeBlocks(value)
    val quotes = value.blocks.filter { it.kind == "quote" && it.start >= 0 && it.end <= value.text.length && it.start < it.end }
        .filter { outer -> value.blocks.none { it.kind == "quote" && it !== outer && it.start <= outer.start && it.end >= outer.end && (it.start < outer.start || it.end > outer.end) } }
        .filter { quote -> code.none { it.start < quote.end && it.end > quote.start } }
    return (code + quotes).sortedBy { it.start }
}

private class MessageChunk(val value: RichText, val code: RichBlock?, val quote: Boolean)

/** A code panel is a bubble of its own; the captions around it are bubbles too, grouped against it. */
@Composable
fun RichMessageText(value: RichText, modifier: Modifier = Modifier, style: TextStyle = MaterialTheme.typography.bodyLarge) {
    val blocks = remember(value) { standaloneBlocks(value) }
    if (blocks.isEmpty()) { RichInlineText(value, modifier, style); return }
    val chunks = remember(value, blocks) {
        buildList {
            var start = 0
            blocks.forEach { block ->
                if (block.start > start) trimmedSlice(value, start, block.start)?.let { add(MessageChunk(it, null, false)) }
                if (block.kind == "code") add(MessageChunk(richSlice(value, block.start, trimmedEnd(value, block)), block, false))
                else trimmedSlice(value, block.start, block.end)?.let { add(MessageChunk(it, null, true)) }
                start = block.end
            }
            if (start < value.text.length) trimmedSlice(value, start, value.text.length)?.let { add(MessageChunk(it, null, false)) }
        }
    }
    val panelled = chunks.any { it.code != null } && LocalMessageBubble.current
    if (!panelled) {
        Column(modifier, verticalArrangement = Arrangement.spacedBy(12.dp)) {
            chunks.forEach { chunk ->
                if (chunk.code != null) CodeBlock(chunk.value, chunk.code.language, Modifier.bleed(CodeBleed))
                else if (chunk.quote) QuoteBlock(chunk.value, style)
                else RichInlineText(chunk.value, style = style)
            }
        }
        return
    }
    // One bubble, no gaps: the panel runs edge to edge and the bubble's own shape rounds the outside.
    Column(modifier) {
        chunks.forEach { chunk ->
            if (chunk.code != null) CodeBlock(chunk.value, chunk.code.language, Modifier.fillMaxWidth(), RectangleShape)
            else Box(Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 10.dp)) {
                if (chunk.quote) QuoteBlock(chunk.value, style) else RichInlineText(chunk.value, style = style)
            }
        }
    }
}
/** True where the caller draws the surrounding bubble, so a code panel may replace it. */
val LocalMessageBubble = staticCompositionLocalOf { false }

/** Trailing blank lines inside a fence would otherwise pad the panel with dead space. */
private fun trimmedEnd(value: RichText, block: RichBlock): Int {
    var end = block.end
    while (end > block.start && value.text[end - 1].isWhitespace()) end--
    return if (end > block.start) end else block.end
}
/** The blank lines that separate blocks belong to neither: they must not pad a caption bubble. */
private fun trimmedSlice(value: RichText, from: Int, to: Int): RichText? {
    var start = from; var end = to
    while (start < end && value.text[start].isWhitespace()) start++
    while (end > start && value.text[end - 1].isWhitespace()) end--
    return if (start < end) richSlice(value, start, end) else null
}

/** The marker stays in the canonical body; the rule replaces it on screen. */
@Composable
private fun QuoteBlock(value: RichText, style: TextStyle) {
    val trimmed = remember(value) { quoteBody(value) }
    Row(Modifier.fillMaxWidth().height(IntrinsicSize.Min), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        Box(Modifier.width(3.dp).fillMaxHeight().clip(RoundedCornerShape(2.dp)).background(LocalContentColor.current.copy(alpha = .32f)))
        RichInlineText(trimmed, Modifier.weight(1f), style)
    }
}
internal fun quoteBody(value: RichText): RichText {
    val marker = Regex("(?m)^> ?")
    if (!marker.containsMatchIn(value.text)) return value
    val keep = ArrayList<Int>(value.text.length)
    var at = 0
    val out = StringBuilder(value.text.length)
    while (at < value.text.length) {
        val lineStart = at == 0 || value.text[at - 1] == '\n'
        if (lineStart && value.text.startsWith(">", at)) { at += if (value.text.startsWith("> ", at)) 2 else 1; continue }
        keep.add(at); out.append(value.text[at]); at++
    }
    val moved = IntArray(value.text.length + 1)
    var next = 0
    for (index in 0..value.text.length) { if (next < keep.size && keep[next] == index) { moved[index] = next; next++ } else moved[index] = next }
    fun map(v: Int) = moved[v.coerceIn(0, value.text.length)]
    return RichText(out.toString(),
        value.spans.mapNotNull { span -> map(span.start).let { s -> map(span.end).let { e -> if (s < e) span.copy(start = s, end = e) else null } } },
        value.blocks.mapNotNull { block -> map(block.start).let { s -> map(block.end).let { e -> if (s < e) block.copy(start = s, end = e) else null } } },
        value.codeTokens.mapNotNull { token -> map(token.start).let { s -> map(token.end).let { e -> if (s < e) token.copy(start = s, end = e) else null } } },
        value.motion.mapNotNull { run -> run.copy(units = run.units.map { map(it.first) to map(it.second) }.filter { it.first < it.second }).takeIf { it.units.isNotEmpty() } })
}

@Composable
private fun RichInlineText(value: RichText, modifier: Modifier = Modifier, style: TextStyle = MaterialTheme.typography.bodyLarge) {
    var revealed by remember(value) { mutableStateOf(emptySet<Int>()) }
    var layout by remember(value) { mutableStateOf<TextLayoutResult?>(null) }
    var brushed by remember(value) { mutableStateOf(emptyList<Offset>()) }
    var wiping by remember(value) { mutableStateOf<Int?>(null) }
    var tapPoint by remember(value) { mutableStateOf(Offset.Zero) }
    val wipe = remember(value) { Animatable(1f) }
    val motion = LocalMotion.current
    val haptic = LocalHapticFeedback.current
    val font = LocalCodeFont.current
    val foreground = LocalContentColor.current
    val surface = LocalMessageSurface.current.takeOrElse { MaterialTheme.colorScheme.surface }
    val alternates = LocalMessageSurfaceAlternates.current
    val scope = rememberCoroutineScope()
    fun uncover(start: Int) {
        if (start in revealed) return
        val spoiler = value.spans.any { it.start == start && it.reveal == "spoiler" }
        scope.launch {
            wipe.snapTo(0f)
            revealed = revealed + start; wiping = start
            wipe.animateTo(1f, motion.tween(if (spoiler) 650 else 600))
            if (wiping == start) wiping = null
        }
    }
    val text = remember(value, revealed, font, surface, foreground, alternates) { richPresentation(value, revealed, font, surface, foreground, alternates, ::uncover) }
    Text(text, modifier
        .then(markHighlights(value, revealed, layout, foreground, surface, alternates))
        .then(revealVeil(value, revealed, layout, foreground, surface, brushed, wiping, wipe.value, tapPoint))
        .then(textMotion(value, revealed, layout, foreground, surface))
        .pointerInput(text) {
            awaitEachGesture {
                val down = awaitFirstDown(requireUnconsumed = false)
                tapPoint = down.position
                val placed = layout ?: return@awaitEachGesture
                val offset = placed.getOffsetForPosition(down.position)
                val tag = text.getStringAnnotations("scratch", offset, offset).firstOrNull() ?: return@awaitEachGesture
                val span = value.spans.firstOrNull { it.start == tag.item.toIntOrNull() } ?: return@awaitEachGesture
                if (span.start in revealed) return@awaitEachGesture
                val radius = InkBrush.toPx()
                val cell = InkCell.toPx()
                val step = 4.dp.toPx()
                val eligible = inkCells(revealRects(value, revealed, placed, span, 0f), cell)
                val covered = HashSet<Long>()
                var last = down.position
                var buzzed = false
                fun paint(to: Offset) {
                    val delta = to - last
                    val steps = (sqrt(delta.x * delta.x + delta.y * delta.y) / step).toInt().coerceIn(1, 40)
                    val marks = (0..steps).map { last + delta * (it.toFloat() / steps) }
                    brushed = (brushed + marks).takeLast(1200)
                    marks.forEach { covered += brushedCells(it, radius * .72f, cell, eligible) }
                    last = to
                    if (!buzzed && eligible.isNotEmpty() && covered.size >= eligible.size * .2f) { buzzed = true; haptic.performHapticFeedback(HapticFeedbackType.LongPress) }
                    if (motion.reduced || (eligible.isNotEmpty() && covered.size >= eligible.size * .67f)) uncover(span.start)
                }
                // A touch on the ink is a scratch, not a scroll: claim it from the first point.
                down.consume()
                paint(down.position)
                drag(down.id) { change -> change.consume(); paint(change.position) }
            }
        }, style = style, onTextLayout = { layout = it })
}
/** A superellipse corner, matching the reference stylesheet's continuous rounding. */
internal fun squirclePath(rect:Rect,radius:Float):Path {
    val r=radius.coerceAtMost(minOf(rect.width,rect.height)/2f)
    if(r<=0f)return Path().apply {addRect(rect)}
    val steps=8
    val path=Path()
    var started=false
    // |x/r|^4 + |y/r|^4 = 1, walked clockwise so each corner starts where the last edge ended.
    fun corner(cx:Float,cy:Float,sx:Float,sy:Float,fromTop:Boolean) {
        for(step in 0..steps) {
            val a=(if(fromTop)steps-step else step).toFloat()/steps*(PI.toFloat()/2f)
            val x=cx+sx*r*cos(a).absoluteValue.pow(.5f)
            val y=cy+sy*r*sin(a).absoluteValue.pow(.5f)
            if(!started) {path.moveTo(x,y);started=true} else path.lineTo(x,y)
        }
    }
    corner(rect.right-r,rect.top+r,1f,-1f,true)
    corner(rect.right-r,rect.bottom-r,1f,1f,false)
    corner(rect.left+r,rect.bottom-r,-1f,1f,true)
    corner(rect.left+r,rect.top+r,-1f,-1f,false)
    path.close()
    return path
}

/** Redaction is permanent removal: the placeholder is the content, sized to what it replaced, and drawn as a solid bar. */
private val RedactionPlaceholder=Regex("""\[(?=R)(?:REDACTED ?)*(?:R|RE|RED|REDA|REDAC|REDACT|REDACTE)?\]""")
internal fun redactionRanges(value:RichText):List<Pair<Int,Int>> =
    if('[' !in value.text) emptyList() else RedactionPlaceholder.findAll(value.text).map {it.range.first to it.range.last+1}
        // A counted span draws its own bar; this only covers messages from before the count existed.
        .filter {(from,to)->value.spans.none {it.redaction>0 && it.start<=from && it.end>=to}}.toList()

/** Per-line squircle highlights and redaction bars, drawn under the text; a span background cannot carry a corner. */
@Composable
internal fun markHighlights(value:RichText,revealed:Set<Int>,layout:TextLayoutResult?,foreground:Color,surface:Color,alternates:List<Color>):Modifier {
    val marked=value.spans.filter {"background" in it.flags}
    val redactions=redactionRanges(value)
    val counted=value.spans.filter {it.redaction>0}
    if((marked.isEmpty() && redactions.isEmpty() && counted.isEmpty()) || layout==null)return Modifier
    return Modifier.drawWithCache {
        val em=(layout.layoutInput.style.fontSize.takeIf {it.type==TextUnitType.Sp}?.toPx()) ?: 16.sp.toPx()
        val shapes=marked.flatMap {span->
            val tint=(span.colors.firstOrNull()?.let {textColor(it,surface,alternates)} ?: foreground).copy(alpha=.21f)
            val start=motionOffsets(value,revealed,span.start)
            val end=motionOffsets(value,revealed,span.end)
            if(start<0 || start>=end || end>layout.layoutInput.text.length)emptyList()
            else (layout.getLineForOffset(start)..layout.getLineForOffset(end-1)).mapNotNull {line->
                lineSpanEdges(layout,line,start,end)?.let {(a,b)->
                    // Measured off the baseline and the em, not the line's glyph extent, so descenders cannot resize it.
                    val base=layout.getLineBaseline(line)
                    // The text's own extent, as a redaction: no side padding to run into a neighbour.
                    val box=Rect(a,base-em*.89f,b,base+em*.29f)
                    if(box.width<=0f || box.height<=0f)null else squirclePath(box,em*.30f) to tint
                }
            }
        }
        val bar=if(surface.luminance()<.5f)Color(0xff070709) else foreground
        val ranges=redactions.map {(from,to)->motionOffsets(value,revealed,from) to motionOffsets(value,revealed,to)}+
            counted.map {span->motionOffsets(value,revealed,span.start).let {it to it+span.redaction}}
        val bars=ranges.flatMap {(start,end)->
            if(start<0 || start>=end || end>layout.layoutInput.text.length)emptyList()
            else (layout.getLineForOffset(start)..layout.getLineForOffset(end-1)).mapNotNull {line->
                lineSpanEdges(layout,line,start,end)?.let {(a,b)->
                    val base=layout.getLineBaseline(line)
                    // Level with a highlight, but no wider than the placeholder: an opaque bar must not cover a neighbour.
                    val box=Rect(a,base-em*.89f,b,base+em*.29f)
                    if(box.width<=0f)null else squirclePath(box,em*.30f)
                }
            }
        }
        onDrawWithContent {
            shapes.forEach {(path,tint)->drawPath(path,tint)}
            drawContent()
            bars.forEach {drawPath(it,bar)}
        }
    }
}
