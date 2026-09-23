package org.sigil

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.*
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.layout.SubcomposeLayout
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.*

private val TableGap = 12.dp
private val TableMinColumn = 56.dp
private val TableWordCap = 120.dp
private const val TableRows = 5

// Columns take their natural width; when the card is too narrow the widest give way first, and what cannot fit is counted, never scrolled.
internal fun tableWidths(natural: List<Float>, floor: List<Float>, available: Float, gap: Float, numeric: List<Boolean> = emptyList()): List<Float> {
    var count = natural.size
    while (count > 1 && floor.take(count).sum() + gap * (count - 1) > available) count--
    val want = natural.take(count)
    val least = floor.take(count).zip(want) { f, n -> minOf(f, n) }
    val space = available - gap * (count - 1)
    // Slack goes to the text columns, so figures stay packed against the end edge.
    if (want.sum() <= space) {
        val takes = want.indices.map { numeric.getOrElse(it) { false } }.let { flags -> if (flags.all { it }) flags.map { true } else flags.map { !it } }
        val total = want.indices.filter { takes[it] }.sumOf { want[it].toDouble() }.toFloat().coerceAtLeast(1f)
        return want.indices.map { want[it] + if (takes[it]) (space - want.sum()) * want[it] / total else 0f }
    }
    var low = 0f; var high = want.maxOrNull() ?: 0f
    repeat(24) { val cap = (low + high) / 2; if (want.indices.sumOf { maxOf(least[it], minOf(want[it], cap)).toDouble() } > space) high = cap else low = cap }
    return want.indices.map { maxOf(least[it], minOf(want[it], low)) }
}

// Rows fade and rise in one stagger apart, the header first; the fifth step holds the rest.
internal const val TableMotionMillis = MotionStagger * 4 + MotionSettle

private fun tableArrival(elapsed: Float?, row: Int) =
    if (elapsed == null) 1f else MotionStandardEasing.transform(((elapsed - MotionStagger * minOf(row, 4)) / MotionSettle).coerceIn(0f, 1f))

@Composable
internal fun TableCard(table: TableContent) {
    val ink = LocalContentColor.current
    val quiet = ink.copy(alpha = .68f)
    val body = MaterialTheme.typography.bodyMedium
    val head = MaterialTheme.typography.labelMedium
    val motion = LocalMotion.current
    val playback = LocalTextMotion.current?.clock.takeIf { !motion.reduced && LocalAppearance.current.messageEffects }
    val numeric = table.columns.indices.map { table.numericOrder.getOrNull(it) != null }
    val shown = table.rows.take(TableRows)
    val rules = remember { FloatArray(TableRows) }
    val line = ink.copy(alpha = .08f)
    // The foot is composed in the same pass as the cells, so the fitted column count never lands a frame late.
    SubcomposeLayout(Modifier.widthIn(min = MessageCardMinWidth, max = MessageCardMaxWidth).fillMaxWidth().padding(vertical = 4.dp)
        .animateContentSize(motion.tween(MotionMillis))
        .semantics { contentDescription = "Table, ${table.rows.size} ${if (table.rows.size == 1) "row" else "rows"}, ${table.columns.size} ${if (table.columns.size == 1) "column" else "columns"}" }
        .drawBehind { val t = playback?.elapsed; for (i in shown.indices) drawRect(line.copy(alpha = line.alpha * tableArrival(t, i + 1)), Offset(0f, rules[i]), Size(size.width, 1.dp.toPx())) }) { constraints ->
        val cells = subcompose(0) {
            CompositionLocalProvider(LocalContentColor provides quiet) {
                table.columns.forEachIndexed { column, label -> TableCell(label, head, numeric[column], 2) }
            }
            shown.forEach { row ->
                table.columns.indices.forEach { column ->
                    val cell = row.getOrNull(column)
                    if (cell == null || cell.text.isBlank()) Text("—", Modifier.clearAndSetSemantics {}, style = body, color = quiet, textAlign = if (numeric[column]) TextAlign.End else TextAlign.Start)
                    else TableCell(cell, body, numeric[column], 3)
                }
            }
        }
        val count = table.columns.size
        val lines = shown.size + 1
        val natural = (0 until count).map { c -> (0 until lines).maxOf { cells[it * count + c].maxIntrinsicWidth(Constraints.Infinity) }.toFloat() }
        val words = (0 until count).map { c -> (0 until lines).maxOf { cells[it * count + c].minIntrinsicWidth(Constraints.Infinity) }.toFloat().coerceIn(TableMinColumn.toPx(), TableWordCap.toPx()) }
        val widths = tableWidths(natural, words, constraints.maxWidth.toFloat(), TableGap.toPx(), numeric).map { it.toInt() }
        val gap = TableGap.roundToPx()
        val placed = List(lines) { r -> List(widths.size) { c -> cells[r * count + c].measure(Constraints.fixedWidth(widths[c])) } }
        val tops = IntArray(lines)
        var y = 0
        placed.forEachIndexed { r, row ->
            if (r > 0) { rules[r - 1] = y.toFloat(); y += 1.dp.roundToPx() + 10.dp.roundToPx() }
            tops[r] = y
            y += (row.maxOfOrNull { it.height } ?: 0) + when { r == 0 -> 8.dp.roundToPx(); r < lines - 1 -> 10.dp.roundToPx(); else -> 0 }
        }
        val meta = listOfNotNull(
            if (table.rows.size > shown.size) "${shown.size} of ${table.rows.size} rows" else null,
            if (widths.size < count) "${widths.size} of $count columns" else null,
        )
        val foot = if (meta.isEmpty()) null else subcompose(1) { Text(meta.joinToString(" · "), style = head, color = quiet, maxLines = 2) }
            .first().measure(Constraints(maxWidth = constraints.maxWidth))
        val footTop = y + 8.dp.roundToPx()
        val rise = 6.dp.toPx()
        layout(constraints.maxWidth, if (foot == null) y else footTop + foot.height) {
            placed.forEachIndexed { r, row ->
                var x = 0
                row.forEachIndexed { c, cell ->
                    cell.placeRelativeWithLayer(x, tops[r]) { val shown = tableArrival(playback?.elapsed, r); alpha = shown; translationY = (1f - shown) * rise }
                    x += widths[c] + gap
                }
            }
            foot?.placeRelativeWithLayer(0, footTop) { val shown = tableArrival(playback?.elapsed, lines); alpha = shown; translationY = (1f - shown) * rise }
        }
    }
}

// Plain cells ellipsize; rich cells keep their spans and clip on a line boundary.
@Composable
private fun TableCell(cell: RichText, style: TextStyle, numeric: Boolean, lines: Int, modifier: Modifier = Modifier) {
    val shaped = if (numeric) style.copy(fontFeatureSettings = "tnum, lnum", textAlign = TextAlign.End) else style
    if (cell.spans.isEmpty() && cell.blocks.isEmpty() && cell.motion.isEmpty()) Text(cell.text, modifier, style = shaped, maxLines = lines, overflow = TextOverflow.Ellipsis)
    else {
        val line = with(LocalDensity.current) { style.lineHeight.toDp() }
        RichMessageText(cell, modifier.heightIn(max = line * lines).clipToBounds(), shaped)
    }
}

@Composable
private fun TableHeader(table: TableContent, width: Dp, full: Boolean, sort: Pair<Int, Boolean>?, onSort: (Pair<Int, Boolean>?) -> Unit) {
    Row(Modifier.width(width).background(LocalContentColor.current.copy(alpha = .07f))) {
        table.columns.forEachIndexed { column, label ->
            val sortable = full && table.numericOrder.getOrNull(column) != null
            Column(Modifier.width(160.dp).heightIn(min = 56.dp).then(if (sortable) Modifier.clickable(role = Role.Button) {
                onSort(when { sort?.first != column -> column to false; sort?.second == false -> column to true; else -> null })
            } else Modifier).semantics { if (sortable) { contentDescription = "Sort column ${column + 1}"; stateDescription = if (sort?.first != column) "Original order" else if (sort?.second == true) "Descending" else "Ascending" } }.padding(12.dp)) {
                RichMessageText(label, Modifier.heightIn(max = if (full) 160.dp else 72.dp).clipToBounds(), MaterialTheme.typography.labelLarge)
                if (sortable) Glyph(if (sort?.first == column && sort?.second == true) "arrow_downward" else "arrow_upward", 16)
            }
        }
    }
}

// The hover highlight belongs to the whole row, so the cells draw no indication of their own.
@Composable
private fun TableRow(table: TableContent, index: Int, width: Dp, full: Boolean, open: ((Int, Int) -> Unit)?) {
    val source = remember { MutableInteractionSource() }
    val hovered by source.collectIsHoveredAsState()
    val stripe = if (index % 2 == 0) Color.Transparent else LocalContentColor.current.copy(alpha = .06f)
    val tint by animateColorAsState(if (hovered) LocalContentColor.current.copy(alpha = .13f) else stripe, LocalMotion.current.tween(MotionFeedback), label = "Table row hover")
    Row(Modifier.width(width).hoverable(source).background(tint)) {
        table.rows[index].forEachIndexed { column, cell ->
            Box(Modifier.width(160.dp).heightIn(min = 56.dp).then(if (open == null) Modifier else Modifier.combinedClickable(interactionSource = source, indication = null, role = Role.Button, onClick = { open(index, column) }, onLongClickLabel = "Open cell", onLongClick = { open(index, column) }))
                .semantics { contentDescription = "Row ${index + 1}, column ${column + 1}" }.padding(12.dp)) {
                RichMessageText(cell, Modifier.fillMaxWidth().heightIn(max = if (full) 160.dp else 96.dp).clipToBounds(),
                    MaterialTheme.typography.bodyMedium.copy(textAlign = if (table.numericOrder.getOrNull(column) != null) TextAlign.End else TextAlign.Start))
            }
        }
    }
}

@Composable
internal fun TableDetails(table: TableContent, dismiss: () -> Unit) {
    var selected by remember(table) { mutableStateOf<Pair<Int, Int>?>(null) }
    var sort by remember(table) { mutableStateOf<Pair<Int, Boolean>?>(null) }
    val clipboard = LocalClipboardManager.current
    val order = remember(table, sort) { sort?.let { (column, descending) -> table.numericOrder.getOrNull(column)?.let { if (descending) it.reversed() else it } } ?: table.rows.indices.toList() }
    val width = (table.columns.size * 160).dp
    val scroll = rememberLazyListState()
    LaunchedEffect(sort) { scroll.scrollToItem(0) }
    Dialog(dismiss, DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            CompositionLocalProvider(LocalMessageSurface provides MaterialTheme.colorScheme.surface) {
                Column(Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        SigilIconButton(dismiss) { Glyph("close", 24, "Close table") }
                        Text("Table", Modifier.weight(1f), style = MaterialTheme.typography.titleLarge)
                        SigilIconButton({ table.copyTable?.let { clipboard.setText(AnnotatedString(it)) } }, enabled = table.copyTable != null) { Glyph("content_copy", 24, "Copy table") }
                    }
                    Text("${table.rows.size} ${if (table.rows.size == 1) "row" else "rows"} · ${table.columns.size} ${if (table.columns.size == 1) "column" else "columns"}", style = MaterialTheme.typography.labelMedium)
                    Column(Modifier.weight(1f).fillMaxWidth().horizontalScroll(rememberScrollState())) {
                        TableHeader(table, width, true, sort) { sort = it }
                        LazyColumn(Modifier.width(width).weight(1f), state = scroll) { items(order, key = { it }) { TableRow(table, it, width, true) { r, c -> selected = r to c } } }
                    }
                }
            }
        }
    }
    selected?.let { (row, column) ->
        val cell = table.rows[row][column]
        val copyable = cell.spans.none { it.reveal.isNotEmpty() }
        AlertDialog({ selected = null }, title = { Text("Row ${row + 1}, column ${column + 1}") }, text = {
            CompositionLocalProvider(LocalMessageSurface provides MaterialTheme.colorScheme.surface) {
                Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                    RichMessageText(cell)
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        SigilTextButton({ clipboard.setText(AnnotatedString(cell.text)) }, enabled = copyable) { Text("Copy cell") }
                        SigilTextButton({ table.copyRows.getOrNull(row)?.let { clipboard.setText(AnnotatedString(it)) } }, enabled = table.copyRows.getOrNull(row) != null) { Text("Copy row") }
                    }
                }
            }
        }, confirmButton = { SigilTextButton({ selected = null }) { Text("Done") } })
    }
}
