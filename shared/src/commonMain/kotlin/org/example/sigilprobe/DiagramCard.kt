package org.sigil

import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.*
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.snapshots.Snapshot
import androidx.compose.ui.*
import androidx.compose.ui.draw.*
import androidx.compose.ui.geometry.*
import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.*
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.*
import androidx.compose.ui.window.*
import kotlin.math.*

// How long a diagram takes to draw itself in on arrival: up to eight 80ms steps, each settling over 360ms.
internal const val DiagramMotionMillis = 1000
private const val DiagramFill = .08f
private const val DiagramLine = .55f
private const val DiagramRail = .16f
private const val DiagramQuiet = .68f
private const val DiagramLevels = 6
private const val DiagramPerLevel = 3
private const val DiagramRows = 8
private const val DiagramEntries = 5
private val DiagramNodeWidth = 120.dp
private val DiagramNodeGap = 12.dp
private val DiagramCell = 88.dp
private val DiagramCorner = 8.dp
private val DiagramMarkRadius = 7.dp
// Space above the initial state for its dot and entry arrow, and below a final state for its exit.
private val DiagramMarkSpace = 32.dp

private fun DiagramContent.descendants(node: Int): Set<Int> {
    val seen = mutableSetOf<Int>()
    val queue = ArrayDeque<Int>(); queue.add(node)
    while (queue.isNotEmpty()) { val at = queue.removeFirst(); if (seen.add(at)) edges.filter { it.from == at }.forEach { queue.add(it.to) } }
    return seen
}

internal fun diagramKindWord(kind: String) = when (kind) { "org" -> "Org chart"; "flow" -> "Flowchart"; "sequence" -> "Sequence diagram"; "timeline" -> "Timeline"; "mindmap" -> "Mind map"; "state" -> "State diagram"; else -> "Diagram" }

// The whole card reads as one sentence: kind, title, then every relationship in order.
internal fun DiagramContent.summary(): String = buildString {
    append(diagramKindWord(kind)); if (title.text.isNotBlank()) append(". ").append(title.text)
    fun name(i: Int) = nodes.getOrNull(i)?.label?.text.orEmpty()
    when (kind) {
        "timeline" -> entries.forEach { append(". ").append(it.date.text).append(": ").append(it.label.text) }
        "mindmap" -> { val tree = mindTree(); append(". ").append(topicDescription(tree, tree.root))
            tree.reading().filter { tree.children[it].isNotEmpty() }.forEach { n -> append(". ").append(name(n)).append(": ").append(tree.children[n].joinToString(", ") { name(it) }) } }
        "org" -> edges.groupBy { it.from }.forEach { (from, out) -> append(". ").append(name(from)).append(": ").append(out.joinToString(", ") { name(it.to) }) }
        else -> edges.forEach { edge ->
            append(". ").append(name(edge.from)).append(" to ").append(name(edge.to))
            if (edge.label.text.isNotBlank()) append(if (kind == "sequence") ": " else ", ").append(edge.label.text)
        }
    }
}

internal class DiagramGraph(val levels: List<List<Int>>, val level: IntArray, val slot: IntArray)
internal class DiagramLane(val edge: Int, val right: Boolean, val index: Int)

// Rows come from the core's layout: its y picks the level and its x the order within it.
private fun DiagramContent.graph(): DiagramGraph {
    val raw = nodes.map { ((it.y - 24f) / 144f).roundToInt().coerceAtLeast(0) }
    val levels = raw.distinct().sorted().map { l -> nodes.indices.filter { raw[it] == l }.sortedBy { nodes[it].x } }
    val level = IntArray(nodes.size); val slot = IntArray(nodes.size)
    levels.forEachIndexed { k, row -> row.forEachIndexed { i, n -> level[n] = k; slot[n] = i } }
    return DiagramGraph(levels, level, slot)
}

// Loops and long jumps ride a lane beside the stack, alternating sides; null when one cannot be drawn cleanly.
private fun DiagramContent.lanes(graph: DiagramGraph): List<DiagramLane>? {
    val pending = edges.indices.filter { graph.level[edges[it].to] != graph.level[edges[it].from] + 1 }
        .sortedBy { abs(graph.level[edges[it].to] - graph.level[edges[it].from]) }
    val sides = listOf(mutableListOf<MutableList<IntRange>>(), mutableListOf())
    val out = mutableListOf<DiagramLane>()
    for (index in pending) {
        val edge = edges[index]; val a = graph.level[edge.from]; val b = graph.level[edge.to]
        if (a == b && edge.from != edge.to) return null
        val span = min(a, b)..max(a, b)
        val choices = listOfNotNull(true.takeIf { graph.slot[edge.from] == graph.levels[a].lastIndex && graph.slot[edge.to] == graph.levels[b].lastIndex },
            false.takeIf { graph.slot[edge.from] == 0 && graph.slot[edge.to] == 0 }).sortedBy { sides[if (it) 1 else 0].size }
        choices.firstOrNull { right ->
            val lanes = sides[if (right) 1 else 0]
            val free = lanes.indexOfFirst { lane -> lane.none { it.first <= span.last && span.first <= it.last } }.let { if (it < 0) lanes.size else it }
            if (free > 1) false else { if (free == lanes.size) lanes.add(mutableListOf()); lanes[free].add(span); out.add(DiagramLane(index, right, free)); true }
        } ?: return null
    }
    return out
}

private fun diagramReveal(elapsed: Float, step: Int) = MotionStandardEasing.transform(((elapsed - step.coerceIn(0, 8) * MotionStagger) / MotionSettle).coerceIn(0f, 1f))

@Composable
private fun diagramElapsed(): () -> Float {
    val clock = LocalTextMotion.current?.clock.takeIf { !LocalMotion.current.reduced && LocalAppearance.current.messageEffects }
    return remember(clock) { { clock?.elapsed ?: Float.MAX_VALUE } }
}

private fun roundedPolyline(points: List<Offset>, radius: Float, sharp: Set<Int> = emptySet()) = Path().apply {
    moveTo(points[0].x, points[0].y)
    for (i in 1 until points.lastIndex) {
        val at = points[i]; val before = points[i - 1] - at; val after = points[i + 1] - at
        if (i in sharp) { lineTo(at.x, at.y); continue }
        val r = minOf(radius, before.getDistance() / 2, after.getDistance() / 2)
        val entry = at + before / before.getDistance().coerceAtLeast(.001f) * r; val exit = at + after / after.getDistance().coerceAtLeast(.001f) * r
        lineTo(entry.x, entry.y); quadraticTo(at.x, at.y, exit.x, exit.y)
    }
    lineTo(points.last().x, points.last().y)
}

// Sharp corners are junctions shared with a sibling connector, kept square so they read as a tee; an arc's four points are Bézier controls.
internal data class DiagramStroke(val points: List<Offset>, val arrow: Boolean, val dashed: Boolean, val step: Int, val sharp: Set<Int> = emptySet(),
    val arc: Boolean = false, val corner: Dp = DiagramCorner, val color: Color = Color.Unspecified, val width: Dp = 1.5.dp)

// A state or pseudo-state marker: the initial dot, or the final bullseye.
internal data class DiagramMark(val center: Offset, val final: Boolean, val step: Int)

// Connector geometry settled in layout and read by the same frame's draw, so lines never lag their tiles.
private class DiagramInk {
    var strokes = emptyList<DiagramStroke>(); private set
    var lifelines = emptyList<Pair<Offset, Offset>>(); private set
    var marks = emptyList<DiagramMark>(); private set
    val tick = mutableIntStateOf(0)
    fun set(strokes: List<DiagramStroke>, lifelines: List<Pair<Offset, Offset>> = emptyList(), marks: List<DiagramMark> = emptyList()) {
        if (strokes == this.strokes && lifelines == this.lifelines && marks == this.marks) return
        val redraw = this.strokes.isNotEmpty() || this.lifelines.isNotEmpty()
        this.strokes = strokes; this.lifelines = lifelines; this.marks = marks
        if (redraw) Snapshot.withoutReadObservation { tick.intValue++ }
    }
}

// Draws each connector trimmed to its reveal, with a filled head once the line lands.
private fun androidx.compose.ui.graphics.drawscope.DrawScope.drawStrokes(strokes: List<DiagramStroke>, color: Color, elapsed: Float) {
    val head = 7.dp.toPx(); val half = 3.5.dp.toPx()
    val dash = PathEffect.dashPathEffect(floatArrayOf(4.dp.toPx(), 4.dp.toPx()))
    val measure = PathMeasure()
    for (stroke in strokes) {
        val t = diagramReveal(elapsed, stroke.step)
        if (t <= 0f || stroke.points.size < 2) continue
        val tint = stroke.color.takeOrElse { color }
        val end = stroke.points.last(); val from = stroke.points[stroke.points.lastIndex - 1]
        val direction = (end - from) / (end - from).getDistance().coerceAtLeast(.001f)
        val trimmed = if (stroke.arrow) stroke.points.dropLast(1) + (end - direction * (head - 1f)) else stroke.points
        var path = if (stroke.arc && trimmed.size == 4) Path().apply { moveTo(trimmed[0].x, trimmed[0].y); cubicTo(trimmed[1].x, trimmed[1].y, trimmed[2].x, trimmed[2].y, trimmed[3].x, trimmed[3].y) }
            else roundedPolyline(trimmed, stroke.corner.toPx(), stroke.sharp)
        if (t < 1f) { measure.setPath(path, false); path = Path().also { measure.getSegment(0f, measure.length * t, it, true) } }
        drawPath(path, tint, style = Stroke(stroke.width.toPx(), cap = StrokeCap.Round, join = StrokeJoin.Round, pathEffect = if (stroke.dashed) dash else null))
        if (stroke.arrow && t > .85f) {
            val normal = Offset(-direction.y, direction.x); val base = end - direction * head
            drawPath(Path().apply { moveTo(end.x, end.y); lineTo(base.x + normal.x * half, base.y + normal.y * half); lineTo(base.x - normal.x * half, base.y - normal.y * half); close() }, tint, alpha = ((t - .85f) / .15f).coerceIn(0f, 1f))
        }
    }
}

// The initial state is a filled ink dot; a final state's exit lands on a ringed dot.
private fun androidx.compose.ui.graphics.drawscope.DrawScope.drawMarks(marks: List<DiagramMark>, ink: Color, elapsed: Float) {
    for (mark in marks) {
        val t = diagramReveal(elapsed, mark.step)
        if (t <= 0f) continue
        if (mark.final) { drawCircle(ink, DiagramMarkRadius.toPx() * t, mark.center, style = Stroke(1.5.dp.toPx())); drawCircle(ink, 4.dp.toPx() * t, mark.center) }
        else drawCircle(ink, 5.dp.toPx() * t, mark.center)
    }
}

@Composable
internal fun DiagramCard(diagram: DiagramContent) {
    val elapsed = diagramElapsed()
    val summary = remember(diagram) { diagram.summary() }
    Column(Modifier.widthIn(min = MessageCardMinWidth, max = MessageCardMaxWidth).fillMaxWidth().padding(vertical = 4.dp)
        .animateContentSize(LocalMotion.current.tween(MotionMillis)).clearAndSetSemantics { contentDescription = summary }, verticalArrangement = Arrangement.spacedBy(12.dp)) {
        if (diagram.title.text.isNotBlank()) DiagramText(diagram.title, MaterialTheme.typography.titleMedium, 3)
        BoxWithConstraints(Modifier.fillMaxWidth()) { val width = maxWidth; Column {
            when (diagram.kind) {
                "timeline" -> DiagramTimeline(diagram, elapsed)
                "mindmap" -> DiagramMindMap(diagram, elapsed)
                "sequence" -> if (diagram.nodes.size in 1..4 && width / diagram.nodes.size >= 72.dp) DiagramSequence(diagram, elapsed) else DiagramTransitions(diagram, elapsed)
                else -> {
                    val plan = rememberLayerPlan(diagram, width)
                    var tangled by remember(diagram, width) { mutableStateOf(false) }
                    if (plan != null && !tangled) DiagramLayers(diagram, plan, elapsed) { tangled = true }
                    else if (diagram.kind == "org") DiagramBranches(diagram, elapsed)
                    else DiagramTransitions(diagram, elapsed)
                }
            }
        } }
    }
}

@Composable
private fun DiagramMore(count: Int) {
    if (count > 0) Text("+$count more", Modifier.padding(top = 8.dp), style = MaterialTheme.typography.labelMedium, color = LocalContentColor.current.copy(alpha = DiagramQuiet), maxLines = 1)
}

@Composable
private fun groundColor() = LocalMessageSurface.current.takeOrElse { MaterialTheme.colorScheme.surface }

// Caps a label at a few lines of its style so one long name cannot stretch the diagram.
@Composable
private fun Modifier.lines(style: TextStyle, count: Int) = heightIn(max = with(LocalDensity.current) { style.lineHeight.toDp() } * count).clipToBounds()

// Plain labels end in an ellipsis at their cap; styled ones clip, as rich text has no line limit.
@Composable
private fun DiagramText(value: RichText, style: TextStyle, lines: Int, modifier: Modifier = Modifier) {
    if (value.spans.isEmpty() && value.blocks.isEmpty() && value.motion.isEmpty()) Text(value.text, modifier, style = style, maxLines = lines, overflow = TextOverflow.Ellipsis)
    else RichMessageText(value, modifier.lines(style, lines), style)
}

// A connector label sits on the bubble's own ground so the line it names breaks around it.
@Composable
private fun DiagramChip(label: RichText, ground: Color) {
    val style = MaterialTheme.typography.labelMedium.copy(textAlign = TextAlign.Center)
    Box(Modifier.background(ground, RoundedCornerShape(6.dp)).padding(horizontal = 6.dp, vertical = 2.dp)) {
        CompositionLocalProvider(LocalContentColor provides LocalContentColor.current.copy(alpha = DiagramQuiet)) {
            DiagramText(label, style, 2)
        }
    }
}

internal class LayerPlan(val graph: DiagramGraph, val lanes: List<DiagramLane>, val offsets: Map<Pair<Boolean, Int>, Dp>, val gutter: Dp)

// Rows, side lanes and their gutter for a width; null when the graph is too wide or tangled to draw in rows.
internal fun DiagramContent.layerPlan(width: Dp, chipWidth: (RichText) -> Dp): LayerPlan? {
    if (nodes.isEmpty()) return null
    val graph = graph()
    if (graph.levels.size > DiagramLevels || graph.levels.maxOf { it.size } > DiagramPerLevel) return null
    val lanes = lanes(graph) ?: return null
    fun chip(edge: Int) = edges[edge].label.takeIf { it.text.isNotBlank() }?.let { min(chipWidth(it), 148.dp) + 12.dp } ?: 0.dp
    val offsets = mutableMapOf<Pair<Boolean, Int>, Dp>()
    var gutter = 0.dp
    for (right in listOf(false, true)) {
        var offset = 0.dp; var previous = 0.dp
        val count = lanes.filter { it.right == right }.maxOfOrNull { it.index + 1 } ?: 0
        for (j in 0 until count) {
            val half = lanes.filter { it.right == right && it.index == j }.maxOf { chip(it.edge) } / 2
            offset = if (j == 0) max(16.dp, 10.dp + half) else offset + max(14.dp, previous + half + 8.dp)
            offsets[right to j] = offset; previous = half
            if (j == count - 1) gutter = max(gutter, offset + max(half, 4.dp))
        }
    }
    val per = graph.levels.maxOf { it.size }
    val cell = (width - gutter * 2 - DiagramNodeGap * (per - 1)) / per
    return if (cell < DiagramCell) null else LayerPlan(graph, lanes, offsets, gutter)
}

@Composable
private fun rememberLayerPlan(diagram: DiagramContent, width: Dp): LayerPlan? {
    val measurer = rememberTextMeasurer()
    val style = MaterialTheme.typography.labelMedium
    val density = LocalDensity.current
    return remember(diagram, width, style, density) { diagram.layerPlan(width) { with(density) { measurer.measure(it.text, style).size.width.toDp() } } }
}

internal class LayerGeometry(val height: Int, val tiles: List<Rect>, val chips: Map<Int, Rect>, val strokes: List<DiagramStroke>, val marks: List<DiagramMark> = emptyList())

// A state machine enters at its first source-less state, else its first state; it ends at states with no way out.
internal fun DiagramContent.initialState() = if (kind != "state") null else nodes.indices.firstOrNull { n -> edges.none { it.to == n } } ?: 0
internal fun DiagramContent.finalStates() = if (kind != "state") emptyList() else nodes.indices.filter { n -> edges.none { it.from == n } }

// Places tiles, connectors and their labels in pixels; null when a label or bus would be misread as another's.
internal fun Density.layerGeometry(diagram: DiagramContent, plan: LayerPlan, width: Int, tiles: List<IntSize>, chips: Map<Int, IntSize>): LayerGeometry? {
    val graph = plan.graph; val edges = diagram.edges
    val laneOf = plan.lanes.associateBy { it.edge }
    val gutter = plan.gutter.roundToPx(); val gap = DiagramNodeGap.roundToPx(); val area = width - gutter * 2
    val band = edges.indices.filter { it !in laneOf }
    val out = IntArray(diagram.nodes.size); val into = IntArray(diagram.nodes.size)
    band.forEach { out[edges[it].from]++; into[edges[it].to]++ }
    // A label rides its target's drop unless that drop is shared, then its source's; a source shared too has no segment of its own.
    val onSource = band.filter { it in chips && into[edges[it].to] > 1 }.toSet()
    if (onSource.any { out[edges[it].from] > 1 }) return null
    val levels = graph.levels.size
    val upper = IntArray(levels); val lower = IntArray(levels)
    band.filter { it in chips }.forEach { i -> val k = graph.level[edges[i].from]; val h = chips.getValue(i).height; if (i in onSource) upper[k] = max(upper[k], h) else lower[k] = max(lower[k], h) }
    val tops = IntArray(levels); val heights = IntArray(levels); val buses = IntArray(levels)
    val initial = diagram.initialState()?.takeIf { graph.level[it] == 0 }
    val finals = diagram.finalStates()
    val space = DiagramMarkSpace.roundToPx()
    // Any row holding a final state keeps room beneath it for the bullseye, above that row's bus.
    val marked = IntArray(levels).also { m -> finals.forEach { m[graph.level[it]] = space } }
    var y = if (initial != null) space else 0
    graph.levels.forEachIndexed { k, row ->
        tops[k] = y; heights[k] = row.maxOf { tiles[it].height }
        val labelled = upper[k] > 0 || lower[k] > 0
        val drop = when { upper[k] > 0 -> upper[k] + 16.dp.roundToPx(); labelled -> 12.dp.roundToPx(); else -> 16.dp.roundToPx() }
        buses[k] = y + heights[k] + marked[k] + drop
        y += heights[k] + marked[k] + drop + if (labelled) lower[k] + 20.dp.roundToPx() else 16.dp.roundToPx()
    }
    val height = tops.last() + heights.last() + marked.last()
    val rects = MutableList(diagram.nodes.size) { Rect.Zero }
    graph.levels.forEachIndexed { k, row ->
        val span = row.sumOf { tiles[it].width } + (row.size - 1) * gap
        var x = gutter + (area - span) / 2
        row.forEach { n -> rects[n] = Rect(Offset(x.toFloat(), (tops[k] + (heights[k] - tiles[n].height) / 2).toFloat()), Size(tiles[n].width.toFloat(), tiles[n].height.toFloat())); x += tiles[n].width + gap }
    }
    // Two parents' buses on one row may not meet unless they share a child: that reads as one family.
    for (i in band) for (j in band) if (i < j && graph.level[edges[i].from] == graph.level[edges[j].from] && edges[i].from != edges[j].from && edges[i].to != edges[j].to) {
        fun span(e: Int) = rects[edges[e].from].center.x.let { a -> rects[edges[e].to].center.x.let { b -> min(a, b)..max(a, b) } }
        val a = span(i); val b = span(j)
        if (max(a.start, b.start) <= min(a.endInclusive, b.endInclusive) + 1f) return null
    }
    val left = rects.minOf { it.left }; val right = rects.maxOf { it.right }
    val arrows = diagram.kind != "org"
    val state = diagram.kind == "state"
    // Flows turn square corners; transitions between states bend in soft curves.
    val corner = if (state) 20.dp else 4.dp
    val lastStep = (levels - 1) * 2 + 1
    val anchors = mutableMapOf<Int, Offset>()
    val strokes = edges.mapIndexed { i, edge ->
        val from = rects[edge.from]; val to = rects[edge.to]
        val lane = laneOf[i]
        if (lane == null) {
            val bus = buses[graph.level[edge.from]].toFloat()
            if (i in chips) anchors[i] = if (i in onSource) Offset(from.center.x, (tops[graph.level[edge.from]] + heights[graph.level[edge.from]] + marked[graph.level[edge.from]] + bus) / 2) else Offset(to.center.x, (bus + to.top) / 2)
            if (abs(from.center.x - to.center.x) < 1f) DiagramStroke(listOf(Offset(from.center.x, from.bottom), Offset(to.center.x, to.top)), arrows, edge.dashed, graph.level[edge.to] * 2 - 1)
            else DiagramStroke(listOf(Offset(from.center.x, from.bottom), Offset(from.center.x, bus), Offset(to.center.x, bus), Offset(to.center.x, to.top)), arrows, edge.dashed,
                graph.level[edge.to] * 2 - 1, setOfNotNull(1.takeIf { out[edge.from] > 1 }, 2.takeIf { into[edge.to] > 1 }), corner = corner)
        } else {
            val offset = plan.offsets.getValue(lane.right to lane.index).toPx()
            val x = if (lane.right) right + offset else left - offset
            val fromX = if (lane.right) from.right else from.left; val toX = if (lane.right) to.right else to.left
            val loop = edge.from == edge.to
            val fromY = if (loop) from.center.y - 8.dp.toPx() else from.center.y; val toY = if (loop) to.center.y + 8.dp.toPx() else to.center.y
            if (i in chips) anchors[i] = Offset(x, (fromY + toY) / 2)
            // A state's return or self-transition is one arc whose apex meets the lane, so its label still sits on it.
            // A self-transition is a round loop tall enough to show above and below its label.
            if (state && loop) { val a = from.center.y - 14.dp.toPx(); val b = from.center.y + 14.dp.toPx(); val c = fromX + (x - fromX) * 4 / 3; val k = 10.dp.toPx()
                DiagramStroke(listOf(Offset(fromX, a), Offset(c, a - k), Offset(c, b + k), Offset(fromX, b)), true, edge.dashed, lastStep + 1, arc = true) }
            else if (state) DiagramStroke(listOf(Offset(fromX, fromY), Offset(fromX + (x - fromX) * 4 / 3, fromY), Offset(toX + (x - toX) * 4 / 3, toY), Offset(toX, toY)), true, edge.dashed, lastStep + 1, arc = true)
            else DiagramStroke(listOf(Offset(fromX, fromY), Offset(x, fromY), Offset(x, toY), Offset(toX, toY)), true, edge.dashed, lastStep + 1, corner = corner)
        }
    }
    val marks = mutableListOf<DiagramMark>(); val entries = mutableListOf<DiagramStroke>()
    initial?.let { n -> val tile = rects[n]; val dot = Offset(tile.center.x, tile.top - space + 7.dp.toPx())
        marks += DiagramMark(dot, false, 0); entries += DiagramStroke(listOf(dot + Offset(0f, 5.dp.toPx()), Offset(tile.center.x, tile.top)), true, false, 0) }
    finals.forEach { n -> val tile = rects[n]; val ring = Offset(tile.center.x, tile.bottom + space - DiagramMarkRadius.toPx() - 1.dp.toPx())
        marks += DiagramMark(ring, true, lastStep + 1); entries += DiagramStroke(listOf(Offset(tile.center.x, tile.bottom), ring - Offset(0f, DiagramMarkRadius.toPx() + 1.dp.toPx())), true, false, lastStep) }
    val placed = anchors.mapValues { (i, c) ->
        val size = chips.getValue(i)
        Rect(Offset((c.x - size.width / 2f).roundToInt().coerceIn(0, max(0, width - size.width)).toFloat(), (c.y - size.height / 2f).roundToInt().toFloat()), Size(size.width.toFloat(), size.height.toFloat()))
    }
    val margin = 2.dp.toPx()
    val list = placed.entries.toList()
    for (a in list.indices) for (b in a + 1 until list.size) if (list[a].value.inflate(margin).overlaps(list[b].value)) return null
    // A label may only break its own line, never a neighbour's drop.
    for ((i, rect) in placed) for (j in band) if (j != i) strokes[j].points.zipWithNext().forEach { (p, q) ->
        if (abs(p.x - q.x) < 1f && abs(p.x - anchors.getValue(i).x) >= 1f && p.x > rect.left - margin && p.x < rect.right + margin && max(p.y, q.y) > rect.top && min(p.y, q.y) < rect.bottom) return null
    }
    return LayerGeometry(height, rects, placed, strokes + entries, marks)
}

// Org charts, flows and state machines: tonal tiles in rows, elbows between rows, loops in side lanes.
@Composable
private fun DiagramLayers(diagram: DiagramContent, plan: LayerPlan, elapsed: () -> Float, tangle: () -> Unit) {
    val ink = LocalContentColor.current
    val ground = groundColor()
    val fill = lerp(ground, ink, DiagramFill)
    val line = lerp(ground, ink, DiagramLine)
    val graph = plan.graph
    val labeled = remember(diagram) { diagram.edges.indices.filter { diagram.edges[it].label.text.isNotBlank() } }
    val laneOf = remember(plan) { plan.lanes.associateBy { it.edge } }
    val drawn = remember(diagram) { DiagramInk() }
    val rise = with(LocalDensity.current) { 6.dp.toPx() }
    val lastStep = (graph.levels.size - 1) * 2 + 1
    Layout(content = {
        diagram.nodes.forEach { DiagramTile(it, diagram.kind, fill) }
        labeled.forEach { DiagramChip(diagram.edges[it].label, ground) }
    }, modifier = Modifier.fillMaxWidth().drawBehind { drawn.tick.intValue; drawStrokes(drawn.strokes, line, elapsed()); drawMarks(drawn.marks, ink, elapsed()) }) { measurables, constraints ->
        val width = constraints.maxWidth
        val gap = DiagramNodeGap.roundToPx()
        val per = graph.levels.maxOf { it.size }
        val cell = (width - plan.gutter.roundToPx() * 2 - gap * (per - 1)) / per
        val nodes = measurables.take(diagram.nodes.size)
        val natural = nodes.maxOf { it.maxIntrinsicWidth(Constraints.Infinity) }
        val nodeWidth = natural.coerceIn(min(DiagramNodeWidth.roundToPx(), cell), cell)
        val tiles = nodes.map { it.measure(Constraints.fixedWidth(nodeWidth)) }
        val chips = measurables.drop(diagram.nodes.size).map { it.measure(Constraints(maxWidth = min(160.dp.roundToPx(), width))) }
        val geometry = layerGeometry(diagram, plan, width, tiles.map { IntSize(it.width, it.height) }, labeled.withIndex().associate { (k, edge) -> edge to IntSize(chips[k].width, chips[k].height) })
        if (geometry == null) { tangle(); return@Layout layout(width, 0) {} }
        drawn.set(geometry.strokes, marks = geometry.marks)
        layout(width, geometry.height) {
            tiles.forEachIndexed { n, tile ->
                val step = graph.level[n] * 2
                tile.placeWithLayer(geometry.tiles[n].left.roundToInt(), geometry.tiles[n].top.roundToInt()) {
                    val t = diagramReveal(elapsed(), step); alpha = t; translationY = rise * (1f - t); scaleX = .94f + .06f * t; scaleY = scaleX
                }
            }
            labeled.forEachIndexed { k, edge ->
                val rect = geometry.chips[edge] ?: return@forEachIndexed
                val step = laneOf[edge]?.let { lastStep + 1 } ?: (graph.level[diagram.edges[edge].to] * 2 - 1)
                chips[k].placeWithLayer(rect.left.roundToInt(), rect.top.roundToInt()) { alpha = diagramReveal(elapsed(), step + 1) }
            }
        }
    }
}

private val DiamondShape = GenericShape { size, _ -> moveTo(size.width / 2, 0f); lineTo(size.width, size.height / 2); lineTo(size.width / 2, size.height); lineTo(0f, size.height / 2); close() }

// Terminals are pills, flow steps are square-cornered boxes, states and org roles are rounded tiles, decisions are diamonds sized so their words sit inside.
internal fun diagramNodeShape(kind: String, shape: String): Shape = when {
    shape == "decision" -> DiamondShape
    shape == "rounded" -> RoundedCornerShape(50)
    kind == "flow" || shape == "process" -> RoundedCornerShape(4.dp)
    else -> RoundedCornerShape(14.dp)
}

@Composable
private fun DiagramTile(node: DiagramNode, kind: String, fill: Color) {
    val style = MaterialTheme.typography.bodyMedium.copy(textAlign = TextAlign.Center)
    if (node.shape == "decision") {
        Layout(content = { DiagramText(node.label, style, 3) }, modifier = Modifier.background(fill, DiamondShape), measurePolicy = object : MeasurePolicy {
            override fun MeasureScope.measure(measurables: List<Measurable>, constraints: Constraints): MeasureResult {
                val width = if (constraints.hasBoundedWidth) constraints.maxWidth else measurables[0].maxIntrinsicWidth(Constraints.Infinity) * 2 + 16.dp.roundToPx()
                val text = measurables[0].measure(Constraints(maxWidth = max(1, width / 2 + 8.dp.roundToPx())))
                val height = max(56.dp.roundToPx(), (text.height / (1f - (text.width + 8.dp.toPx()) / width).coerceAtLeast(.34f)).roundToInt() + 8.dp.roundToPx())
                return layout(width, height) { text.place((width - text.width) / 2, (height - text.height) / 2) }
            }
            override fun IntrinsicMeasureScope.maxIntrinsicWidth(measurables: List<IntrinsicMeasurable>, height: Int) = measurables[0].maxIntrinsicWidth(height) * 2 + 16.dp.roundToPx()
            override fun IntrinsicMeasureScope.minIntrinsicWidth(measurables: List<IntrinsicMeasurable>, height: Int) = measurables[0].minIntrinsicWidth(height) * 2 + 16.dp.roundToPx()
        })
        return
    }
    val pill = node.shape == "rounded"
    Box(Modifier.background(fill, diagramNodeShape(kind, node.shape)).heightIn(min = 44.dp).padding(horizontal = if (pill) 16.dp else 12.dp, vertical = 10.dp), contentAlignment = Alignment.Center) {
        DiagramText(node.label, style, 3)
    }
}

// Lifelines across the top, each message an arrow in reading order with its words above it.
@Composable
private fun DiagramSequence(diagram: DiagramContent, elapsed: () -> Float) {
    val ink = LocalContentColor.current
    val ground = groundColor()
    val fill = lerp(ground, ink, DiagramFill)
    val line = lerp(ground, ink, DiagramLine)
    val rail = lerp(ground, ink, DiagramRail)
    val shown = diagram.edges.take(DiagramRows)
    val drawn = remember(diagram) { DiagramInk() }
    Layout(content = {
        diagram.nodes.forEach { node ->
            Box(Modifier.background(fill, RoundedCornerShape(10.dp)).padding(horizontal = 8.dp, vertical = 6.dp), contentAlignment = Alignment.Center) {
                val style = MaterialTheme.typography.bodyMedium.copy(textAlign = TextAlign.Center)
                DiagramText(node.label, style, 3)
            }
        }
        shown.forEach { edge -> DiagramChip(edge.label, ground) }
    }, modifier = Modifier.fillMaxWidth().drawBehind {
        drawn.tick.intValue
        val grow = diagramReveal(elapsed(), 0)
        drawn.lifelines.forEach { (top, bottom) -> drawLine(rail, top, Offset(top.x, top.y + (bottom.y - top.y) * grow), 1.dp.toPx()) }
        drawStrokes(drawn.strokes, line, elapsed())
    }) { measurables, constraints ->
        val width = constraints.maxWidth
        val count = diagram.nodes.size
        val column = width / count.toFloat()
        val centers = List(count) { column * (it + .5f) }
        val heads = measurables.take(count).map { it.measure(Constraints(maxWidth = (column - 8.dp.toPx()).roundToInt().coerceAtLeast(1))) }
        val headHeight = heads.maxOf { it.height }
        var y = headHeight + 16.dp.toPx()
        val labels = mutableListOf<Pair<Placeable, IntOffset>>()
        val arrows = shown.mapIndexed { j, edge ->
            val from = centers[edge.from]; val to = centers[edge.to]
            val loop = edge.from == edge.to
            val room = if (loop) width - from - 12.dp.toPx() else max(abs(to - from), column) - 12.dp.toPx()
            val label = measurables[count + j].measure(Constraints(maxWidth = room.roundToInt().coerceAtLeast(1)))
            val words = edge.label.text.isNotBlank()
            val x = if (loop) from + 6.dp.toPx() else (from + to) / 2 - label.width / 2f
            if (words) labels += label to IntOffset(x.roundToInt().coerceIn(0, max(0, width - label.width)), y.roundToInt())
            val arrow = y + (if (words) label.height + 4.dp.toPx() else 4.dp.toPx())
            val points = if (loop) listOf(Offset(from, arrow), Offset(from + 24.dp.toPx(), arrow), Offset(from + 24.dp.toPx(), arrow + 14.dp.toPx()), Offset(from, arrow + 14.dp.toPx()))
                else listOf(Offset(from, arrow), Offset(to, arrow))
            y = points.maxOf { it.y } + 16.dp.toPx()
            DiagramStroke(points, true, edge.dashed, j + 1)
        }
        val height = (y - 4.dp.toPx()).roundToInt()
        val lines = centers.map { Offset(it, headHeight.toFloat()) to Offset(it, height.toFloat()) }
        drawn.set(arrows, lines)
        layout(width, height) {
            heads.forEachIndexed { i, head -> head.placeWithLayer((centers[i] - head.width / 2f).roundToInt(), headHeight - head.height) { alpha = diagramReveal(elapsed(), 0) } }
            val steps = shown.indices.filter { shown[it].label.text.isNotBlank() }
            labels.forEachIndexed { k, (label, at) -> label.placeWithLayer(at) { alpha = diagramReveal(elapsed(), steps[k] + 1) } }
        }
    }
    DiagramMore(diagram.edges.size - shown.size)
}

// Dated entries on a rail: the date as quiet meta, the event in body text.
@Composable
private fun DiagramTimeline(diagram: DiagramContent, elapsed: () -> Float) {
    val ink = LocalContentColor.current
    val ground = groundColor()
    val rail = lerp(ground, ink, DiagramRail)
    val shown = diagram.entries.take(DiagramEntries)
    val meta = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum, lnum")
    val measurer = rememberTextMeasurer()
    val density = LocalDensity.current
    // Dates are lining figures: the dot sits at half their cap height above the baseline.
    val dotY = remember(meta, density) { measurer.measure("0", meta).firstBaseline - with(density) { meta.fontSize.toPx() } * .35f }
    val rise = with(LocalDensity.current) { 6.dp.toPx() }
    Column {
        shown.forEachIndexed { index, entry ->
            val last = index == shown.lastIndex
            Row(Modifier.fillMaxWidth().drawBehind {
                val t = diagramReveal(elapsed(), index)
                val x = 12.dp.toPx()
                if (!last) drawLine(rail, Offset(x, dotY), Offset(x, dotY + size.height * diagramReveal(elapsed(), index + 1)), 2.dp.toPx(), StrokeCap.Round)
                drawCircle(ink, 4.dp.toPx() * t, Offset(x, dotY))
            }.padding(bottom = if (last) 0.dp else 16.dp).graphicsLayer { val t = diagramReveal(elapsed(), index); alpha = t; translationY = rise * (1f - t) }) {
                Spacer(Modifier.width(36.dp))
                Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    CompositionLocalProvider(LocalContentColor provides ink.copy(alpha = DiagramQuiet)) { RichMessageText(entry.date, style = meta) }
                    DiagramText(entry.label, MaterialTheme.typography.bodyMedium, 3)
                }
            }
        }
    }
    DiagramMore(diagram.entries.size - shown.size)
}

// A tree read top to bottom: each child hangs from its parent's dot, one indent deeper.
private fun DiagramContent.branchRows(): List<Triple<Int, Int, RichText>> {
    val children = nodes.indices.map { from -> edges.filter { it.from == from }.map { it.to to it.label } }
    val entering = edges.map { it.to }.toSet()
    val roots = nodes.indices.filter { it !in entering }.ifEmpty { listOf(0) }
    val seen = mutableSetOf<Int>()
    val rows = mutableListOf<Triple<Int, Int, RichText>>()
    fun walk(node: Int, depth: Int, label: RichText) {
        if (!seen.add(node)) return
        rows += Triple(node, depth, label)
        children[node].forEach { (child, edge) -> walk(child, depth + 1, edge) }
    }
    roots.forEach { walk(it, 0, RichText("")) }
    nodes.indices.forEach { walk(it, 0, RichText("")) }
    return rows
}

@Composable
private fun DiagramBranches(diagram: DiagramContent, elapsed: () -> Float) {
    val ink = LocalContentColor.current
    val ground = groundColor()
    val line = lerp(ground, ink, DiagramLine)
    val all = remember(diagram) { diagram.branchRows() }
    val shown = all.take(DiagramRows)
    val body = MaterialTheme.typography.bodyMedium
    val density = LocalDensity.current
    val measurer = rememberTextMeasurer()
    // Dots and elbows meet the name at its x-height middle, not the line box's.
    val centerY = remember(body, density) { with(density) { 6.dp.toPx() + measurer.measure("x", body).firstBaseline - body.fontSize.toPx() * .25f } }
    val rise = with(density) { 6.dp.toPx() }
    // For each shown row: whether a later sibling follows, for each depth, so the trunk lines carry on.
    val continues = remember(shown) {
        shown.indices.map { i -> (0..shown[i].second).map { d -> shown.drop(i + 1).takeWhile { it.second >= d }.any { it.second == d } } }
    }
    Column {
        shown.forEachIndexed { i, (node, depth, edge) ->
            val hasChild = shown.getOrNull(i + 1)?.second == depth + 1
            Row(Modifier.fillMaxWidth().drawBehind {
                val step = 24.dp.toPx(); val radius = 4.dp.toPx(); val stroke = 1.5.dp.toPx()
                val t = diagramReveal(elapsed(), min(i, 4))
                fun dot(d: Int) = d * step + 12.dp.toPx()
                for (d in 1 until depth) if (continues[i][d]) drawLine(line, Offset(dot(d - 1), 0f), Offset(dot(d - 1), size.height), stroke)
                if (depth > 0) {
                    val parent = dot(depth - 1)
                    drawPath(Path().apply { moveTo(parent, 0f); lineTo(parent, centerY - DiagramCorner.toPx()); quadraticTo(parent, centerY, parent + DiagramCorner.toPx(), centerY); lineTo(dot(depth) - radius - 3.dp.toPx(), centerY) }, line, style = Stroke(stroke, cap = StrokeCap.Round))
                    if (continues[i][depth]) drawLine(line, Offset(parent, centerY - DiagramCorner.toPx()), Offset(parent, size.height), stroke)
                }
                if (hasChild) drawLine(line, Offset(dot(depth), centerY + radius + 3.dp.toPx()), Offset(dot(depth), size.height), stroke * t)
                drawCircle(ink, (if (depth == 0) 5.dp.toPx() else radius) * t, Offset(dot(depth), centerY))
            }.graphicsLayer { val t = diagramReveal(elapsed(), min(i, 4)); alpha = t; translationY = rise * (1f - t) }.padding(start = (24 * depth + 36).dp, top = 6.dp, bottom = 6.dp),
                verticalAlignment = Alignment.Top, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                DiagramText(diagram.nodes[node].label, body, 3, Modifier.weight(1f, false).alignByBaseline())
                if (edge.text.isNotBlank()) CompositionLocalProvider(LocalContentColor provides ink.copy(alpha = DiagramQuiet)) {
                    DiagramText(edge, MaterialTheme.typography.labelMedium, 2, Modifier.alignByBaseline())
                }
            }
        }
    }
    DiagramMore(all.size - shown.size)
}

internal class MindTree(val root: Int, val parent: IntArray, val depth: IntArray, val order: List<Int>, val family: IntArray, val branches: List<Int>, val children: List<List<Int>>)

// The map's tree: breadth-first from its one root; each node belongs to the family of the first-level branch above it.
internal fun DiagramContent.mindTree(): MindTree {
    val children = nodes.indices.map { from -> edges.filter { it.from == from && it.to != from }.map { it.to }.distinct() }
    val root = nodes.indices.firstOrNull { n -> edges.none { it.to == n } } ?: 0
    val parent = IntArray(nodes.size) { -1 }; val depth = IntArray(nodes.size) { -1 }; val family = IntArray(nodes.size) { -1 }
    val order = mutableListOf(root); depth[root] = 0
    var i = 0
    while (i < order.size) { val at = order[i++]; children[at].forEach { c -> if (depth[c] < 0) { depth[c] = depth[at] + 1; parent[c] = at; family[c] = if (at == root) c else family[at]; order += c } } }
    val kept = order.toSet()
    return MindTree(root, parent, depth, order, family, children[root], children.map { it.filter { c -> parent[c] >= 0 && c in kept } })
}

// How a branch's topics continue: beyond it across the card, in a row beyond it, or stacked beyond it and indented like a list.
private enum class MindGrowth { Beside, Row, Stack }

// A branch grown away from the root: main runs away from the root, cross along the side. Positions are (main, cross) top-left corners
// for growth right or down with a stack indented toward larger cross; the caller mirrors them.
private class MindBlock(val main: Float, val cross: Float, val at: Map<Int, Pair<Float, Float>>)

private fun MindTree.grow(n: Int, growth: MindGrowth, sizes: List<IntSize>, gap: Float): MindBlock {
    val sideways = growth == MindGrowth.Beside
    val a = (if (sideways) sizes[n].width else sizes[n].height).toFloat(); val b = (if (sideways) sizes[n].height else sizes[n].width).toFloat()
    val kids = children[n].map { grow(it, growth, sizes, gap) }
    if (kids.isEmpty()) return MindBlock(a, b, mapOf(n to (0f to 0f)))
    val at = mutableMapOf(n to (0f to 0f))
    if (growth == MindGrowth.Stack) {
        val indent = b / 2 + gap * 1.5f; var m = a + gap
        kids.forEach { k -> k.at.forEach { (t, p) -> at[t] = m + p.first to indent + p.second }; m += k.main + gap }
        return MindBlock(m - gap, max(b, indent + kids.maxOf { it.cross }), at)
    }
    val stack = kids.sumOf { it.cross.toDouble() }.toFloat() + gap * (kids.size - 1)
    val cross = max(b, stack); val reach = a + gap * if (sideways) 3f else 2.5f
    at[n] = 0f to (cross - b) / 2
    var c = (cross - stack) / 2
    kids.forEach { k -> k.at.forEach { (t, p) -> at[t] = reach + p.first to c + p.second }; c += k.cross + gap }
    return MindBlock(reach + kids.maxOf { it.main }, cross, at)
}

// The reference mind map: the central topic in the middle, branches clockwise above, right, below and left of it, and each branch's topics
// continuing outward on its far side, spread where the card has room and stacked where it is narrow. Every split of the branches between
// the four sides is tried; the one that fits the width with the central topic nearest the middle wins. Null marks a topic with no room.
internal fun MindTree.arrange(sizes: List<IntSize>, width: Int, gap: Float, step: Float = 4f): List<Rect?> {
    val rootSize = sizes[root]
    val half = Offset(rootSize.width / 2f, rootSize.height / 2f)
    val grown = MindGrowth.entries.associateWith { g -> branches.map { grow(it, g, sizes, gap) } }
    fun layout(up: Int, right: Int, down: Int, sides: MindGrowth, rows: MindGrowth): Map<Int, Rect> {
        val out = mutableMapOf(root to Rect(-half, half))
        // Places a block with its top-left at (x, y); flip mirrors the growth, mirror the stack's indent.
        fun put(block: MindBlock, x: Float, y: Float, growth: MindGrowth, flip: Boolean, mirror: Boolean) = block.at.forEach { (n, p) ->
            val w = sizes[n].width.toFloat(); val h = sizes[n].height.toFloat()
            val sideways = growth == MindGrowth.Beside
            var px = if (sideways) p.first else p.second; var py = if (sideways) p.second else p.first
            if (flip) { if (sideways) px = block.main - px - w else py = block.main - py - h }
            if (mirror && !sideways) px = block.cross - px - w
            out[n] = Rect(Offset(x + px, y + py), Size(w, h))
        }
        fun wide(k: MindBlock, g: MindGrowth) = if (g == MindGrowth.Beside) k.main else k.cross
        fun tall(k: MindBlock, g: MindGrowth) = if (g == MindGrowth.Beside) k.cross else k.main
        val reach = gap * 3
        fun column(range: IntRange, leftward: Boolean) {
            val blocks = range.map { grown.getValue(sides)[it] }.let { if (leftward) it.reversed() else it }
            var y = -(blocks.sumOf { tall(it, sides).toDouble() }.toFloat() + gap * (blocks.size - 1).coerceAtLeast(0)) / 2
            blocks.forEach { k -> put(k, if (leftward) -half.x - reach - wide(k, sides) else half.x + reach, y, sides, leftward, leftward); y += tall(k, sides) + gap }
        }
        column(up until up + right, false); column(up + right + down until branches.size, true)
        val flank = out.values.filter { it != out[root] }
        fun row(range: IntRange, upward: Boolean) {
            val blocks = range.map { grown.getValue(rows)[it] }.let { if (upward) it else it.reversed() }
            if (blocks.isEmpty()) return
            val total = blocks.sumOf { it.cross.toDouble() }.toFloat() + gap * (blocks.size - 1)
            // A row clears the root, and any side column it reaches over.
            val blocking = flank.filter { it.right > -total / 2 - gap && it.left < total / 2 + gap }
            val edge = if (upward) min(-half.y, blocking.minOfOrNull { it.top } ?: 0f) - gap * 2.5f else max(half.y, blocking.maxOfOrNull { it.bottom } ?: 0f) + gap * 2.5f
            var x = -total / 2
            // A stack indents away from the centre; a lone one above leans left and below leans right, turning around the root.
            blocks.forEach { k -> val mid = x + k.cross / 2; put(k, x, if (upward) edge - k.main else edge, rows, upward, if (abs(mid) < 1f) upward else mid < 0); x += k.cross + gap }
        }
        row(0 until up, true); row(up + right until up + right + down, false)
        return out
    }
    var best: Map<Int, Rect>? = null; var score = Float.MAX_VALUE; var narrowest: Map<Int, Rect>? = null; var least = Float.MAX_VALUE
    val n = branches.size
    for (sides in listOf(MindGrowth.Beside, MindGrowth.Stack)) for (rows in listOf(MindGrowth.Row, MindGrowth.Stack))
    for (up in 0..n) for (right in 0..n - up) for (down in 0..n - up - right) {
        val left = n - up - right - down
        if (n >= 2 && listOf(up, right, down, left).count { it > 0 } < 2) continue
        if ((sides == MindGrowth.Stack && right + left == 0) || (rows == MindGrowth.Stack && up + down == 0)) continue
        val placed = layout(up, right, down, sides, rows)
        val l = placed.values.minOf { it.left }; val r = placed.values.maxOf { it.right }
        val t = placed.values.minOf { it.top }; val b = placed.values.maxOf { it.bottom }
        if (r - l < least) { least = r - l; narrowest = placed }
        if (r - l > width) continue
        // The height the card needs with the central topic centred, a lighter pull toward balance, and a nudge toward spreading out.
        val stacked = (if (sides == MindGrowth.Stack) 1 else 0) + (if (rows == MindGrowth.Stack) 1 else 0)
        val cost = 2 * max(-t, b) + .5f * abs(r + l) + .25f * abs(up - down) * gap + stacked * gap * 2
        if (cost < score) { score = cost; best = placed }
    }
    val chosen = best ?: narrowest!!.let { placed ->
        // Nothing fits: centre the narrowest split and leave out topics past either edge, and whatever hangs from them.
        val l = placed.values.minOf { it.left }; val r = placed.values.maxOf { it.right }; val mid = (l + r) / 2
        val kept = placed.filterValues { it.left >= mid - width / 2f && it.right <= mid + width / 2f }.keys
        placed.filterKeys { k -> generateSequence(k) { parent[it].takeIf { p -> p >= 0 } }.all { it in kept } }
    }
    // A map too crowded for any split falls back to the free search when that keeps more topics.
    if (best == null) search(sizes, width, gap, step).let { free -> if (free.count { it != null } > chosen.size) return free }
    return sizes.indices.map { chosen[it] }
}

private fun segmentHits(a: Offset, b: Offset, rect: Rect): Boolean {
    var t0 = 0f; var t1 = 1f; val d = b - a
    for ((p, q) in listOf(-d.x to a.x - rect.left, d.x to rect.right - a.x, -d.y to a.y - rect.top, d.y to rect.bottom - a.y)) {
        if (abs(p) < 1e-6f) { if (q < 0) return false; continue }
        val r = q / p
        if (p < 0) t0 = max(t0, r) else t1 = min(t1, r)
        if (t0 > t1) return false
    }
    return true
}

// Free placement for maps too crowded for the compass in a narrow card: the root at the centre, first-level branches around it by weight,
// and each child searched outward from its parent until it clears every placed topic and connector inside the width.
internal fun MindTree.place(sizes: List<IntSize>, width: Int, gap: Float, step: Float, start: Float = -PI.toFloat() / 2, reach: Float = 0f): List<Rect?> {
    val rects = arrayOfNulls<Rect>(sizes.size)
    val links = mutableListOf<Triple<Offset, Offset, Int>>()
    val half = width / 2f
    fun rect(center: Offset, n: Int) = Rect(center - Offset(sizes[n].width / 2f, sizes[n].height / 2f), Size(sizes[n].width.toFloat(), sizes[n].height.toFloat()))
    rects[root] = rect(Offset.Zero, root)
    val weight = IntArray(sizes.size) { 1 }
    for (n in order.reversed()) if (children[n].isNotEmpty()) weight[n] = children[n].sumOf { weight[it] }
    val sector = Array(sizes.size) { 0f to 0f }
    sector[root] = start to start + 2 * PI.toFloat()
    for (n in order) {
        val (start, end) = sector[n]; var at = start
        children[n].forEach { c -> val span = (end - start) * weight[c] / weight[n]; sector[c] = at to at + span; at += span }
    }
    val sweep = listOf(0) + (1..9).flatMap { listOf(it * 12, -it * 12) }
    val wide = sweep + (10..15).flatMap { listOf(it * 12, -it * 12) }
    for (n in order) {
        if (n == root) continue
        val p = parent[n]; val from = rects[p] ?: continue
        val angle = (sector[n].first + sector[n].second) / 2
        val origin = rects[root]!!.center
        var best: Pair<Float, Rect>? = null
        for (turn in if (p == root) wide else sweep) {
            val direction = angle + turn * PI.toFloat() / 180
            val unit = Offset(cos(direction), sin(direction))
            val clear = min(((from.width + sizes[n].width) / 2 + gap) / abs(unit.x).coerceAtLeast(.001f), ((from.height + sizes[n].height) / 2 + gap) / abs(unit.y).coerceAtLeast(.001f))
            var d = max(clear + if (p == root) 0f else gap, if (p == root) reach else 0f)
            while (d < width * 3f) {
                val center = from.center + unit * d
                val candidate = rect(center, n)
                d += step
                if (candidate.left < -half || candidate.right > half) { if (abs(unit.x) > .2f && (candidate.left < -half) == (unit.x < 0)) break else continue }
                if (p != root && (center - origin).getDistance() <= (from.center - origin).getDistance()) continue
                if (rects.withIndex().any { (k, r) -> r != null && r.inflate(if (k != p) gap else if (p == root) gap * 2.5f else gap * 1.5f).overlaps(candidate) }) continue
                if (rects.withIndex().any { (k, r) -> r != null && k != p && segmentHits(from.center, center, r.inflate(gap / 2)) }) continue
                if (links.any { (a, b, owner) -> owner != p && segmentHits(a, b, candidate.inflate(gap / 2)) }) continue
                val cost = d + abs(turn) * step * .9f
                if (best == null || cost < best.first) best = cost to candidate
                break
            }
        }
        best?.let { (_, r) -> rects[n] = r; links += Triple(from.center, r.center, p) }
    }
    return rects.toList()
}

// Tries a few turns of the first branch and radii for the first ring until one places every topic, else keeps whichever places most.
private fun MindTree.search(sizes: List<IntSize>, width: Int, gap: Float, step: Float): List<Rect?> {
    val quarter = PI.toFloat() / 2
    var best: List<Rect?>? = null; var score = -1 to 0f
    for (reach in listOf(0f, gap * 4, gap * 8)) for (start in listOf(-quarter, -quarter * 2 / 3, -quarter / 3, -quarter * 4 / 3)) {
        val rects = place(sizes, width, gap, step, start, reach)
        val placed = rects.filterNotNull()
        val next = placed.size to -(placed.maxOf { it.bottom } - placed.minOf { it.top })
        if (next.first > score.first || (next.first == score.first && next.second > score.second)) { best = rects; score = next }
        if (placed.size == order.size) return rects
    }
    return best!!
}

// Rings for maps too big to search: each topic mid-sector on its depth's ring, each ring pushed out until every topic clears its neighbours. Linear in topics.
internal fun MindTree.radial(sizes: List<IntSize>, gap: Float, start: Float = -PI.toFloat() / 2): List<Rect?> {
    val weight = IntArray(sizes.size) { 1 }
    for (n in order.reversed()) if (children[n].isNotEmpty()) weight[n] = children[n].sumOf { weight[it] }
    val sector = Array(sizes.size) { 0f to 0f }
    sector[root] = start to start + 2 * PI.toFloat()
    for (n in order) { val (a, b) = sector[n]; var at = a; children[n].forEach { c -> val span = (b - a) * weight[c] / weight[n]; sector[c] = at to at + span; at += span } }
    fun angle(n: Int) = (sector[n].first + sector[n].second) / 2
    // Half the topic's extent along its ray, and its full extent across it.
    fun along(n: Int, a: Float) = (abs(cos(a)) * sizes[n].width + abs(sin(a)) * sizes[n].height) / 2
    fun across(n: Int) = abs(sin(angle(n))) * sizes[n].width + abs(cos(angle(n))) * sizes[n].height
    val rings = order.groupBy { depth[it] }.let { g -> g.keys.sorted().map { g.getValue(it) } }
    val radius = FloatArray(rings.size)
    for (k in 1 until rings.size) {
        val out = rings[k].maxOf { n -> along(parent[n], angle(n)) + along(n, angle(n)) } + gap * 2
        val room = rings[k].maxOf { n -> (across(n) + gap) / (2 * sin(min(sector[n].second - sector[n].first, PI.toFloat()) / 2).coerceAtLeast(.001f)) }
        radius[k] = max(radius[k - 1] + out, room)
    }
    val rects = arrayOfNulls<Rect>(sizes.size)
    rings.forEachIndexed { k, ring -> ring.forEach { n -> val c = Offset(cos(angle(n)), sin(angle(n))) * radius[k]
        rects[n] = Rect(c - Offset(sizes[n].width / 2f, sizes[n].height / 2f), Size(sizes[n].width.toFloat(), sizes[n].height.toFloat())) } }
    return rects.toList()
}

// Laid-out maps by content, sizes and width, so a map scrolled back into view or reopened is not searched again.
private val mindLayouts = LinkedHashMap<Any, List<Rect?>>()
private fun mindLayout(key: Any, make: () -> List<Rect?>): List<Rect?> =
    mindLayouts.remove(key)?.also { mindLayouts[key] = it } ?: make().also { mindLayouts[key] = it; if (mindLayouts.size > 32) mindLayouts.remove(mindLayouts.keys.first()) }

// A connector leaves the parent's facing side and lands square on the child's, bending once like a hand-drawn branch.
internal fun mindLink(from: Rect, to: Rect, others: List<Rect> = emptyList()): List<Offset> {
    val d = to.center - from.center
    // A stacked topic hangs off a line dropped from its parent's centre and hooks into its near side, as in a list.
    val beyond = to.top >= from.bottom || to.bottom <= from.top
    val aside = to.left >= from.center.x + 4f || to.right <= from.center.x - 4f
    if (beyond && aside) {
        val a = Offset(from.center.x, if (d.y > 0) from.bottom else from.top); val corner = Offset(from.center.x, to.center.y)
        val b = Offset(if (to.left >= from.center.x) to.left else to.right, to.center.y)
        if (others.none { segmentHits(a, corner, it) || segmentHits(corner, b, it) }) return listOf(a, corner, corner, b)
    }
    val sideways = abs(d.x) / ((from.width + to.width) / 2) > abs(d.y) / ((from.height + to.height) / 2)
    if (sideways) {
        val s = if (d.x > 0) 1f else -1f
        val a = Offset(if (s > 0) from.right else from.left, from.center.y); val b = Offset(if (s > 0) to.left else to.right, to.center.y)
        if ((b.x - a.x) * s > 8f) { val mid = (a.x + b.x) / 2; return listOf(a, Offset(mid, a.y), Offset(mid, b.y), b) }
    }
    val s = if (d.y > 0) 1f else -1f
    val a = Offset(from.center.x, if (s > 0) from.bottom else from.top); val b = Offset(to.center.x, if (s > 0) to.top else to.bottom)
    val mid = (a.y + b.y) / 2
    return listOf(a, Offset(a.x, mid), Offset(b.x, mid), b)
}

// Branch families follow the chart rule, up to four an ink ramp and five or more the named palette, each drawn toward ink until its connector reads at 3:1.
internal fun mindColors(count: Int, ink: Color, ground: Color) = chartColors(count, ink, ground).map { c ->
    var t = 0f; var x = c
    while (chartContrast(x, ground) < 3f && t < 1f) { t += .05f; x = lerp(c, ink, t) }
    x
}

@Composable
private fun mindFamilies(tree: MindTree, ink: Color, ground: Color) = remember(tree, ink, ground) { mindColors(tree.branches.size, ink, ground) }

// Depth-first reading order, so a screen reader hears each branch through before the next.
internal fun MindTree.reading(): List<Int> = buildList { fun walk(n: Int) { add(n); children[n].forEach(::walk) }; walk(root) }

internal fun DiagramContent.topicDescription(tree: MindTree, n: Int): String = nodes[n].label.text.let { name ->
    if (n == tree.root) "$name, central topic, ${tree.children[n].size} branches" else "$name, under ${nodes[tree.parent[n]].label.text}" }

private const val MindMapNodes = 16

// A mind map: the central topic in ink, branches radiating in every direction, each branch's topics continuing outward in its colour family.
// The expanded view passes its viewport width as base: every topic is shown and the map widens until all of them fit.
@Composable
private fun DiagramMindMap(diagram: DiagramContent, elapsed: () -> Float, base: Dp? = null, hidden: Set<Int> = emptySet(), focused: Int? = null, focus: ((Int) -> Unit)? = null) {
    val ink = LocalContentColor.current
    val ground = groundColor()
    val tree = remember(diagram) { diagram.mindTree() }
    val families = mindFamilies(tree, ink, ground)
    fun hue(n: Int) = tree.branches.indexOf(tree.family[n]).let { if (it < 0) ink else families[it] }
    val drawn = remember(diagram) { DiagramInk() }
    val rise = with(LocalDensity.current) { 6.dp.toPx() }
    val shown = remember(tree, hidden, base) { tree.order.filter { it !in hidden }.let { if (base == null) it.take(MindMapNodes) else it } }
    val reading = remember(tree) { tree.reading().withIndex().associate { (i, n) -> n to i.toFloat() } }
    SubcomposeLayout((if (base == null) Modifier.fillMaxWidth() else Modifier.semantics { isTraversalGroup = true }).drawBehind { drawn.tick.intValue; drawStrokes(drawn.strokes, ink, elapsed()) }) { constraints ->
        val viewport = base?.roundToPx() ?: constraints.maxWidth
        val tiles = subcompose("nodes") { shown.forEach { n ->
            val spoken = if (focus == null) Modifier else Modifier.clearAndSetSemantics {
                contentDescription = diagram.topicDescription(tree, n); traversalIndex = reading[n] ?: 0f; role = Role.Button; selected = focused == n
                onClick("Focus topic") { focus(n); true }
            }
            MindTopic(diagram.nodes[n].label, tree.depth[n], hue(n), ink, ground, focused == n, focus?.let { { it(n) } }, spoken)
        } }
            .mapIndexed { i, m -> m.measure(Constraints(maxWidth = min(viewport, (if (tree.depth[shown[i]] == 0) 168.dp else 128.dp).roundToPx()))) }
        val sizes = MutableList(diagram.nodes.size) { IntSize(1, 1) }
        shown.forEachIndexed { i, n -> sizes[n] = IntSize(tiles[i].width, tiles[i].height) }
        val limited = MindTree(tree.root, tree.parent, tree.depth, shown, tree.family, tree.branches, tree.children.map { it.filter { c -> c in shown } })
        var width = viewport
        val gap = 12.dp.toPx(); val step = 4.dp.toPx()
        // The card searches for a compact map; the expanded view does too while it is small, else takes the rings, which always hold every topic.
        val raw = mindLayout(listOf(diagram, shown, sizes.toList(), viewport, base != null)) {
            if (base == null) limited.arrange(sizes, viewport, gap, step)
            else limited.takeIf { shown.size <= MindMapNodes }?.arrange(sizes, viewport, gap, step)?.takeIf { r -> shown.all { r[it] != null } } ?: limited.radial(sizes, gap)
        }
        val placed = raw.filterNotNull()
        val left = placed.minOf { it.left }; val top = placed.minOf { it.top }; val span = placed.maxOf { it.right } - left
        if (base != null) width = max(viewport, span.roundToInt())
        val shift = Offset((width - span) / 2 - left, -top)
        val rects = raw.map { it?.translate(shift) }
        val height = placed.maxOf { it.bottom } - top
        drawn.set(shown.filter { it != tree.root && rects[it] != null && rects[tree.parent[it]] != null }.map { n ->
            DiagramStroke(mindLink(rects[tree.parent[n]]!!, rects[n]!!, shown.mapNotNull { k -> rects[k]?.takeIf { k != n && k != tree.parent[n] }?.inflate(-1f) }), true, false, tree.depth[n] * 2 - 1, arc = true,
                color = hue(n), width = if (tree.depth[n] == 1) 2.5.dp else 1.5.dp)
        })
        val missing = diagram.nodes.size - hidden.size - shown.count { rects[it] != null }
        val more = subcompose("more") { DiagramMore(missing) }.map { it.measure(Constraints(maxWidth = width)) }
        val body = height.roundToInt()
        layout(width, body + more.sumOf { it.height }) {
            shown.forEachIndexed { i, n ->
                val r = rects[n] ?: return@forEachIndexed
                tiles[i].placeWithLayer(r.left.roundToInt(), r.top.roundToInt()) {
                    val t = diagramReveal(elapsed(), tree.depth[n] * 2); alpha = t; translationY = rise * (1f - t); scaleX = .94f + .06f * t; scaleY = scaleX
                }
            }
            more.forEach { it.place(0, body) }
        }
    }
}

@Composable
private fun MindTopic(label: RichText, depth: Int, hue: Color, ink: Color, ground: Color, selected: Boolean = false, onClick: (() -> Unit)? = null, spoken: Modifier = Modifier) {
    val center = MaterialTheme.typography.bodyMedium.copy(textAlign = TextAlign.Center)
    val shape = when (depth) { 0 -> RoundedCornerShape(16.dp); 1 -> RoundedCornerShape(14.dp); else -> RoundedCornerShape(50) }
    val fill = if (depth == 0) ink else mindFill(ground, hue, ink, if (selected) MindSelectedTint else if (depth == 1) MindBranchTint else MindLeafTint)
    val tap = if (onClick == null) Modifier else spoken.clip(shape).clickable(role = Role.Button, onClickLabel = "Focus topic", onClick = onClick)
    Box(tap.background(fill, shape).then(if (selected) Modifier.border(2.dp, ink, shape) else Modifier).heightIn(min = if (depth == 0) 48.dp else 40.dp).padding(horizontal = if (depth == 1) 12.dp else if (depth == 0) 16.dp else 14.dp, vertical = if (depth == 0) 10.dp else 8.dp),
        contentAlignment = Alignment.Center) {
        CompositionLocalProvider(LocalContentColor provides if (depth == 0) ground else ink, LocalMessageSurface provides fill) {
            DiagramText(label, if (depth == 0) MaterialTheme.typography.titleMedium.copy(textAlign = TextAlign.Center) else center, 3)
        }
    }
}

internal const val MindSelectedTint = .36f
internal const val MindBranchTint = .26f
internal const val MindLeafTint = .13f

// A family tint over the ground, eased back until body text in ink still reads at 4.5:1.
internal fun mindFill(ground: Color, hue: Color, ink: Color, tint: Float): Color {
    var t = tint
    while (t > 0f && chartContrast(ink, lerp(ground, hue, t)) < 4.5f) t -= .02f
    return lerp(ground, hue, t.coerceAtLeast(0f))
}

// The fallback for graphs too wide to draw legibly: one row per connection, source to target.
@Composable
private fun DiagramTransitions(diagram: DiagramContent, elapsed: () -> Float) {
    val ink = LocalContentColor.current
    val shown = diagram.edges.take(DiagramRows)
    val rise = with(LocalDensity.current) { 6.dp.toPx() }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        shown.forEachIndexed { i, edge ->
            Column(Modifier.graphicsLayer { val t = diagramReveal(elapsed(), min(i, 4)); alpha = t; translationY = rise * (1f - t) }, verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    DiagramText(diagram.nodes[edge.from].label, MaterialTheme.typography.bodyMedium, 3, Modifier.weight(1f, false))
                    CompositionLocalProvider(LocalContentColor provides ink.copy(alpha = DiagramQuiet)) { Glyph("arrow_forward", 16) }
                    DiagramText(diagram.nodes[edge.to].label, MaterialTheme.typography.bodyMedium, 3, Modifier.weight(1f, false))
                }
                if (edge.label.text.isNotBlank()) CompositionLocalProvider(LocalContentColor provides ink.copy(alpha = DiagramQuiet)) {
                    DiagramText(edge.label, MaterialTheme.typography.labelMedium, 2)
                }
            }
        }
    }
    DiagramMore(diagram.edges.size - shown.size)
}

@Composable
internal fun DiagramDetails(diagram: DiagramContent, dismiss: () -> Unit) {
    var focused by remember(diagram) { mutableStateOf<Int?>(null) }
    var collapsed by remember(diagram) { mutableStateOf(emptySet<Int>()) }
    var branch by remember(diagram) { mutableStateOf<Int?>(null) }
    var outline by remember(diagram) { mutableStateOf(false) }
    val hidden = remember(diagram, collapsed, branch) { collapsed.flatMap { diagram.descendants(it) - it }.toSet() + (branch?.let { diagram.nodes.indices.toSet() - diagram.descendants(it) } ?: emptySet()) }
    val kindName = when (diagram.kind) { "org" -> "Org chart"; "mindmap" -> "Mind map"; else -> diagram.kind.replaceFirstChar { it.uppercase() } }
    Dialog(dismiss, DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            CompositionLocalProvider(LocalMessageSurface provides MaterialTheme.colorScheme.surface) {
                Column(Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        SigilIconButton(dismiss) { Glyph("close", 24, "Close diagram") }
                        Text(kindName, Modifier.weight(1f), style = MaterialTheme.typography.titleLarge)
                        if (diagram.kind != "timeline") SigilIconButton({ outline = !outline }) { Glyph(if (outline) "schema" else "format_list_bulleted", 24, if (outline) "Show diagram" else "List nodes") }
                    }
                    RichMessageText(diagram.title, Modifier.heightIn(max = 96.dp).verticalScroll(rememberScrollState()), MaterialTheme.typography.titleMedium)
                    if (diagram.kind == "timeline") LazyColumn(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                        itemsIndexed(diagram.entries, key = { index, _ -> index }) { index, entry -> Row(itemMotion(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            Column(Modifier.width(88.dp)) { Text("${index + 1}", style = MaterialTheme.typography.labelSmall); RichMessageText(entry.date, style = MaterialTheme.typography.labelLarge) }
                            RichMessageText(entry.label, Modifier.weight(1f))
                        } }
                    } else {
                        if (outline) LazyColumn(Modifier.weight(1f).fillMaxWidth()) {
                            itemsIndexed(diagram.nodes, key = { index, _ -> index }) { index,node ->
                                Row(itemMotion().fillMaxWidth().heightIn(min=48.dp).combinedClickable(onClick={ focused=index;outline=false;collapsed=emptySet();branch=null },onLongClickLabel="Focus node",onLongClick={ focused=index;outline=false;collapsed=emptySet();branch=null }).padding(12.dp),horizontalArrangement=Arrangement.spacedBy(12.dp)) {
                                    Text("${index+1}",style=MaterialTheme.typography.labelLarge)
                                    RichMessageText(node.label,Modifier.weight(1f))
                                }
                            }
                        } else if (diagram.kind == "mindmap") BoxWithConstraints(Modifier.weight(1f).fillMaxWidth().semantics { contentDescription = "mindmap diagram viewport" }) {
                            val base = maxWidth
                            Box(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).horizontalScroll(rememberScrollState())) {
                                DiagramMindMap(diagram, { Float.MAX_VALUE }, base, hidden, focused) { focused = it }
                            }
                        } else DiagramPlot(diagram, hidden, focused, { focused = it }, Modifier.weight(1f).fillMaxWidth(), true)
                        if (branch != null) SigilTextButton({ branch = null }) { Text("Show whole diagram") }
                        focused?.let { index ->
                            Column(Modifier.fillMaxWidth().heightIn(max = 260.dp).verticalScroll(rememberScrollState()).background(lerp(MaterialTheme.colorScheme.surface, LocalContentColor.current, DiagramFill), RoundedCornerShape(24.dp)).padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                                Row(verticalAlignment = Alignment.CenterVertically) {
                                    Column(Modifier.weight(1f)) { RichMessageText(diagram.nodes[index].label, style = MaterialTheme.typography.titleMedium) }
                                    SigilIconButton({ focused = null }) { Glyph("close", 20, "Clear node focus") }
                                }
                                if (diagram.kind == "mindmap" && diagram.edges.any { it.from == index }) SigilTextButton({ collapsed = if (index in collapsed) collapsed - index else collapsed + index }) { Text(if (index in collapsed) "Expand branch" else "Collapse branch") }
                                if (diagram.kind == "org" && diagram.edges.any { it.from == index }) SigilTextButton({ branch = index }) { Text("Focus branch") }
                                diagram.edges.filter { it.from == index || it.to == index }.forEach { edge ->
                                    val target = if (edge.from == index) edge.to else edge.from
                                    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).clip(RoundedCornerShape(12.dp)).clickable(role = Role.Button) { focused = target }.padding(vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                                        Glyph(if (edge.from == index) "arrow_forward" else "arrow_back", 20, if (edge.from == index) "Connects to" else "Connected from")
                                        Column(Modifier.weight(1f)) { RichMessageText(diagram.nodes[target].label); if (edge.label.text.isNotBlank()) RichMessageText(edge.label, style = MaterialTheme.typography.labelMedium) }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun DiagramPlot(diagram: DiagramContent, hidden: Set<Int>, focused: Int?, focus: (Int) -> Unit, modifier: Modifier, interactive: Boolean) {
    var bounds by remember { mutableStateOf(IntSize.Zero) }
    var zoom by remember(diagram) { mutableFloatStateOf(1f) }
    var pan by remember(diagram) { mutableStateOf(Offset.Zero) }
    var initialized by remember(diagram) { mutableStateOf(false) }
    val density = LocalDensity.current.density
    val ink = LocalContentColor.current
    val surface = LocalMessageSurface.current.takeOrElse { MaterialTheme.colorScheme.surface }
    val connected = remember(diagram, focused) { focused?.let { node -> setOf(node) + diagram.edges.filter { it.from == node || it.to == node }.flatMap { listOf(it.from,it.to) } } ?: diagram.nodes.indices.toSet() }
    val tree = remember(diagram) { diagram.takeIf { it.kind == "mindmap" }?.mindTree() }
    val families = remember(tree, ink, surface) { tree?.let { mindColors(it.branches.size, ink, surface) } }
    fun hue(n: Int) = tree?.let { t -> t.branches.indexOf(t.family[n]).takeIf { it >= 0 }?.let { families!![it] } }
    val initial = remember(diagram) { diagram.initialState() }
    val finals = remember(diagram) { diagram.finalStates() }
    // Room under the core's last row for a final bullseye.
    val tall = diagram.height + if (finals.isNotEmpty()) 16f else 0f
    // Each return edge takes its own lane beside the column, so two returns into one node stay apart.
    val backRank = remember(diagram) { var k = 0; diagram.edges.map { e -> if (diagram.kind != "sequence" && diagram.kind != "mindmap" && diagram.nodes[e.to].y <= diagram.nodes[e.from].y) k++ else -1 } }
    fun backSide(start: Offset, end: Offset, edge: Int) = max(start.x, end.x) + (48 + 40 * backRank[edge].coerceAtLeast(0)) * density * zoom
    fun minimumZoom() = min(bounds.width / (diagram.width * density), bounds.height / (tall * density)).coerceIn(.0001f, 1f)
    fun fit() {
        if (bounds.width == 0 || bounds.height == 0) return
        zoom = if (interactive) minimumZoom() else minimumZoom().coerceAtLeast(.65f)
        pan = Offset(max(0f,(bounds.width-diagram.width*density*zoom)/2),max(0f,(bounds.height-tall*density*zoom)/2))
    }
    LaunchedEffect(diagram, bounds) { if (!initialized && bounds.width > 0 && bounds.height > 0) { fit(); initialized=true } }
    LaunchedEffect(focused, initialized) {
        if (interactive && initialized && focused != null) {
            val node=diagram.nodes[focused]
            zoom=max(zoom,.65f)
            pan=Offset(bounds.width/2f,bounds.height/2f)-Offset((node.x+80)*density*zoom,(node.y+36)*density*zoom)
        }
    }
    fun at(x: Float,y: Float) = Offset(x*density*zoom,y*density*zoom)+pan
    // A mind map's centre is ink and its branches keep their families; every other node is a tonal tile.
    fun nodeFill(index: Int) = when { tree?.root == index -> ink; hue(index) != null -> mindFill(surface, hue(index)!!, ink, if (focused == index) MindSelectedTint else if (tree!!.depth[index] == 1) MindBranchTint else MindLeafTint)
        else -> lerp(surface, ink, if (focused == index) .16f else DiagramFill) }
    fun shown(rect: Rect) = rect.right > 0 && rect.bottom > 0 && rect.left < bounds.width && rect.top < bounds.height
    Column(modifier, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Box(Modifier.weight(1f).fillMaxWidth().clipToBounds().onSizeChanged { bounds=it }.then(if (!interactive) Modifier else Modifier.pointerInput(diagram, hidden) {
            detectTapGestures { position ->
                if (zoom < .45f) {
                    val node = diagram.nodes.indices.filter { it !in hidden }.minByOrNull { (at(diagram.nodes[it].x+80,diagram.nodes[it].y+36)-position).getDistanceSquared() }
                    if (node != null && (at(diagram.nodes[node].x+80,diagram.nodes[node].y+36)-position).getDistance() < 24.dp.toPx()) focus(node)
                }
            }
        }.pointerInput(diagram) {
            detectTransformGestures { centroid, change, scale, _ ->
                val next = (zoom*scale).coerceIn(minimumZoom(),3f)
                pan=(pan-centroid)*(next/zoom)+centroid+change
                zoom=next
                pan=Offset(pan.x.coerceIn(-diagram.width*density*zoom, size.width.toFloat()),pan.y.coerceIn(-tall*density*zoom,size.height.toFloat()))
            }
        }).semantics { contentDescription="${diagram.kind} diagram viewport" }) {
            Canvas(Modifier.fillMaxSize()) {
                if (zoom < .45f) diagram.nodes.forEachIndexed { index,node -> if (index !in hidden) drawRoundRect(ink.copy(alpha=if(index in connected).7f else .2f),at(node.x,node.y),Size(160*density*zoom,72*density*zoom),CornerRadius(3.dp.toPx()*zoom)) }
                if (diagram.kind == "sequence") diagram.nodes.forEachIndexed { index,node -> if (index !in hidden) drawLine(ink.copy(alpha=.25f),at(node.x+80,node.y+72),at(node.x+80,diagram.height-24),1.dp.toPx(),pathEffect=PathEffect.dashPathEffect(floatArrayOf(6.dp.toPx(),4.dp.toPx()))) }
                diagram.edges.forEachIndexed { index, edge ->
                    if (edge.from in hidden || edge.to in hidden) return@forEachIndexed
                    val from=diagram.nodes[edge.from]; val to=diagram.nodes[edge.to]
                    val strong=focused==null || edge.from==focused || edge.to==focused
                    val color=(hue(edge.to) ?: ink).copy(alpha=if(strong) .8f else .16f)
                    val start:Offset; val end:Offset; val control1:Offset; val control2:Offset
                    if(diagram.kind=="sequence") {
                        start=at(from.x+80,edge.y); end=at(to.x+80,edge.y)
                        if(edge.from==edge.to) { control1=start+Offset(48.dp.toPx()*zoom,0f);control2=end+Offset(48.dp.toPx()*zoom,32.dp.toPx()*zoom) } else { control1=start;control2=end }
                    } else if(diagram.kind=="mindmap") {
                        val a=Offset(from.x+80,from.y+36);val b=Offset(to.x+80,to.y+36);val delta=b-a
                        val border=min(if(abs(delta.x)<.001f)Float.MAX_VALUE else 80/abs(delta.x),if(abs(delta.y)<.001f)Float.MAX_VALUE else 36/abs(delta.y))
                        start=at((a+delta*border).x,(a+delta*border).y);end=at((b-delta*border).x,(b-delta*border).y);control1=start;control2=end
                    } else if(to.y>from.y) {
                        start=at(from.x+80,from.y+72);end=at(to.x+80,to.y);control1=Offset(start.x,(start.y+end.y)/2);control2=Offset(end.x,control1.y)
                    } else {
                        // Returns into one state land at spread points down its side so their heads stay apart.
                        start=at(from.x+160,from.y+36);end=at(to.x+160,to.y+36+listOf(0f,-16f,16f)[backRank[index].coerceAtLeast(0)%3]);val side=backSide(start,end,index);control1=Offset(side,start.y-36.dp.toPx()*zoom);control2=Offset(side,end.y+36.dp.toPx()*zoom)
                    }
                    val path=Path().apply { moveTo(start.x,start.y);cubicTo(control1.x,control1.y,control2.x,control2.y,end.x,end.y) }
                    drawPath(path,color,style=Stroke(if(strong)2.dp.toPx() else 1.5.dp.toPx(),pathEffect=if(edge.dashed)PathEffect.dashPathEffect(floatArrayOf(6.dp.toPx(),4.dp.toPx()))else null))
                    val direction=end-(if(control2==end)start else control2);val angle=atan2(direction.y,direction.x);val head=8.dp.toPx()
                    drawPath(Path().apply {moveTo(end.x,end.y);lineTo(end.x-cos(angle-.42f)*head,end.y-sin(angle-.42f)*head);lineTo(end.x-cos(angle+.42f)*head,end.y-sin(angle+.42f)*head);close()},color)
                }
                // The same state marks as the card: an entry arrow from a dot, and a bullseye beneath each state with no way out.
                fun arrow(a: Offset, b: Offset) { val head=7.dp.toPx()*zoom; val half=3.5.dp.toPx()*zoom; val u=(b-a)/(b-a).getDistance().coerceAtLeast(.001f); val n=Offset(-u.y,u.x); val base=b-u*head
                    drawLine(ink,a,base,1.5.dp.toPx()*zoom); drawPath(Path().apply { moveTo(b.x,b.y); lineTo(base.x+n.x*half,base.y+n.y*half); lineTo(base.x-n.x*half,base.y-n.y*half); close() },ink) }
                if (zoom >= .45f) {
                    initial?.takeIf { it !in hidden }?.let { n -> val node=diagram.nodes[n]; val dot=at(node.x+80,node.y-22)
                        drawCircle(ink,5.dp.toPx()*zoom,dot); arrow(dot+Offset(0f,5.dp.toPx()*zoom),at(node.x+80,node.y)) }
                    finals.filter { it !in hidden }.forEach { n -> val node=diagram.nodes[n]; val ring=at(node.x+80,node.y+72+26); val r=DiagramMarkRadius.toPx()*zoom
                        arrow(at(node.x+80,node.y+72),ring-Offset(0f,r+1.dp.toPx()*zoom)); drawCircle(ink,r,ring,style=Stroke(1.5.dp.toPx()*zoom)); drawCircle(ink,4.dp.toPx()*zoom,ring) }
                }
            }
            // An edge label is a UI chip, not geometry: it keeps its theme size at every zoom and rides a plate so the line cannot cut through it.
            diagram.edges.forEachIndexed { index, edge ->
                if(zoom < .45f || edge.from in hidden || edge.to in hidden || edge.label.text.isBlank()) return@forEachIndexed
                val from=diagram.nodes[edge.from];val to=diagram.nodes[edge.to]
                val lane=if(diagram.kind=="sequence")max(96f,abs(from.x-to.x)) else 128f
                val middle=when {
                    diagram.kind=="sequence" -> at((from.x+to.x)/2+80,edge.y-40)
                    diagram.kind=="mindmap" -> at((from.x+to.x)/2+80,(from.y+to.y)/2+20)
                    to.y>from.y -> at((from.x+to.x)/2+80,(from.y+72+to.y)/2-20)
                    else -> { val s=at(from.x+160,from.y+36); val e=at(to.x+160,to.y+36); Offset((s.x+e.x+6*backSide(s,e,index))/8,(s.y+e.y)/2-12*density) }
                }
                val plate=max(96f,lane*zoom)
                val left=middle.x-plate*density/2
                if(!shown(Rect(Offset(left,middle.y),Size(plate*density,40*density)))) return@forEachIndexed
                Box(Modifier.offset { IntOffset(left.roundToInt(),middle.y.roundToInt()) }.width(plate.dp)
                    .semantics { contentDescription="Connection ${index+1}" },contentAlignment=Alignment.Center) {
                    Surface(shape=RoundedCornerShape(6.dp),color=surface,contentColor=ink,
                        modifier=if(interactive)Modifier.clickable(role=Role.Button) { focus(edge.from) } else Modifier) {
                        RichMessageText(edge.label,Modifier.padding(horizontal=6.dp,vertical=2.dp),MaterialTheme.typography.labelMedium)
                    }
                }
            }
            diagram.nodes.forEachIndexed { index,node ->
                if(zoom < .45f || index in hidden) return@forEachIndexed
                val position=at(node.x,node.y)
                if(!shown(Rect(position,Size(160*density*zoom,72*density*zoom)))) return@forEachIndexed
                val shape=diagramNodeShape(diagram.kind,node.shape)
                Surface(Modifier.offset { IntOffset(position.x.roundToInt(),position.y.roundToInt()) }
                    .graphicsLayer { scaleX=zoom;scaleY=zoom;transformOrigin=TransformOrigin(0f,0f);alpha=if(index in connected)1f else .3f }.size(160.dp,72.dp)
                    .then(if(interactive)Modifier.combinedClickable(onClick={focus(index)},onLongClickLabel="Focus node",onLongClick={focus(index)}) else Modifier)
                    .semantics { contentDescription="Node ${index+1}"; selected=focused==index }, shape=shape,
                    color=nodeFill(index),contentColor=if(tree?.root==index) surface else ink) {
                    CompositionLocalProvider(LocalMessageSurface provides nodeFill(index)) {
                        Box(Modifier.padding(horizontal=if(node.shape=="decision")28.dp else 10.dp,vertical=8.dp).clipToBounds(),contentAlignment=Alignment.Center) { RichMessageText(node.label,style=MaterialTheme.typography.bodyMedium) }
                    }
                }
            }
        }
        if(interactive) SigilTextButton({fit()}) { Text("Fit diagram") }
        if(interactive && zoom < .45f) Text("Overview · zoom in to read labels", style=MaterialTheme.typography.labelSmall)
    }
}
