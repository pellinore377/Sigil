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
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.foundation.focusable
import androidx.compose.ui.input.key.*
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.text
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


/** A SigilText named colour resolved against the ground it sits on; see shadeColor. */
internal fun textColor(name: String, surface: Color, alternates: List<Color> = emptyList()): Color = shadeColor(name, surface, alternates)

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
            // Laid out as written, at its own size and weight, so uncovering only lifts the veil.
            plain(span.end)
            addStyle(SpanStyle(
                fontWeight = if ("bold" in span.flags) FontWeight.Bold else null,
                fontStyle = if ("italic" in span.flags) FontStyle.Italic else null,
                fontFamily = if ("monospace" in span.flags || "code" in span.flags) codeFont else null,
                fontSize = if (span.size != 0) sizeStep(span.size).em else if ("code" in span.flags) .88.em else TextUnit.Unspecified,
            ), start, length)
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
                // Monospace sets large beside the serif; inline code steps down to sit level with it.
                fontSize = if (span.size != 0) sizeStep(span.size).em else if ("code" in flags) .88.em else TextUnit.Unspecified,
                color = if (background || colors.isEmpty()) Color.Unspecified else colors.first(),
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
            "heading" -> addStyle(SpanStyle(fontSize = (1.28f - block.level * .075f).em), start, end)
            "code" -> addStyle(SpanStyle(fontFamily = codeFont, background = foreground.copy(alpha = .08f)), start, end)
            "quote" -> addStyle(SpanStyle(fontStyle = FontStyle.Italic), start, end)
            else -> {}
        }
    }
}

/** A code panel sits closer to the bubble edge than body text does. */
internal val CodeBleed = 14.dp
internal fun Modifier.bleed(amount: Dp) = layout { measurable, constraints ->
    val extra = amount.roundToPx() * 2
    val placeable = measurable.measure(constraints.copy(minWidth = 0, maxWidth = constraints.maxWidth + extra))
    layout((placeable.width - extra).coerceAtLeast(0), placeable.height) { placeable.place(-amount.roundToPx(), 0) }
}

/** Structured text is laid out block by block; a capped preview stays one run of text. */
@Composable
fun RichMessageText(value: RichText, modifier: Modifier = Modifier, style: TextStyle = MaterialTheme.typography.bodyLarge, maxLines: Int = Int.MAX_VALUE) {
    val nodes = remember(value, maxLines) { if (maxLines == Int.MAX_VALUE) markdownTree(value) else emptyList() }
    if (nodes.isEmpty()) RichInlineText(value, modifier, style, maxLines) else MarkdownBody(value, nodes, style, modifier)
}
/** True where the caller draws the surrounding bubble, so a code panel may replace it. */
val LocalMessageBubble = staticCompositionLocalOf { false }
// The bubble's own colour, painted by captions and text chunks; the surface itself stays clear under filled content so no fringe shows at the corners.
val LocalBubbleGround = staticCompositionLocalOf { Color.Transparent }
// A revealed spoiler stays revealed when its row is scrolled back or the conversation reopened.
internal val LocalMessageKey = staticCompositionLocalOf<String?> { null }
private val revealLedger = mutableStateMapOf<String, Set<Int>>()

/** Trailing blank lines inside a fence would otherwise pad the panel with dead space. */
internal fun trimmedEnd(value: RichText, block: RichBlock): Int {
    var end = block.end
    while (end > block.start && value.text[end - 1].isWhitespace()) end--
    return if (end > block.start) end else block.end
}
/** The blank lines that separate blocks belong to neither: they must not pad a caption bubble. */
internal fun trimmedSlice(value: RichText, from: Int, to: Int): RichText? {
    var start = from; var end = to
    while (start < end && value.text[start].isWhitespace()) start++
    while (end > start && value.text[end - 1].isWhitespace()) end--
    return if (start < end) richSlice(value, start, end) else null
}

/** The marker stays in the canonical body; the rule replaces it on screen. */
@Composable
internal fun QuoteBlock(value: RichText, style: TextStyle) {
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
        value.motion.mapNotNull { run -> run.copy(units = run.units.map { map(it.first) to map(it.second) }.filter { it.first < it.second }).takeIf { it.units.isNotEmpty() } },
        value.mentions.mapNotNull { m -> map(m.start).let { s -> map(m.end).let { e -> if (s < e) m.copy(start = s, end = e) else null } } })
}

@Composable
internal fun RichInlineText(value: RichText, modifier: Modifier = Modifier, style: TextStyle = MaterialTheme.typography.bodyLarge, maxLines: Int = Int.MAX_VALUE) {
    val ledgerKey = LocalMessageKey.current?.let { "$it/${value.text.hashCode()}/${value.spans.size}" }
    var local by remember(value) { mutableStateOf(emptySet<Int>()) }
    val revealed = if (ledgerKey != null) revealLedger[ledgerKey].orEmpty() else local
    fun reveal(set: Set<Int>) { if (ledgerKey != null) { if (revealLedger.size >= 4096) revealLedger.clear(); revealLedger[ledgerKey] = set } else local = set }
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
            reveal(revealed + start); wiping = start
            wipe.animateTo(1f, motion.tween(if (spoiler) 650 else 600))
            if (wiping == start) wiping = null
        }
    }
    val directory = LocalMentionDirectory.current
    val mentions = remember(value, directory) { mentionMarks(value, directory) }
    val after = mentionSpacingAfter()
    val (text, sizedStyle) = remember(value, revealed, font, surface, foreground, alternates, style, mentions) {
        sizedParagraphs(richPresentation(value, revealed, font, surface, foreground, alternates, ::uncover), style).let { (shown, sizedStyle) ->
            // An address keeps its server, quietly: the handle is what reads. 76%, not 68%: it sits on the chip, which narrows contrast.
            val servers = mentions.mapNotNull { (run, _) -> run.server?.let { motionOffsets(value, revealed, run.start + 1 + run.handle.length) to motionOffsets(value, revealed, run.end) } }
            (if (servers.isEmpty()) shown else AnnotatedString.Builder(shown).apply { servers.forEach { (a, b) -> if (a < b && b <= length) addStyle(SpanStyle(color = foreground.copy(alpha = .76f)), a, b) } }.toAnnotatedString()).withMentionRoom(mentions, value, revealed, after) to sizedStyle
        }
    }
    val uriHandler = LocalUriHandler.current
    MentionHost(mentions.isNotEmpty(), modifier, { MentionTargets(mentions, value, revealed, layout, after) }) { modifier ->
    Text(text, modifier
        .concealedSemantics(value, revealed, ::uncover) { runCatching { uriHandler.openUri(it) } }
        .mentionChips(mentions, value, revealed, layout, foreground, after)
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
        }, style = sizedStyle, maxLines = maxLines, overflow = if (maxLines == Int.MAX_VALUE) TextOverflow.Clip else TextOverflow.Ellipsis, onTextLayout = { layout = it })
    }
}
/** What a screen reader hears: never a redacted or still-hidden secret. Null when the drawn text already reads true. */
internal fun spokenText(value: RichText, revealed: Set<Int>): String? {
    val legacy = redactionRanges(value)
    val hidden = value.spans.filter { (it.reveal.isNotEmpty() && it.start !in revealed) || it.redaction > 0 }
    if (hidden.isEmpty() && legacy.isEmpty()) return null
    val cuts = (hidden.map { Triple(it.start, it.end, if (it.redaction > 0) "[REDACTED]" else if (it.reveal == "scratch") "[Scratch to reveal, hidden]" else "[Spoiler, hidden]") } +
        legacy.map { (from, to) -> Triple(from, to, "[REDACTED]") }).sortedBy { it.first }
    return buildString {
        var at = 0
        cuts.forEach { (from, to, said) -> if (from >= at) { append(value.text, at, from); append(said); at = to } }
        append(value.text, at, value.text.length)
    }
}
/** Hidden text is replaced in semantics, with a Reveal action per veil; links stay reachable as actions. */
internal fun Modifier.concealedSemantics(value: RichText, revealed: Set<Int>, uncover: (Int) -> Unit, open: (String) -> Unit): Modifier {
    val spoken = spokenText(value, revealed) ?: return this
    val veiled = value.spans.filter { it.reveal.isNotEmpty() && it.start !in revealed }
    val scratch = veiled.firstOrNull { it.reveal == "scratch" }
    val actions = veiled.mapIndexed { i, span ->
        CustomAccessibilityAction(if (veiled.size == 1) "Reveal" else "Reveal ${i + 1}") { uncover(span.start); true }
    } + value.spans.mapNotNull { span -> span.link?.takeIf { span.reveal.isEmpty() || span.start in revealed }?.let { url -> CustomAccessibilityAction("Open $url") { open(url); true } } }
    // Scratch is a drag; Enter or Space reveals it for a keyboard.
    val keys = if (scratch == null) this else this.onKeyEvent { event ->
        if (event.type == KeyEventType.KeyUp && (event.key == Key.Enter || event.key == Key.Spacebar || event.key == Key.NumPadEnter)) { uncover(scratch.start); true } else false
    }.focusable()
    return keys.clearAndSetSemantics { text = AnnotatedString(spoken); customActions = actions }
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
    // Inline code gets the same squircle, a touch wider so the glyphs have air.
    val code=value.spans.filter {"code" in it.flags && "background" !in it.flags && it.reveal.isEmpty()}
    val redactions=redactionRanges(value)
    val counted=value.spans.filter {it.redaction>0}
    if((marked.isEmpty() && code.isEmpty() && redactions.isEmpty() && counted.isEmpty()) || layout==null)return Modifier
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
        val pad=3.dp.toPx()
        val chips=code.flatMap {span->
            val start=motionOffsets(value,revealed,span.start)
            val end=motionOffsets(value,revealed,span.end)
            if(start<0 || start>=end || end>layout.layoutInput.text.length)emptyList()
            else (layout.getLineForOffset(start)..layout.getLineForOffset(end-1)).mapNotNull {line->
                lineSpanEdges(layout,line,start,end)?.let {(a,b)->
                    val base=layout.getLineBaseline(line)
                    val box=Rect(a-pad,base-em*.86f,b+pad,base+em*.30f)
                    if(box.width<=pad*2)null else squirclePath(box,em*.28f)
                }
            }
        }
        val chip=foreground.copy(alpha=.10f)
        val bar=if(surface.luminance()<.5f)Color(0xff070709) else foreground
        // The reference rings the bar so it parts from a dark ground; the ring is as strong as 3:1 needs.
        val ring=if(contrastRatio(bar,surface)>=3f)null else redactionRing(surface,foreground)
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
            chips.forEach {drawPath(it,chip)}
            shapes.forEach {(path,tint)->drawPath(path,tint)}
            drawContent()
            bars.forEach {drawPath(it,bar);if(ring!=null)drawPath(it,ring,style=androidx.compose.ui.graphics.drawscope.Stroke(1.dp.toPx()))}
        }
    }
}
internal fun contrastRatio(a:Color,b:Color):Float {val x=a.luminance()+.05f;val y=b.luminance()+.05f;return maxOf(x,y)/minOf(x,y)}
/** The faintest ink ring, from the reference's 8%, that parts a redaction bar from its ground at 3:1. */
internal fun redactionRing(surface:Color,ink:Color):Color {
    var alpha=.08f
    while(alpha<1f && contrastRatio(lerp(surface,ink,alpha),surface)<3f)alpha+=.02f
    return lerp(surface,ink,alpha.coerceAtMost(1f))
}
