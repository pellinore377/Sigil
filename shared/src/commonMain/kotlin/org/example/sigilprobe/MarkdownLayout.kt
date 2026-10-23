package org.sigil

import androidx.compose.ui.text.font.FontWeight
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.Canvas
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.*
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.text.*
import androidx.compose.ui.unit.*
import androidx.compose.ui.graphics.takeOrElse

/** A block and the blocks nested directly inside it. */
internal class MarkdownNode(val block: RichBlock, val children: List<MarkdownNode>)

/** The block tree, or empty when one run of text serves better: no structure, or motion that runs across blocks. */
internal fun markdownTree(value: RichText): List<MarkdownNode> {
    val blocks = value.blocks.filter { it.start >= 0 && it.end <= value.text.length && it.start < it.end }
    if (blocks.isEmpty()) return emptyList()
    fun nodes(depth: Int, start: Int, end: Int): List<MarkdownNode> = blocks
        .filter { it.depth == depth && it.start >= start && it.end <= end }.sortedBy { it.start }
        .map { MarkdownNode(it, nodes(depth + 1, it.start, it.end)) }
    val top = nodes(blocks.minOf { it.depth }, 0, value.text.length)
    val crossing = value.motion.any { run ->
        val first = run.units.minOfOrNull { it.first } ?: return@any false
        val last = run.units.maxOf { it.second }
        top.none { first >= it.block.start && last <= it.block.end }
    }
    return if (crossing) emptyList() else top
}

/** What a list item shows before its text: a bullet, a number, or a task box. */
internal data class ItemMarker(val content: Int, val label: String, val task: Boolean?)

internal fun itemMarker(value: RichText, item: RichBlock, ordinal: Long?): ItemMarker {
    var at = item.start
    if (value.text.startsWith("• ", at)) at += 2
    val task = when {
        value.text.startsWith("[x] ", at) || value.text.startsWith("[X] ", at) -> true
        value.text.startsWith("[ ] ", at) -> false
        else -> null
    }
    if (task != null) at += 4
    val label = ordinal?.let { "$it." } ?: "•"
    return ItemMarker(at.coerceAtMost(item.end), label, task)
}

private sealed interface Piece
private class Prose(val value: RichText) : Piece
private class Block(val node: MarkdownNode) : Piece

/** Leaves carry no blocks: the layout draws the structure, so block styling must not apply twice. */
private fun leaf(value: RichText, start: Int, end: Int) = trimmedSlice(value, start, end)?.copy(blocks = emptyList())

private fun pieces(value: RichText, start: Int, end: Int, children: List<MarkdownNode>): List<Piece> = buildList {
    var at = start
    children.forEach { child ->
        val from = maxOf(child.block.start, start)
        if (from > at) leaf(value, at, from)?.let { add(Prose(it)) }
        add(Block(if (from == child.block.start) child else MarkdownNode(child.block.copy(start = from), child.children)))
        at = maxOf(at, child.block.end)
    }
    if (end > at) leaf(value, at, end)?.let { add(Prose(it)) }
}

private fun Piece.heading() = (this as? Block)?.node?.block?.takeIf { it.kind == "heading" }

// Text is trimmed to its glyphs, so these sit on top of the 8dp the line height would have added.
/** Vertical rhythm: 16dp between blocks, 24dp above a heading, 12dp below one, 8dp inside a list item. */
private fun gap(previous: Piece, next: Piece, tight: Boolean): Dp = when {
    next.heading() != null -> 24.dp
    previous.heading() != null -> 12.dp
    tight -> 8.dp
    else -> 16.dp
}

@Composable
internal fun headingStyle(level: Int, body: TextStyle): TextStyle {
    val type = MaterialTheme.typography
    // Bold and scaled as the reference sets them; italic stays for emphasis and quotes; h4 and below are small capitals.
    val base = when (level) { 1 -> type.headlineMedium; 2 -> type.titleLarge; 3 -> type.titleMedium; else -> type.labelSmall.copy(letterSpacing = 1.4.sp) }
    // Scales with the body it sits in, so a heading in a small card stays in proportion.
    val ratio = if (body.fontSize.isSp && type.bodyLarge.fontSize.isSp) body.fontSize.value / type.bodyLarge.fontSize.value else 1f
    return body.merge(base.copy(fontSize = base.fontSize * ratio, lineHeight = base.lineHeight * ratio, fontWeight = if (level <= 3) FontWeight.Bold else base.fontWeight))
}

/** Headings, lists, tasks, quotes and code set as a note; a code panel may stand in for the bubble. */
@Composable
internal fun MarkdownBody(value: RichText, nodes: List<MarkdownNode>, style: TextStyle, modifier: Modifier = Modifier) {
    val top = remember(value, nodes) { pieces(value, 0, value.text.length, nodes) }
    val panels = remember(value) { visibleCodeBlocks(value).toSet() }
    fun panel(piece: Piece) = piece is Block && piece.node.block in panels
    if (!(LocalMessageBubble.current && top.any(::panel))) {
        Flow(value, top, style, false, modifier, CodeBleed)
        return
    }
    // One bubble, no gaps: the panel runs edge to edge and the bubble's own shape rounds the outside.
    val groups = remember(top) {
        buildList<List<Piece>> {
            var run = mutableListOf<Piece>()
            top.forEach { if (panel(it)) { if (run.isNotEmpty()) add(run); add(listOf(it)); run = mutableListOf() } else run.add(it) }
            if (run.isNotEmpty()) add(run)
        }
    }
    Column(modifier) {
        groups.forEach { group ->
            val only = group.singleOrNull()?.takeIf { panel(it) } as? Block
            if (only != null) Code(value, only.node, Modifier.fillMaxWidth(), RectangleShape)
            else Box(Modifier.fillMaxWidth().background(LocalBubbleGround.current).padding(horizontal = 14.dp, vertical = 10.dp)) {
                Flow(value, group, style, false)
            }
        }
    }
}

@Composable
private fun Flow(value: RichText, pieces: List<Piece>, style: TextStyle, tight: Boolean, modifier: Modifier = Modifier, bleed: Dp = 0.dp) {
    Column(modifier) {
        pieces.forEachIndexed { index, piece ->
            if (index > 0) Spacer(Modifier.height(gap(pieces[index - 1], piece, tight)))
            when (piece) {
                is Prose -> RichInlineText(piece.value, style = style)
                is Block -> Node(value, piece.node, style, bleed)
            }
        }
    }
}

@Composable
private fun Node(value: RichText, node: MarkdownNode, style: TextStyle, bleed: Dp) {
    val block = node.block
    when (block.kind) {
        "heading" -> leaf(value, block.start, block.end)?.let {
            if (block.level <= 3) RichInlineText(it, Modifier.semantics { heading() }, headingStyle(block.level, style))
            else CompositionLocalProvider(LocalContentColor provides LocalContentColor.current.copy(alpha = .68f)) {
                RichInlineText(smallCapitals(it), Modifier.semantics { heading() }, headingStyle(block.level, style))
            }
        }
        // Paragraphs and lists inside a quote keep their own rhythm and marker column.
        "quote" -> QuoteRule { Flow(value, pieces(value, quoteStart(value, block), block.end, node.children), style.copy(fontStyle = FontStyle.Italic), false) }
        "code" -> if (block in visibleCodeBlocks(value)) Code(value, node, if (bleed > 0.dp) Modifier.bleed(bleed) else Modifier)
            else leaf(value, block.start, block.end)?.let { RichInlineText(it, style = style) }
        "list" -> ListBlock(value, node, style)
        else -> Flow(value, pieces(value, block.start, block.end, node.children), style, false)
    }
}

/** The first line keeps its `>` in the body; the rule stands in for it. */
private fun quoteStart(value: RichText, block: RichBlock): Int {
    var at = block.start
    if (value.text.startsWith(">", at)) at++
    if (value.text.startsWith(" ", at)) at++
    return at.coerceAtMost(block.end)
}

/** Upper case drawn over the same offsets; a letter whose capital is longer stays as written. */
internal fun smallCapitals(value: RichText) = value.copy(text = buildString(value.text.length) {
    value.text.forEach { c -> append(c.uppercase().singleOrNull() ?: c) }
})

@Composable
private fun QuoteRule(content: @Composable () -> Unit) {
    Row(Modifier.fillMaxWidth().height(IntrinsicSize.Min), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        Box(Modifier.width(3.dp).fillMaxHeight().clip(RoundedCornerShape(2.dp)).background(LocalContentColor.current.copy(alpha = .32f)))
        Box(Modifier.weight(1f)) { content() }
    }
}

@Composable
private fun Code(value: RichText, node: MarkdownNode, modifier: Modifier, shape: androidx.compose.ui.graphics.Shape = RoundedCornerShape(12.dp)) =
    CodeBlock(richSlice(value, node.block.start, trimmedEnd(value, node.block)), node.block.language, modifier, shape)

private fun listDepth(value: RichText, list: RichBlock) =
    value.blocks.count { it.kind == "list" && it !== list && it.start <= list.start && it.end >= list.end && it.depth < list.depth }

@Composable
private fun ListBlock(value: RichText, node: MarkdownNode, style: TextStyle) {
    val items = node.children.filter { it.block.kind == "item" }
    val depth = remember(value, node) { listDepth(value, node.block) }
    val markers = remember(value, node) { items.mapIndexed { index, item -> itemMarker(value, item.block, node.block.first?.plus(index)) } }
    val numeral = style.merge(TextStyle(fontFeatureSettings = "tnum, lnum"))
    val measurer = rememberTextMeasurer()
    val density = LocalDensity.current
    // One marker column per list, as wide as its widest marker, so every item's text starts on the same edge.
    val column = remember(markers, numeral, density) {
        val widest = markers.filter { it.task == null }.maxOfOrNull { measurer.measure(it.label, numeral).size.width }
            ?.let { with(density) { it.toDp() } } ?: 0.dp
        val box = if (markers.any { it.task != null }) 20.dp else 0.dp
        maxOf(widest, box, 10.dp) + if (box > 0.dp) 12.dp else 8.dp
    }
    val ink = LocalContentColor.current
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        items.forEachIndexed { index, item ->
            val marker = markers[index]
            val checked = marker.task == true
            Row(if (marker.task == null) Modifier else Modifier.semantics(mergeDescendants = true) {
                toggleableState = ToggleableState(checked); stateDescription = if (checked) "Done" else "Not done"
            }) {
                // A transparent line of the body type gives the marker the text's baseline; the mark centres on it.
                Box(Modifier.width(column).alignByBaseline().clearAndSetSemantics { if (node.block.first != null) contentDescription = marker.label },
                    contentAlignment = Alignment.CenterStart) {
                    Text(if (node.block.first != null) marker.label else " ", Modifier.width(column - 8.dp), style = numeral, maxLines = 1,
                        color = if (node.block.first != null && marker.task == null) ink.copy(alpha = .68f) else Color.Transparent, textAlign = TextAlign.End)
                    if (marker.task != null) TaskBox(checked)
                    else if (node.block.first == null) Bullet(depth, Modifier.width(column - 8.dp))
                }
                CompositionLocalProvider(LocalContentColor provides if (checked) ink.copy(alpha = .68f) else ink) {
                    Flow(value, pieces(value, marker.content, item.block.end, item.children), style, true, Modifier.weight(1f, fill = false).alignByBaseline())
                }
            }
        }
    }
}

/** Dot, ring, then square by nesting depth, drawn so no font has to carry the glyph. */
@Composable
private fun Bullet(depth: Int, modifier: Modifier) {
    val ink = LocalContentColor.current.copy(alpha = .68f)
    Box(modifier, contentAlignment = Alignment.Center) {
        Canvas(Modifier.size(6.dp)) {
            when (depth % 3) {
                0 -> drawCircle(ink)
                1 -> drawCircle(ink, radius = size.minDimension / 2 - .75.dp.toPx(), style = Stroke(1.5.dp.toPx()))
                else -> drawRect(ink, topLeft = Offset(.5.dp.toPx(), .5.dp.toPx()), size = Size(5.dp.toPx(), 5.dp.toPx()))
            }
        }
    }
}

/** A read-only task box: the sender's state, not a control; interactive lists are checklist cards. */
@Composable
private fun TaskBox(checked: Boolean) {
    val ink = LocalContentColor.current
    val shape = RoundedCornerShape(6.dp)
    Box(Modifier.size(20.dp).clip(shape).then(if (checked) Modifier.background(ink) else Modifier.border(1.5.dp, ink.copy(alpha = .55f), shape)),
        contentAlignment = Alignment.Center) {
        if (checked) CompositionLocalProvider(LocalContentColor provides LocalMessageSurface.current.takeOrElse { MaterialTheme.colorScheme.surface }) { Glyph("check", 16) }
    }
}
