package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.*

@Composable
internal fun TableCard(table: TableContent) {
    var expanded by remember(table) { mutableStateOf(false) }
    var selected by remember(table) { mutableStateOf<Pair<Int, Int>?>(null) }
    var sort by remember(table) { mutableStateOf<Pair<Int, Boolean>?>(null) }
    val clipboard = LocalClipboardManager.current
    val order = remember(table, sort) { sort?.let { (column, descending) -> table.numericOrder.getOrNull(column)?.let { if (descending) it.reversed() else it } } ?: table.rows.indices.toList() }
    val width = (table.columns.size * 160).dp
    val scroll = rememberLazyListState()
    LaunchedEffect(sort) { scroll.scrollToItem(0) }
    @Composable fun header(full: Boolean) {
        Row(Modifier.width(width).background(LocalContentColor.current.copy(alpha = .07f))) {
            table.columns.forEachIndexed { column, label ->
                val sortable = full && table.numericOrder.getOrNull(column) != null
                Column(Modifier.width(160.dp).heightIn(min = 56.dp).then(if (sortable) Modifier.clickable(role = Role.Button) {
                    sort = when { sort?.first != column -> column to false; sort?.second == false -> column to true; else -> null }
                } else Modifier).semantics { if (sortable) { contentDescription = "Sort column ${column + 1}"; stateDescription = if (sort?.first != column) "Original order" else if (sort?.second == true) "Descending" else "Ascending" } }.padding(12.dp)) {
                    RichMessageText(label, Modifier.heightIn(max = if (full) 160.dp else 72.dp).clipToBounds(), MaterialTheme.typography.labelLarge)
                    if (sortable) Glyph(if (sort?.first == column && sort?.second == true) "arrow_downward" else "arrow_upward", 16)
                }
            }
        }
    }
    @Composable fun row(index: Int, full: Boolean) {
        Row(Modifier.width(width).background(LocalContentColor.current.copy(alpha = if (index % 2 == 0) .025f else .055f))) {
            table.rows[index].forEachIndexed { column, cell ->
                Box(Modifier.width(160.dp).heightIn(min = 56.dp).combinedClickable(role = Role.Button, onClick = { selected = index to column }, onLongClickLabel = "Open cell", onLongClick = { selected = index to column })
                    .semantics { contentDescription = "Row ${index + 1}, column ${column + 1}" }.padding(12.dp)) {
                    RichMessageText(cell, Modifier.fillMaxWidth().heightIn(max = if (full) 160.dp else 96.dp).clipToBounds(),
                        MaterialTheme.typography.bodyMedium.copy(textAlign = if (table.numericOrder.getOrNull(column) != null) TextAlign.End else TextAlign.Start))
                }
            }
        }
    }
    Column(Modifier.widthIn(min = 200.dp, max = 280.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) { Glyph("table", 20); Text("Table", style = MaterialTheme.typography.labelMedium) }
        Column(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState())) {
            header(false)
            table.rows.indices.take(3).forEach { row(it, false) }
        }
        SigilTextButton({ expanded = true }) { Glyph("open_in_full", 18); Spacer(Modifier.width(8.dp)); Text("Open table · ${table.rows.size} ${if (table.rows.size == 1) "row" else "rows"}") }
    }
    if (expanded) Dialog({ expanded = false }, DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            CompositionLocalProvider(LocalMessageSurface provides MaterialTheme.colorScheme.surface) {
                Column(Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        SigilIconButton({ expanded = false }) { Glyph("close", 24, "Close table") }
                        Text("Table", Modifier.weight(1f), style = MaterialTheme.typography.titleLarge)
                        SigilIconButton({ table.copyTable?.let { clipboard.setText(AnnotatedString(it)) } }, enabled = table.copyTable != null) { Glyph("content_copy", 24, "Copy table") }
                    }
                    Text("${table.rows.size} ${if (table.rows.size == 1) "row" else "rows"} · ${table.columns.size} ${if (table.columns.size == 1) "column" else "columns"}", style = MaterialTheme.typography.labelMedium)
                    Column(Modifier.weight(1f).fillMaxWidth().horizontalScroll(rememberScrollState())) {
                        header(true)
                        LazyColumn(Modifier.width(width).weight(1f), state = scroll) { items(order, key = { it }) { row(it, true) } }
                    }
                }
            }
        }
    }
    selected?.let { (row, column) ->
        val cell = table.rows[row][column]
        val copyable = cell.spans.none { it.reveal.isNotEmpty() }
        Dialog(onDismissRequest = { selected = null }, properties = DialogProperties()) {
            Surface(shape = MaterialTheme.shapes.large) {
                CompositionLocalProvider(LocalMessageSurface provides MaterialTheme.colorScheme.surface) {
                    Column(Modifier.heightIn(max = 560.dp).verticalScroll(rememberScrollState()).padding(20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        Text("Row ${row + 1}, column ${column + 1}", style = MaterialTheme.typography.labelLarge)
                        RichMessageText(cell)
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            SigilTextButton({ clipboard.setText(AnnotatedString(cell.text)) }, enabled = copyable) { Text("Copy cell") }
                            SigilTextButton({ table.copyRows.getOrNull(row)?.let { clipboard.setText(AnnotatedString(it)) } }, enabled = table.copyRows.getOrNull(row) != null) { Text("Copy row") }
                        }
                        SigilTextButton({ selected = null }) { Text("Done") }
                    }
                }
            }
        }
    }
}
