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
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.*

@Composable
internal fun TableCard(table: TableContent) {
    val width = (table.columns.size * 160).dp
    Column(Modifier.widthIn(min = MessageCardMinWidth, max = MessageCardMaxWidth).animateContentSize(LocalMotion.current.tween(MotionMillis)), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) { Glyph("table", 20); Text("Table", style = MaterialTheme.typography.labelMedium) }
        Column(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState())) {
            TableHeader(table, width, false, null) {}
            table.rows.indices.take(3).forEach { TableRow(table, it, width, false, null) }
        }
        Text("${table.rows.size} ${if (table.rows.size == 1) "row" else "rows"} · ${table.columns.size} ${if (table.columns.size == 1) "column" else "columns"}", style = MaterialTheme.typography.labelSmall)
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
