package org.sigil

import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.*
import androidx.compose.foundation.gestures.*
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsFocusedAsState
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.*
import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.*
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.*
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.*
import androidx.compose.ui.util.lerp
import androidx.compose.ui.window.*
import kotlin.math.*

// Named SigilText colours for five or more categories; greys stop reading apart past four.
internal fun chartPalette(count: Int, surface: Color) = List(count) { textColor(listOf("purple2", "blue2", "green2", "orange2", "pink2", "cyan2")[it % 6], surface) }
// Up to four categories are an ink ramp over the bubble ground, in data order.
internal fun chartColors(count: Int, ink: Color, ground: Color) = if (count <= 4) chartRamp(count, ink, ground) else chartPalette(count, ground)

internal fun chartContrast(a: Color, b: Color) = (max(a.luminance(), b.luminance()) + .05f) / (min(a.luminance(), b.luminance()) + .05f)

/** Ramp stops spaced evenly in contrast: the palest clears 3:1 on the ground where the ink leaves room for 1.6:1 steps. */
internal fun chartRamp(count: Int, ink: Color, ground: Color): List<Color> {
    if (count <= 1) return List(count) { ink }
    val total = chartContrast(ink, ground)
    val even = total.pow(1f / count)
    // 3.1 leaves room for 8-bit rounding of the stop.
    val palest = if (even >= 3.1f) even else max(even, min(3.1f, total / 1.6f.pow(count - 1)))
    val darker = ink.luminance() < ground.luminance()
    val base = ground.luminance() + .05f
    fun mix(t: Float) = Color(lerp(ground.red, ink.red, t), lerp(ground.green, ink.green, t), lerp(ground.blue, ink.blue, t))
    return List(count) { k ->
        val target = palest * (total / palest).pow((count - 1 - k) / (count - 1f))
        val luminance = if (darker) base / target - .05f else base * target - .05f
        // Luminance is monotonic along the sRGB mix of two greys, so bisection finds the stop.
        var lo = 0f; var hi = 1f
        repeat(24) { val mid = (lo + hi) / 2; if ((mix(mid).luminance() < luminance) == darker) hi = mid else lo = mid }
        mix((lo + hi) / 2)
    }
}

// Marks grow over MotionSettle with at most five stagger steps; fits inside ChartMotionMillis.
internal const val ChartReveal = MotionSettle + 4 * MotionStagger
private val ChartPlotHeight = 144.dp
private val ChartPie = 168.dp
private const val ChartLegendRows = 5
private const val ChartBarRows = 8
private val Figures = TextStyle(fontFeatureSettings = "tnum, lnum")

internal fun chartKindName(kind: String) = kind.replaceFirstChar { it.uppercase() }
private fun RichText.concealed() = spans.any { it.reveal.isNotEmpty() || it.redaction > 0 }
internal fun ChartContent.pointName(index: Int) = points[index].label.let { if (it.concealed() || it.text.isBlank()) "Item ${index + 1}" else it.text }

/** People-facing figure: true minus sign, grouped thousands, % hard against the number. */
internal fun chartFigure(raw: String, group: Boolean = true): String {
    val percent = raw.trim().endsWith("%")
    var body = raw.trim().removeSuffix("%").trim()
    val negative = body.startsWith("-") || body.startsWith("−")
    body = body.trimStart('-', '−', '+')
    if (body.isEmpty() || !body.all { it.isDigit() || it == '.' } || body.count { it == '.' } > 1) return raw.replace('-', '−')
    val whole = body.substringBefore('.').ifEmpty { "0" }
    val fraction = body.substringAfter('.', "")
    val grouped = if (group && whole.length > 3) whole.reversed().chunked(3).joinToString(",").reversed() else whole
    val sign = if (negative && body.any { it in '1'..'9' }) "−" else ""
    return sign + grouped + (if (fraction.isNotEmpty()) ".$fraction" else "") + if (percent) "%" else ""
}

private fun decimalsOf(step: Double): Int = (0..6).firstOrNull { d -> val s = step * 10.0.pow(d); abs(s - s.roundToLong()) < 1e-6 * max(1.0, s) } ?: 6

internal fun chartFixed(value: Double, decimals: Int, group: Boolean = true): String {
    val scale = 10.0.pow(decimals)
    val scaled = (value * scale).roundToLong()
    val magnitude = abs(scaled)
    val whole = (magnitude / scale.toLong()).toString()
    val fraction = if (decimals > 0) "." + (magnitude % scale.toLong()).toString().padStart(decimals, '0') else ""
    return chartFigure((if (scaled < 0) "-" else "") + whole + fraction, group)
}

/** Share of the whole: whole numbers stay whole, otherwise one decimal. */
internal fun chartPercent(share: Float): String {
    val tenths = (share * 1000.0).roundToLong()
    return (if (tenths % 10 == 0L) (tenths / 10).toString() else "${tenths / 10}.${tenths % 10}") + "%"
}

/** Three to five "nice" ticks, four preferred (1, 2, 2.5 or 5 × 10ⁿ) covering [low, high]. */
internal class ChartScale(val min: Double, val max: Double, val step: Double) {
    val ticks = (0..((max - min) / step).roundToInt()).map { min + it * step }
    fun at(value: Double) = ((value - min) / (max - min)).toFloat()
}
internal fun chartScale(low: Double, high: Double, whole: Boolean = false): ChartScale {
    var a = min(low, high); var b = max(low, high)
    if (b - a < 1e-12) { if (a == 0.0) b = 1.0 else { a -= 1; b += 1 } }
    val base = 10.0.pow(floor(log10((b - a) / 3)))
    var best: ChartScale? = null; var score = Double.MAX_VALUE
    for (k in listOf(.1, 1.0, 10.0)) for (m in listOf(1.0, 2.0, 2.5, 5.0)) {
        val step = m * k * base
        // Whole-number data never gets fractional ticks.
        if (whole && decimalsOf(step) > 0) continue
        val from = floor(a / step + 1e-9) * step; val to = ceil(b / step - 1e-9) * step
        val count = ((to - from) / step).roundToInt() + 1
        if (count !in 3..5) continue
        val s = (to - from) / (b - a) * (if (m == 2.5) 1.05 else 1.0) + abs(count - 4) * .4
        if (s < score) { score = s; best = ChartScale(from, to, step) }
    }
    return best ?: if (whole) floor(a).let { ChartScale(it, max(ceil(b), it + 2), 1.0) } else ChartScale(a, b, (b - a) / 2)
}

/** Positions on one axis (0 bottom/left, 1 top/right) and the tick labels that go with them. */
internal class ChartAxis(val positions: List<Float>, val zero: Float, val ticks: List<Pair<Float, String>>)

private fun parse(raw: String?) = raw?.trim()?.removeSuffix("%")?.replace('−', '-')?.toDoubleOrNull()?.takeIf { it.isFinite() }

internal fun ChartContent.valueAxis(): ChartAxis {
    val values = points.map { parse(it.value) }
    val low = values.filterNotNull().minOrNull()?.let { min(it, 0.0) }; val high = values.filterNotNull().maxOrNull()?.let { max(it, 0.0) }
    val unit = if (points.isNotEmpty() && points.all { it.value.trim().endsWith("%") }) "%" else ""
    if (low == null || high == null || values.isEmpty() || values.any { it == null }) return ChartAxis(points.map { it.y }, zero, yTicks.mapIndexed { i, t -> i / max(1f, yTicks.size - 1f) to chartFigure(t) })
    val scale = chartScale(min(low, values.minOf { it!! }), max(high, values.maxOf { it!! }), values.all { it!! == floor(it) })
    val decimals = decimalsOf(scale.step)
    return ChartAxis(values.map { scale.at(it!!) }, scale.at(0.0).coerceIn(0f, 1f), scale.ticks.map { scale.at(it) to chartFixed(it, decimals) + unit })
}

internal fun ChartContent.xAxis(): ChartAxis {
    val values = points.map { parse(it.xValue) }
    val low = values.filterNotNull().minOrNull()?.let { min(it, 0.0) }; val high = values.filterNotNull().maxOrNull()?.let { max(it, 0.0) }
    if (low == null || high == null || values.isEmpty() || values.any { it == null }) return ChartAxis(points.map { it.x }, 0f, xTicks.mapIndexed { i, t -> i / max(1f, xTicks.size - 1f) to chartFigure(t, false) })
    val scale = chartScale(min(low, values.minOf { it!! }), max(high, values.maxOf { it!! }), values.all { it!! == floor(it) })
    val decimals = decimalsOf(scale.step)
    return ChartAxis(values.map { scale.at(it!!) }, scale.at(0.0), scale.ticks.map { scale.at(it) to chartFixed(it, decimals, false) })
}

/** Sum of the values at the precision they were written in; null when any value is not a plain number. */
internal fun ChartContent.total(): String? {
    if (points.isEmpty()) return null
    val values = points.map { parse(it.value) ?: return null }
    val decimals = points.maxOf { it.value.trim().removeSuffix("%").substringAfter('.', "").length }.coerceAtMost(6)
    return chartFixed(values.sum(), decimals) + if (points.all { it.value.trim().endsWith("%") }) "%" else ""
}

internal fun ChartContent.summary() = points.indices.joinToString("; ") { i ->
    val p = points[i]
    listOfNotNull(pointName(i), p.xValue?.let { "x ${chartFigure(it, false)}" }, chartFigure(p.value), chartPercent(p.share).takeIf { kind in listOf("pie", "donut") }).joinToString(", ")
}

@Composable
private fun chartDetails(chart: ChartContent, index: Int) {
    val point = chart.points[index]
    Column(Modifier.fillMaxWidth().padding(vertical = 4.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        RichMessageText(point.label, Modifier.fillMaxWidth(), MaterialTheme.typography.bodyMedium)
        Text(listOfNotNull(point.xValue?.let { "x = ${chartFigure(it, false)}" }, chartFigure(point.value), chartPercent(point.share).takeIf { chart.kind in listOf("pie", "donut") }?.let { "$it of total" }).joinToString(" · "),
            style = MaterialTheme.typography.labelMedium.merge(Figures), color = LocalContentColor.current.copy(alpha = .68f), maxLines = 2, overflow = TextOverflow.Ellipsis)
    }
}

/** Plain text ellipsizes at [lines]; styled or concealed text keeps its spans and is clipped instead. */
@Composable
private fun ChartText(value: RichText, lines: Int, style: TextStyle, modifier: Modifier = Modifier) {
    if (value.spans.isEmpty() && value.blocks.isEmpty() && value.codeTokens.isEmpty() && value.motion.isEmpty()) Text(value.text, modifier, style = style, maxLines = lines, overflow = TextOverflow.Ellipsis)
    else RichMessageText(value, modifier.heightIn(max = with(LocalDensity.current) { style.lineHeight.toDp() } * lines).clipToBounds(), style)
}

@Composable
internal fun ChartCard(chart: ChartContent) {
    var selected by remember(chart) { mutableStateOf<Int?>(null) }
    val toggle: (Int) -> Unit = { selected = if (selected == it) null else it }
    Column(Modifier.widthIn(min = MessageCardMinWidth, max = MessageCardMaxWidth).fillMaxWidth().padding(vertical = 4.dp)
        .semantics { contentDescription = "${chartKindName(chart.kind)} chart" }
        .animateContentSize(LocalMotion.current.tween(MotionMillis)), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        if (chart.title.text.isNotBlank()) ChartText(chart.title, 3, MaterialTheme.typography.titleMedium)
        ChartPlot(chart, emptySet(), selected, toggle, Modifier.fillMaxWidth())
        if (chart.kind in listOf("pie", "donut")) ChartLegend(chart, selected, toggle)
    }
}

/** Ink on a 14dp squircle, 7% pressed or hovered and 10% keyboard-focused, eased over MotionFeedback. */
@Composable
private fun interactionFill(interaction: MutableInteractionSource): Float {
    val pressed by interaction.collectIsPressedAsState()
    val hovered by interaction.collectIsHoveredAsState()
    val focused by interaction.collectIsFocusedAsState()
    return animateFloatAsState(if (focused && !pressed) .10f else if (pressed || hovered) .07f else 0f, LocalMotion.current.tween(MotionFeedback), label = "Chart target").value
}

/** Reaches a little past the row so text keeps the card edge. */
private fun Modifier.rowFill(alpha: () -> Float, ink: Color, bleed: Dp = 6.dp) = drawBehind {
    val a = alpha()
    if (a > 0f) drawRoundRect(ink.copy(alpha = a), Offset(-bleed.toPx(), 0f), Size(size.width + bleed.toPx() * 2, size.height), CornerRadius(14.dp.toPx()))
}

@Composable
private fun ChartLegend(chart: ChartContent, selected: Int?, toggle: (Int) -> Unit) {
    val ink = LocalContentColor.current
    val ground = LocalMessageSurface.current.takeOrElse { MaterialTheme.colorScheme.surface }
    val colors = remember(chart, ink, ground) { chartColors(chart.points.size, ink, ground) }
    val body = MaterialTheme.typography.bodyMedium
    val meta = MaterialTheme.typography.labelMedium.merge(Figures)
    val measurer = rememberTextMeasurer()
    val density = LocalDensity.current
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        // Four or fewer read as one column; wider legends split by a hairline so each value stays with its own label.
        val column = (maxWidth - 33.dp) / 2
        val two = chart.points.size > 4 && maxWidth >= 280.dp && chart.points.indices.all { i ->
            val label = with(density) { measurer.measure(chart.pointName(i), body, maxLines = 1).size.width.toDp() }
            val value = with(density) { measurer.measure(chartFigure(chart.points[i].value) + " · " + chartPercent(chart.points[i].share), meta, maxLines = 1).size.width.toDp() }
            10.dp + 12.dp + label + 12.dp + value <= column
        }
        val columns = if (two) 2 else 1
        val shown = chart.points.indices.take(ChartLegendRows * columns)
        Column(Modifier.fillMaxWidth()) {
            shown.chunked(columns).forEach { row ->
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    row.forEachIndexed { k, i ->
                        // A hairline between columns closes each value off from the next swatch.
                        if (k > 0) Box(Modifier.width(1.dp).height(24.dp).background(ink.copy(alpha = .08f)))
                        LegendRow(chart, i, colors[i], selected, toggle, Modifier.weight(1f))
                    }
                    if (row.size < columns) { Spacer(Modifier.width(1.dp)); Spacer(Modifier.weight(1f)) }
                }
            }
            if (chart.points.size > shown.size) Text("+${chart.points.size - shown.size} more", Modifier.padding(top = 4.dp), style = MaterialTheme.typography.labelMedium, color = ink.copy(alpha = .68f), maxLines = 1)
        }
    }
}

@Composable
private fun LegendRow(chart: ChartContent, index: Int, color: Color, selected: Int?, toggle: (Int) -> Unit, modifier: Modifier) {
    val ink = LocalContentColor.current
    val motion = LocalMotion.current
    val point = chart.points[index]
    val active = selected == index
    val emphasis by animateFloatAsState(if (selected == null || active) 1f else 0f, motion.tween(MotionMillis))
    val interaction = remember { MutableInteractionSource() }
    val fill = interactionFill(interaction)
    Row(modifier.heightIn(min = 48.dp).rowFill({ fill }, ink)
        .clickable(interaction, null, role = Role.Button) { toggle(index) }
        .semantics { this.selected = active; stateDescription = "${chartPercent(point.share)} of total" }
        .padding(vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.size(10.dp).clip(CircleShape).background(color.copy(alpha = lerp(.32f, 1f, emphasis))))
        Spacer(Modifier.width(12.dp))
        ChartText(point.label, 1, MaterialTheme.typography.bodyMedium.copy(color = ink.copy(alpha = lerp(.68f, 1f, emphasis))), Modifier.weight(1f))
        Spacer(Modifier.width(12.dp))
        Text(chartFigure(point.value) + if (active) " · ${chartPercent(point.share)}" else "", style = MaterialTheme.typography.labelMedium.merge(Figures), color = ink.copy(alpha = if (active) 1f else .68f), maxLines = 1)
    }
}

@Composable
internal fun ChartDetails(chart: ChartContent, dismiss: () -> Unit) {
    var selected by remember(chart) { mutableStateOf<Int?>(null) }
    var hidden by remember(chart) { mutableStateOf(emptySet<Int>()) }
    val clipboard = LocalClipboardManager.current
    Dialog(dismiss, DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            CompositionLocalProvider(LocalMessageSurface provides MaterialTheme.colorScheme.surface) {
                Column(Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        SigilIconButton(dismiss) { Glyph("close", 24, "Close chart") }
                        Text("Chart", Modifier.weight(1f), style = MaterialTheme.typography.titleLarge)
                        SigilIconButton({ chart.copyData?.let { clipboard.setText(AnnotatedString(it)) } }, enabled = chart.copyData != null) { Glyph("content_copy", 24, "Copy chart data") }
                    }
                    RichMessageText(chart.title, Modifier.heightIn(max = 100.dp).verticalScroll(rememberScrollState()), MaterialTheme.typography.titleMedium)
                    ChartPlot(chart, hidden, selected, { selected = if (selected == it) null else it }, Modifier.fillMaxWidth().weight(1f), zoomable = true)
                    selected?.let { index -> Box(Modifier.semantics { contentDescription = "Selected point ${index + 1}"; liveRegion = LiveRegionMode.Polite }) { chartDetails(chart, index) } }
                    LazyColumn(Modifier.fillMaxWidth().weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        itemsIndexed(chart.points, key = { i, _ -> i }) { index, _ ->
                            val active = selected == index
                            Row(itemMotion().fillMaxWidth().clip(MaterialTheme.shapes.medium).semantics { this.selected = active }.background(if (active) LocalContentColor.current.copy(alpha = .16f) else Color.Transparent).clickable(role = Role.Button) { selected = index }.padding(8.dp), verticalAlignment = Alignment.CenterVertically) {
                                Column(Modifier.weight(1f)) { chartDetails(chart, index) }
                                SigilIconButton({ hidden = if (index in hidden) hidden - index else hidden + index }) { Glyph(if (index in hidden) "visibility_off" else "visibility", 20, "${if (index in hidden) "Show" else "Hide"} point ${index + 1}") }
                            }
                        }
                    }
                }
            }
        }
    }
}

/** Milliseconds into the arrival reveal; settled when effects are off, reduced, or in the expanded viewer. */
@Composable
private fun chartElapsed(zoomable: Boolean): Float {
    val playback = LocalTextMotion.current?.clock.takeIf { !zoomable && !LocalMotion.current.reduced && LocalAppearance.current.messageEffects }
    return playback?.elapsed ?: TextMotionCap.toFloat()
}
private fun stagger(elapsed: Float, index: Int, count: Int): Float {
    val delay = if (count > 1) index * 4f * MotionStagger / (count - 1) else 0f
    return MotionStandardEasing.transform(((elapsed - delay) / MotionSettle).coerceIn(0f, 1f))
}

@Composable
internal fun ChartPlot(chart: ChartContent, hidden: Set<Int>, selected: Int?, select: (Int) -> Unit, modifier: Modifier, zoomable: Boolean = false) {
    val description = "${chartKindName(chart.kind)} chart. ${chart.summary()}"
    when {
        chart.kind in listOf("pie", "donut") -> CircularPlot(chart, hidden, selected, select, modifier, zoomable, description)
        chart.horizontal -> BarRows(chart, hidden, selected, select, modifier, zoomable, description)
        else -> CartesianPlot(chart, hidden, selected, select, modifier, zoomable, description)
    }
}

@Composable
private fun emphases(chart: ChartContent, selected: Int?): List<Float> {
    val motion = LocalMotion.current
    return chart.points.indices.map { i -> animateFloatAsState(if (selected == null || selected == i) 1f else 0f, motion.tween(MotionMillis)).value }
}

/** Pinch and pan for the expanded viewer; taps fall through to [tap]. */
private fun Modifier.pinchZoom(chart: ChartContent, enabled: Boolean, zoom: MutableFloatState, pan: MutableState<Offset>, tap: (Offset) -> Unit) =
    pointerInput(chart) { detectTapGestures { tap(it) } }
        .pointerInput(chart, enabled) { if (enabled) detectTransformGestures { centroid, change, scale, _ ->
            val next = (zoom.floatValue * scale).coerceIn(1f, 4f)
            val center = Offset(size.width / 2f, size.height / 2f)
            pan.value = ((pan.value + center - centroid) * (next / zoom.floatValue) + centroid - center + change).let { Offset(it.x.coerceIn(-size.width * (next - 1) / 2f, size.width * (next - 1) / 2f), it.y.coerceIn(-size.height * (next - 1) / 2f, size.height * (next - 1) / 2f)) }
            zoom.floatValue = next
        } }
        .graphicsLayer { scaleX = zoom.floatValue; scaleY = zoom.floatValue; translationX = pan.value.x; translationY = pan.value.y }

@Composable
private fun FitButton(zoom: MutableFloatState, pan: MutableState<Offset>) =
    SigilTextButton({ zoom.floatValue = 1f; pan.value = Offset.Zero }, enabled = zoom.floatValue != 1f || pan.value != Offset.Zero) { Text("Fit chart") }

@Composable
private fun CircularPlot(chart: ChartContent, hidden: Set<Int>, selected: Int?, select: (Int) -> Unit, modifier: Modifier, zoomable: Boolean, description: String) {
    val ink = LocalContentColor.current
    val ground = LocalMessageSurface.current.takeOrElse { MaterialTheme.colorScheme.surface }
    val colors = remember(chart, ink, ground) { chartColors(chart.points.size, ink, ground) }
    val elapsed = chartElapsed(zoomable)
    val emphasis = emphases(chart, selected)
    val donut = chart.kind == "donut"
    val zoom = remember(chart) { mutableFloatStateOf(1f) }
    val pan = remember(chart) { mutableStateOf(Offset.Zero) }
    var bounds by remember { mutableStateOf(IntSize.Zero) }
    val latestHidden by rememberUpdatedState(hidden)
    val latestSelect by rememberUpdatedState(select)
    fun choose(raw: Offset) {
        if (bounds.width == 0) return
        val center = Offset(bounds.width / 2f, bounds.height / 2f)
        val cursor = (raw - center - pan.value) / zoom.floatValue
        val radius = min(bounds.width, bounds.height) / 2f
        val distance = cursor.getDistance()
        if (distance > radius || (donut && distance < radius * .6f)) return
        val turn = ((atan2(cursor.y, cursor.x) * 180f / PI.toFloat() + 450f) % 360f) / 360f
        var sum = 0f
        chart.points.indices.firstOrNull { i -> val from = sum; sum += chart.points[i].share; i !in latestHidden && turn >= from && turn <= sum }?.let(latestSelect)
    }
    Column(modifier, horizontalAlignment = Alignment.CenterHorizontally) {
        Box(Modifier.then(if (zoomable) Modifier.weight(1f) else Modifier).widthIn(max = if (zoomable) Dp.Infinity else ChartPie).fillMaxWidth().aspectRatio(1f, zoomable).clipToBounds()
            .onSizeChanged { bounds = it }.pinchZoom(chart, zoomable, zoom, pan, ::choose).semantics { contentDescription = description }, contentAlignment = Alignment.Center) {
            Canvas(Modifier.fillMaxSize()) {
                val progress = MotionStandardEasing.transform((elapsed / ChartReveal).coerceIn(0f, 1f))
                val radius = size.minDimension / 2f
                val topLeft = center - Offset(radius, radius)
                val reach = progress * 360f
                var start = 0f
                val edges = mutableListOf<Float>()
                chart.points.forEachIndexed { i, p ->
                    val sweep = p.share * 360f
                    if (i !in hidden && sweep > 0f) {
                        val visible = min(sweep, reach - start)
                        if (visible > 0f) drawArc(lerp(ground, colors[i], lerp(.32f, 1f, emphasis[i])), start - 90f, visible, true, topLeft, Size(radius * 2, radius * 2))
                        if (start > 0f && start <= reach) edges += start
                    }
                    start += sweep
                }
                if (chart.points.count { it.share > 0f } > 1 && reach >= 360f) edges += 0f
                edges.forEach { a -> val r = (a - 90f) * PI.toFloat() / 180f; drawLine(ground, center, center + Offset(cos(r), sin(r)) * (radius + 1f), 2.dp.toPx()) }
                if (donut) drawCircle(ground, radius * .6f, center)
            }
            if (donut) DonutCentre(chart, selected, elapsed)
        }
        if (zoomable) FitButton(zoom, pan)
    }
}

@Composable
private fun DonutCentre(chart: ChartContent, selected: Int?, elapsed: Float) {
    val ink = LocalContentColor.current
    val figure = selected?.let { chartPercent(chart.points[it].share) } ?: chart.total() ?: return
    val caption: @Composable () -> Unit = {
        if (selected != null) RichMessageText(chart.points[selected].label, Modifier.heightIn(max = with(LocalDensity.current) { MaterialTheme.typography.labelMedium.lineHeight.toDp() }).clipToBounds(),
            MaterialTheme.typography.labelMedium.copy(color = ink.copy(alpha = .68f), textAlign = TextAlign.Center))
        else Text("TOTAL", style = MaterialTheme.typography.labelSmall.copy(letterSpacing = 1.4.sp), color = ink.copy(alpha = .68f), maxLines = 1)
    }
    BoxWithConstraints(Modifier.fillMaxSize(.6f).clip(CircleShape).graphicsLayer { alpha = ((elapsed - ChartReveal * .5f) / (ChartReveal * .5f)).coerceIn(0f, 1f) }, contentAlignment = Alignment.Center) {
        val measurer = rememberTextMeasurer()
        val big = MaterialTheme.typography.displaySmall.merge(Figures)
        val fits = with(LocalDensity.current) { measurer.measure(figure, big, maxLines = 1).size.width.toDp() } <= maxWidth * .84f
        Column(Modifier.widthIn(max = maxWidth * .84f), horizontalAlignment = Alignment.CenterHorizontally) {
            Text(figure, style = if (fits) big else MaterialTheme.typography.titleMedium.merge(Figures), color = ink, maxLines = 1, overflow = TextOverflow.Ellipsis)
            caption()
        }
    }
}

@Composable
private fun BarRows(chart: ChartContent, hidden: Set<Int>, selected: Int?, select: (Int) -> Unit, modifier: Modifier, zoomable: Boolean, description: String) {
    val ink = LocalContentColor.current
    val elapsed = chartElapsed(zoomable)
    val emphasis = emphases(chart, selected)
    val axis = remember(chart) { ChartAxis(chart.points.map { it.y }, chart.zero, emptyList()) }
    val shown = chart.points.indices.filter { it !in hidden }.let { if (zoomable) it else it.take(ChartBarRows) }
    Column(modifier.then(if (zoomable) Modifier.verticalScroll(rememberScrollState()) else Modifier).semantics(mergeDescendants = false) { contentDescription = description }) {
        shown.forEachIndexed { order, i ->
            val point = chart.points[i]
            val interaction = remember { MutableInteractionSource() }
            val fill = interactionFill(interaction)
            Column(Modifier.fillMaxWidth().heightIn(min = 48.dp).rowFill({ fill }, ink).clickable(interaction, null, role = Role.Button) { select(i) }
                .semantics(mergeDescendants = true) { this.selected = selected == i }.padding(vertical = 6.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    ChartText(point.label, 1, MaterialTheme.typography.bodyMedium.copy(color = ink.copy(alpha = lerp(.68f, 1f, emphasis[i]))), Modifier.weight(1f))
                    Spacer(Modifier.width(12.dp))
                    Text(chartFigure(point.value), style = MaterialTheme.typography.labelMedium.merge(Figures), color = ink.copy(alpha = lerp(.68f, 1f, emphasis[i])), maxLines = 1)
                }
                Canvas(Modifier.fillMaxWidth().height(8.dp)) {
                    val grow = stagger(elapsed, order, shown.size)
                    val zero = axis.zero * size.width
                    val end = zero + (axis.positions[i] * size.width - zero) * grow
                    drawRoundRect(ink.copy(alpha = .08f), cornerRadius = CornerRadius(size.height / 2))
                    val left = min(zero, end); val width = abs(end - zero)
                    if (width > .5f) drawRoundRect(ink.copy(alpha = lerp(.32f, if (selected == i) 1f else .72f, emphasis[i])), Offset(left, 0f), Size(max(width, size.height), size.height), CornerRadius(size.height / 2))
                    if (axis.zero > 0f) drawLine(ink.copy(alpha = .4f), Offset(zero, -2.dp.toPx()), Offset(zero, size.height + 2.dp.toPx()), 1.dp.toPx())
                }
            }
        }
        val rest = chart.points.size - hidden.size - shown.size
        if (rest > 0) Text("+$rest more", Modifier.padding(top = 4.dp), style = MaterialTheme.typography.labelMedium, color = ink.copy(alpha = .68f), maxLines = 1)
    }
}

@Composable
private fun CartesianPlot(chart: ChartContent, hidden: Set<Int>, selected: Int?, select: (Int) -> Unit, modifier: Modifier, zoomable: Boolean, description: String) {
    val ink = LocalContentColor.current
    val quiet = ink.copy(alpha = .68f)
    val ground = LocalMessageSurface.current.takeOrElse { MaterialTheme.colorScheme.surface }
    val density = LocalDensity.current
    val elapsed = chartElapsed(zoomable)
    val emphasis = emphases(chart, selected)
    val scatter = chart.kind == "scatter"
    val bar = chart.kind == "bar"
    val y = remember(chart) { chart.valueAxis() }
    val x = remember(chart) { if (scatter) chart.xAxis() else null }
    val measurer = rememberTextMeasurer()
    val tickStyle = MaterialTheme.typography.labelSmall.merge(Figures).copy(color = quiet)
    val valueStyle = MaterialTheme.typography.labelMedium.merge(Figures)
    val yLabels = remember(y, tickStyle, measurer) { y.ticks.map { measurer.measure(it.second, tickStyle, maxLines = 1) } }
    val xLabels = remember(x, tickStyle, measurer) { x?.ticks?.map { measurer.measure(it.second, tickStyle, maxLines = 1) }.orEmpty() }
    val values = remember(chart, valueStyle, measurer, ink) { chart.points.map { measurer.measure(chartFigure(if (scatter) "${it.xValue.orEmpty()}" else it.value, false), valueStyle.copy(color = ink), maxLines = 1) } }
    val callouts = remember(chart, valueStyle, measurer, ink) { chart.points.map { p -> measurer.measure(if (scatter) "${chartFigure(p.xValue.orEmpty(), false)}, ${chartFigure(p.value)}" else chartFigure(p.value), valueStyle.copy(color = ink), maxLines = 1) } }
    val tickHeight = yLabels.maxOfOrNull { it.size.height } ?: 0
    val valueHeight = values.maxOfOrNull { it.size.height } ?: 0
    val negative = y.positions.indices.any { y.positions[it] < y.zero - 1e-4f }
    // Everything in pixels comes from one geometry so marks, targets and labels agree.
    val gutter = with(density) { ((yLabels.maxOfOrNull { it.size.width } ?: 0).toDp() + 8.dp) }
    val top = with(density) { if (bar) valueHeight.toDp() + 6.dp else max(tickHeight.toDp() / 2, 8.dp) }
    val bottom = with(density) { (if (bar && negative) valueHeight.toDp() + 6.dp else max(tickHeight.toDp() / 2, 6.dp)) + if (scatter) tickHeight.toDp() + 6.dp else 0.dp }
    val inset = if (bar) 0.dp else 8.dp
    val zoom = remember(chart) { mutableFloatStateOf(1f) }
    val pan = remember(chart) { mutableStateOf(Offset.Zero) }
    val count = chart.points.size
    // Bars sit in band centres; a line or area runs edge to edge.
    fun px(i: Int, left: Float, width: Float) = left + (if (scatter) x!!.positions[i] else if (bar || count == 1) (i + .5f) / count else i / (count - 1f)) * width
    fun py(i: Int, top: Float, height: Float) = top + (1f - y.positions[i]) * height
    Column(modifier) {
        BoxWithConstraints(Modifier.fillMaxWidth().then(if (zoomable) Modifier.weight(1f) else Modifier.height(top + ChartPlotHeight + bottom)).clipToBounds()) {
            val plotLeft = with(density) { (gutter + inset).toPx() }
            val plotRight = with(density) { (maxWidth - inset).toPx() }
            val plotTop = with(density) { top.toPx() }
            val plotBottom = with(density) { (maxHeight - bottom).toPx() }
            val plotWidth = plotRight - plotLeft; val plotHeight = plotBottom - plotTop
            fun tap(at: Offset) {
                val cursor = (at - Offset(constraints.maxWidth / 2f, constraints.maxHeight / 2f) - pan.value) / zoom.floatValue + Offset(constraints.maxWidth / 2f, constraints.maxHeight / 2f)
                chart.points.indices.filter { it !in hidden }.minByOrNull { i ->
                    if (scatter) (Offset(px(i, plotLeft, plotWidth), py(i, plotTop, plotHeight)) - cursor).getDistanceSquared() else abs(px(i, plotLeft, plotWidth) - cursor.x)
                }?.let(select)
            }
            Box(Modifier.fillMaxSize().pinchZoom(chart, zoomable, zoom, pan, ::tap)) {
                Canvas(Modifier.fillMaxSize().semantics { contentDescription = description }) {
                    val hair = 1.dp.toPx()
                    y.ticks.forEachIndexed { t, (at, _) ->
                        val gy = plotTop + (1f - at) * plotHeight
                        drawLine(ink.copy(alpha = .08f), Offset(gutter.toPx(), gy), Offset(size.width, gy), hair)
                        val label = yLabels[t]
                        drawText(label, topLeft = Offset(gutter.toPx() - 8.dp.toPx() - label.size.width, gy - label.size.height / 2f))
                    }
                    val zeroY = plotTop + (1f - y.zero) * plotHeight
                    if (y.zero > 1e-4f) drawLine(ink.copy(alpha = .32f), Offset(gutter.toPx(), zeroY), Offset(size.width, zeroY), hair)
                    x?.ticks?.forEachIndexed { t, (at, _) ->
                        val label = xLabels[t]
                        val gx = plotLeft + at * plotWidth
                        drawText(label, topLeft = Offset((gx - label.size.width / 2f).coerceIn(gutter.toPx(), size.width - label.size.width), size.height - label.size.height))
                    }
                    val visible = chart.points.indices.filter { it !in hidden }
                    if (bar) {
                        val band = plotWidth / count
                        val thick = min(band * .56f, 32.dp.toPx())
                        visible.forEach { i ->
                            val grow = stagger(elapsed, i, count)
                            val cx = px(i, plotLeft, plotWidth)
                            val end = zeroY + (py(i, plotTop, plotHeight) - zeroY) * grow
                            val up = end <= zeroY
                            val h = abs(end - zeroY)
                            val alpha = lerp(.32f, if (selected == i) 1f else .72f, emphasis[i])
                            if (h > .5f) {
                                val r = CornerRadius(min(6.dp.toPx(), min(h, thick / 2)))
                                val rect = if (up) RoundRect(cx - thick / 2, end, cx + thick / 2, zeroY, r, r, CornerRadius.Zero, CornerRadius.Zero)
                                    else RoundRect(cx - thick / 2, zeroY, cx + thick / 2, end, CornerRadius.Zero, CornerRadius.Zero, r, r)
                                drawPath(Path().apply { addRoundRect(rect) }, ink.copy(alpha = alpha))
                            }
                            val label = values[i]
                            if (grow > 0f && label.size.width <= band + 4.dp.toPx() && count <= 7) {
                                val ly = if (up) end - 4.dp.toPx() - label.size.height else end + 4.dp.toPx()
                                drawText(label, topLeft = Offset(cx - label.size.width / 2f, ly), alpha = grow * lerp(.68f, 1f, emphasis[i]))
                            }
                        }
                    } else {
                        val wipe = if (scatter) size.width else plotLeft + plotWidth * MotionStandardEasing.transform((elapsed / ChartReveal).coerceIn(0f, 1f))
                        val points = visible.map { Offset(px(it, plotLeft, plotWidth), py(it, plotTop, plotHeight)) }
                        selected?.takeIf { it in visible }?.let { s ->
                            val sx = px(s, plotLeft, plotWidth)
                            drawLine(ink.copy(alpha = .16f), Offset(sx, py(s, plotTop, plotHeight)), Offset(sx, plotBottom), hair)
                        }
                        if (!scatter && points.size > 1) clipRect(right = wipe) {
                            val path = Path().apply { points.forEachIndexed { k, p -> if (k == 0) moveTo(p.x, p.y) else lineTo(p.x, p.y) } }
                            if (chart.kind == "area") drawPath(Path().apply { addPath(path); lineTo(points.last().x, zeroY); lineTo(points.first().x, zeroY); close() }, ink.copy(alpha = .12f))
                            drawPath(path, ink, style = Stroke(2.dp.toPx(), cap = StrokeCap.Round, join = StrokeJoin.Round))
                        }
                        val dots = scatter || visible.size <= 12
                        visible.forEachIndexed { k, i ->
                            val p = points[k]
                            val grow = if (scatter) stagger(elapsed, k, visible.size) else ((wipe - p.x) / 12.dp.toPx() + 1f).coerceIn(0f, 1f)
                            if (grow <= 0f || !(dots || selected == i)) return@forEachIndexed
                            val r = (if (scatter) 5.dp else 3.5.dp).toPx() + (if (selected == i) 1.5.dp.toPx() else 0f)
                            val alpha = if (scatter) lerp(.32f, 1f, emphasis[i]) else 1f
                            drawCircle(ground, (r + 2.dp.toPx()) * grow, p)
                            drawCircle(lerp(ground, ink, alpha), r * grow, p)
                        }
                        selected?.takeIf { it in visible }?.let { s ->
                            val p = points[visible.indexOf(s)]
                            val label = callouts[s]
                            val w = label.size.width + 12.dp.toPx(); val h = label.size.height + 4.dp.toPx()
                            val left = (p.x - w / 2).coerceIn(0f, size.width - w)
                            val above = p.y - 10.dp.toPx() - h
                            val tip = if (above >= 0f) above else p.y + 10.dp.toPx()
                            drawRoundRect(ground, Offset(left, tip), Size(w, h), CornerRadius(h / 2))
                            drawRoundRect(ink.copy(alpha = .1f), Offset(left, tip), Size(w, h), CornerRadius(h / 2))
                            drawText(label, topLeft = Offset(left + 6.dp.toPx(), tip + 2.dp.toPx()))
                        }
                    }
                }
                // One focusable target per mark: a full-height band, or a 40dp disc on a scatter point.
                Layout({ chart.points.indices.forEach { i ->
                    val interaction = remember { MutableInteractionSource() }
                    val fill = interactionFill(interaction)
                    Box(Modifier.semantics { contentDescription = "${chart.pointName(i)}: ${if (scatter) "x ${chartFigure(chart.points[i].xValue.orEmpty(), false)}, " else ""}${chartFigure(chart.points[i].value)}"; this.selected = selected == i }
                        .drawBehind { if (fill > 0f) drawRoundRect(ink.copy(alpha = fill), cornerRadius = CornerRadius(if (scatter) size.minDimension / 2 else min(14.dp.toPx(), size.minDimension / 2))) }
                        .clickable(interaction, null, enabled = i !in hidden, role = Role.Button) { select(i) })
                } }, Modifier.fillMaxSize()) { measurables, constraints ->
                    val disc = 40.dp.roundToPx()
                    val band = max(1, (plotWidth / if (bar) count else max(1, count - 1)).roundToInt())
                    // Bands stay right of the tick gutter and inside the card.
                    val spans = chart.points.indices.map { i -> val c = px(i, plotLeft, plotWidth); (c - band / 2f).roundToInt().coerceAtLeast(gutter.roundToPx()) to (c + band / 2f).roundToInt().coerceAtMost(constraints.maxWidth) }
                    val placed = measurables.mapIndexed { i, m -> if (scatter) m.measure(Constraints.fixed(disc, disc)) else m.measure(Constraints.fixed(max(1, spans[i].second - spans[i].first), constraints.maxHeight)) }
                    layout(constraints.maxWidth, constraints.maxHeight) {
                        placed.forEachIndexed { i, p ->
                            if (scatter) p.place((px(i, plotLeft, plotWidth) - disc / 2f).roundToInt(), (py(i, plotTop, plotHeight) - disc / 2f).roundToInt())
                            else p.place(spans[i].first, 0)
                        }
                    }
                }
            }
        }
        if (!scatter) CategoryLabels(chart, gutter, inset, bar, quiet)
        if (zoomable) FitButton(zoom, pan)
    }
}

/** Category names under their marks, thinned so each keeps about 48dp and held inside the card; concealed labels keep their own reveals. */
@Composable
private fun CategoryLabels(chart: ChartContent, gutter: Dp, inset: Dp, bands: Boolean, quiet: Color) {
    val style = MaterialTheme.typography.labelSmall.copy(color = quiet, textAlign = TextAlign.Center)
    val line = with(LocalDensity.current) { style.lineHeight.toDp() }
    val count = chart.points.size
    BoxWithConstraints(Modifier.fillMaxWidth().padding(top = 4.dp).clearAndSetSemantics { }) {
        val left = gutter + inset
        val step = (maxWidth - left - inset) / if (bands || count == 1) count.toFloat() else count - 1f
        val every = max(1, ceil(48.dp / step).toInt())
        val shown = chart.points.indices.filter { it % every == 0 }
        Layout({ shown.forEach { i ->
            val p = chart.points[i]
            if (p.label.concealed()) RichMessageText(p.label, Modifier.heightIn(max = line).clipToBounds(), style)
            else Text(p.label.text, style = style, maxLines = 1, softWrap = false, overflow = TextOverflow.Ellipsis)
        } }, Modifier.fillMaxWidth()) { measurables, constraints ->
            val slot = max(1, (step * min(every, 2) - 4.dp).roundToPx())
            val placed = measurables.map { it.measure(Constraints(maxWidth = slot)) }
            layout(constraints.maxWidth, placed.maxOfOrNull { it.height } ?: 0) {
                placed.forEachIndexed { k, p ->
                    val centre = left.toPx() + step.toPx() * (if (bands || count == 1) shown[k] + .5f else shown[k].toFloat())
                    p.place((centre - p.width / 2f).roundToInt().coerceIn(0, max(0, constraints.maxWidth - p.width)), 0)
                }
            }
        }
    }
}
