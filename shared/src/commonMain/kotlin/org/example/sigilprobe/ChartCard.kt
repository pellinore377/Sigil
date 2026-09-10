package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.gestures.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.*
import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.*
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.*
import androidx.compose.ui.unit.*
import androidx.compose.ui.window.*
import kotlin.math.*

@Composable
internal fun ChartCard(chart: ChartContent) {
    var expanded by remember(chart) { mutableStateOf(false) }
    var selected by remember(chart) { mutableStateOf<Int?>(null) }
    var hidden by remember(chart) { mutableStateOf(emptySet<Int>()) }
    val clipboard = LocalClipboardManager.current
    @Composable fun details(index: Int) {
        val point = chart.points[index]
        Column(Modifier.fillMaxWidth().padding(vertical = 4.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) { Text("${index + 1}.", style = MaterialTheme.typography.labelLarge); RichMessageText(point.label, Modifier.weight(1f), MaterialTheme.typography.bodyMedium) }
            Text(listOfNotNull(point.xValue?.let { "x = $it" }, point.value, point.percent.takeIf { chart.kind in listOf("pie", "donut") }?.let { "$it% of total" }).joinToString(" · "), style = MaterialTheme.typography.labelMedium)
        }
    }
    Column(Modifier.widthIn(min = 200.dp, max = 280.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        RichMessageText(chart.title, style = MaterialTheme.typography.titleMedium)
        ChartPlot(chart, emptySet(), selected, { selected = it }, Modifier.fillMaxWidth().height(200.dp))
        selected?.let { details(it) } ?: chart.points.indices.take(2).forEach { details(it) }
        SigilTextButton({ expanded = true }) { Glyph("open_in_full", 18); Spacer(Modifier.width(8.dp)); Text("Open chart · ${chart.points.size} points") }
    }
    if (expanded) Dialog({ expanded = false }, DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            CompositionLocalProvider(LocalMessageSurface provides MaterialTheme.colorScheme.surface) {
                Column(Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        SigilIconButton({ expanded = false }) { Glyph("close", 24, "Close chart") }
                        Text("Chart", Modifier.weight(1f), style = MaterialTheme.typography.titleLarge)
                        SigilIconButton({ chart.copyData?.let { clipboard.setText(AnnotatedString(it)) } }, enabled = chart.copyData != null) { Glyph("content_copy", 24, "Copy chart data") }
                    }
                    RichMessageText(chart.title, Modifier.heightIn(max = 100.dp).verticalScroll(rememberScrollState()), MaterialTheme.typography.titleMedium)
                    ChartPlot(chart, hidden, selected, { selected = it }, Modifier.fillMaxWidth().weight(1f), zoomable = true)
                    selected?.let { index -> Box(Modifier.semantics { contentDescription = "Selected point ${index + 1}"; liveRegion = LiveRegionMode.Polite }) { details(index) } }
                    LazyColumn(Modifier.fillMaxWidth().weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        itemsIndexed(chart.points, key = { i, _ -> i }) { index, _ ->
                            Row(Modifier.fillMaxWidth().background(if (selected == index) MaterialTheme.colorScheme.surfaceVariant else Color.Transparent, MaterialTheme.shapes.medium).padding(8.dp), verticalAlignment = Alignment.CenterVertically) {
                                Column(Modifier.weight(1f).clickable(role = Role.Button) { selected = index }) { details(index) }
                                SigilIconButton({ hidden = if (index in hidden) hidden - index else hidden + index }) { Glyph(if (index in hidden) "visibility_off" else "visibility", 20, "${if (index in hidden) "Show" else "Hide"} point ${index + 1}") }
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun ChartPlot(chart: ChartContent, hidden: Set<Int>, selected: Int?, select: (Int) -> Unit, modifier: Modifier, zoomable: Boolean = false) {
    var zoom by remember(chart) { mutableFloatStateOf(1f) }
    var pan by remember(chart) { mutableStateOf(Offset.Zero) }
    var bounds by remember { mutableStateOf(IntSize.Zero) }
    val circular = chart.kind in listOf("pie", "donut")
    val ink = LocalContentColor.current
    val background = LocalMessageSurface.current.takeOrElse { MaterialTheme.colorScheme.surface }
    val colors = remember(chart, background) { chart.points.indices.map { textColor(listOf("purple2", "blue2", "green2", "orange2", "pink2", "cyan2")[it % 6], background) } }
    val measurer = rememberTextMeasurer()
    val labelStyle = MaterialTheme.typography.labelSmall.copy(color = ink)
    val labels = remember(chart, labelStyle, measurer) { chart.points.indices.map { measurer.measure(AnnotatedString("${it + 1}"), style = labelStyle) } }
    val latestSelect by rememberUpdatedState(select)
    val latestHidden by rememberUpdatedState(hidden)
    val axisColor = ink.copy(alpha = .25f)
    fun point(index: Int, width: Float, height: Float): Offset {
        val value = chart.points[index]
        return if (chart.horizontal) Offset(value.y * width, value.x * height) else Offset(value.x * width, (1 - value.y) * height)
    }
    fun choose(raw: Offset) {
        if (bounds.width == 0 || bounds.height == 0) return
        val center = Offset(bounds.width / 2f, bounds.height / 2f)
        val cursor = (raw - center - pan) / zoom + center
        val available = chart.points.indices.filter { it !in latestHidden }
        val index = if (circular) {
            val distance = (cursor - center).getDistance()
            val radius = min(bounds.width, bounds.height) * if (chart.kind == "donut") .36f else .42f
            if (distance > radius * if (chart.kind == "donut") 1.21f else 1f) return
            if (chart.kind == "donut" && distance < radius * .79f) return
            val angle = ((atan2(cursor.y - center.y, cursor.x - center.x) * 180f / PI.toFloat() + 450f) % 360f) / 360f
            var sum = 0f
            chart.points.indices.firstOrNull { i -> val from = sum; sum += chart.points[i].share; i !in latestHidden && angle >= from && angle <= sum }
        } else available.minByOrNull { i ->
            val p = point(i, bounds.width.toFloat(), bounds.height.toFloat())
            if (chart.kind in listOf("bar", "line", "area")) abs(if (chart.horizontal) p.y - cursor.y else p.x - cursor.x) else (p - cursor).getDistanceSquared()
        }
        index?.let(latestSelect)
    }
    Column(modifier, verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(Modifier.weight(1f).fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        if (!circular && !chart.horizontal) Column(Modifier.width(44.dp).fillMaxHeight(), verticalArrangement = Arrangement.SpaceBetween) {
            listOf(chart.yTicks.lastOrNull(), chart.yTicks.getOrNull(2), chart.yTicks.firstOrNull()).forEach { Text(it.orEmpty(), style = MaterialTheme.typography.labelSmall, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis) }
        }
        Box(Modifier.weight(1f).fillMaxHeight().clipToBounds().onSizeChanged { bounds = it }
            .pointerInput(chart) { detectTapGestures { choose(it) } }
            .pointerInput(chart, zoomable) { detectTransformGestures { centroid, change, scale, _ ->
                if (zoomable && (scale != 1f || zoom > 1f)) {
                    val next = (zoom * scale).coerceIn(1f, 4f)
                    val center = Offset(size.width / 2f, size.height / 2f)
                    pan = ((pan + center - centroid) * (next / zoom) + centroid - center + change).let { Offset(it.x.coerceIn(-size.width * (next - 1) / 2f, size.width * (next - 1) / 2f), it.y.coerceIn(-size.height * (next - 1) / 2f, size.height * (next - 1) / 2f)) }
                    zoom = next
                } else choose(centroid)
            } }) {
            Canvas(Modifier.fillMaxSize().graphicsLayer { scaleX = zoom; scaleY = zoom; translationX = pan.x; translationY = pan.y }
                .semantics { contentDescription = "${chart.kind.replaceFirstChar { it.uppercase() }} chart, ${chart.points.size} points. Values are listed below." }) {
                val radius = min(size.width, size.height) * if (chart.kind == "donut") .36f else .42f
                fun marker(index: Int, at: Offset) {
                    val color = colors[index]
                    when (index % 3) {
                        0 -> drawCircle(color, 4.dp.toPx(), at)
                        1 -> drawRect(color, at - Offset(4.dp.toPx(), 4.dp.toPx()), Size(8.dp.toPx(), 8.dp.toPx()))
                        else -> drawPath(Path().apply { moveTo(at.x, at.y - 5.dp.toPx()); lineTo(at.x + 5.dp.toPx(), at.y + 4.dp.toPx()); lineTo(at.x - 5.dp.toPx(), at.y + 4.dp.toPx()); close() }, color)
                    }
                    if (selected == index) drawCircle(ink, 8.dp.toPx(), at, style = Stroke(2.dp.toPx()))
                }
                if (circular) {
                    var start = -90f
                    chart.points.forEachIndexed { index, p ->
                        val sweep = p.share * 360f
                        if (index !in hidden) {
                            drawArc(colors[index], start, sweep, chart.kind == "pie", topLeft = center - Offset(radius, radius), size = Size(radius * 2, radius * 2), style = if (chart.kind == "pie") Fill else Stroke(radius * .42f))
                            if (p.share > .04f || selected == index) {
                                val angle = (start + sweep / 2) * PI.toFloat() / 180f
                                val at = center + Offset(cos(angle), sin(angle)) * (radius * if (chart.kind == "pie") .63f else 1f)
                                drawCircle(background, 11.dp.toPx(), at)
                                drawText(labels[index], topLeft = at - Offset(labels[index].size.width / 2f, labels[index].size.height / 2f))
                            }
                        }
                        start += sweep
                    }
                } else {
                    val zero = if (chart.horizontal) Offset(chart.zero * size.width, 0f) else Offset(0f, (1 - chart.zero) * size.height)
                    drawLine(axisColor, zero, zero + if (chart.horizontal) Offset(0f, size.height) else Offset(size.width, 0f), 1.dp.toPx())
                    if (chart.kind == "area") {
                        chart.points.indices.zipWithNext().forEach { (a, b) -> if (a !in hidden && b !in hidden) {
                            val from = point(a, size.width, size.height); val to = point(b, size.width, size.height)
                            drawPath(Path().apply { moveTo(from.x, zero.y); lineTo(from.x, from.y); lineTo(to.x, to.y); lineTo(to.x, zero.y); close() }, colors[a].copy(alpha = .18f))
                        } }
                    }
                    chart.points.forEachIndexed { index, _ -> if (index !in hidden) {
                        val at = point(index, size.width, size.height)
                        if (chart.kind == "bar") {
                            val band = (if (chart.horizontal) size.height else size.width) / chart.points.size
                            val thickness = (band * .7f).coerceAtMost(36.dp.toPx())
                            val top = if (chart.horizontal) Offset(min(zero.x, at.x), at.y - thickness / 2) else Offset(at.x - thickness / 2, min(zero.y, at.y))
                            val rect = if (chart.horizontal) Size(abs(at.x - zero.x).coerceAtLeast(1f), thickness) else Size(thickness, abs(at.y - zero.y).coerceAtLeast(1f))
                            drawRect(colors[index], top, rect)
                            if (selected == index) drawRect(ink, top, rect, style = Stroke(2.dp.toPx()))
                            if (band >= 24.dp.toPx() || selected == index) {
                                val insetX = min(12.dp.toPx(), size.width / 2); val insetY = min(12.dp.toPx(), size.height / 2)
                                val atLabel = if (chart.horizontal) Offset(((zero.x + at.x) / 2).coerceIn(insetX, size.width - insetX), at.y) else Offset(at.x, ((zero.y + at.y) / 2).coerceIn(insetY, size.height - insetY))
                                drawCircle(background, 11.dp.toPx(), atLabel)
                                drawText(labels[index], topLeft = atLabel - Offset(labels[index].size.width / 2f, labels[index].size.height / 2f))
                            }
                        } else {
                            if (chart.kind in listOf("line", "area") && index > 0 && index - 1 !in hidden) drawLine(colors[index], point(index - 1, size.width, size.height), at, 2.dp.toPx())
                            marker(index, at)
                        }
                    } }
                }
            }
        }
        }
        if (chart.horizontal || chart.kind == "scatter") Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            val ticks = if (chart.horizontal) chart.yTicks else chart.xTicks
            Text(ticks.firstOrNull().orEmpty(), style = MaterialTheme.typography.labelSmall, maxLines = 1, modifier = Modifier.weight(1f))
            Text(ticks.lastOrNull().orEmpty(), style = MaterialTheme.typography.labelSmall, maxLines = 1, modifier = Modifier.weight(1f), textAlign = androidx.compose.ui.text.style.TextAlign.End)
        }
        if (zoomable) SigilTextButton({ zoom = 1f; pan = Offset.Zero }, enabled = zoom != 1f || pan != Offset.Zero) { Text("Fit chart") }
    }
}
