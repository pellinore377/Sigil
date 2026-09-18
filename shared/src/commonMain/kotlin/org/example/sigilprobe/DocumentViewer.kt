@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
package org.sigil

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

internal val ViewerHeaderShape = RoundedCornerShape(24.dp)
private val ViewerCaptionShape = RoundedCornerShape(28.dp)

// The timeline's floating glass header, over whatever the page shows beneath it.
@Composable internal fun ViewerHeader(backdrop: ChromeBackdrop, modifier: Modifier = Modifier, extent: (Dp) -> Unit = {}, content: @Composable RowScope.() -> Unit) {
    val density = androidx.compose.ui.platform.LocalDensity.current
    // The page learns how far the pill reaches, so nothing lands behind the glass.
    FloatingChrome(backdrop, modifier.padding(top = WindowInsets.statusBars.asPaddingValues().calculateTopPadding() + 12.dp).widthIn(max = 920.dp).fillMaxWidth().padding(horizontal = 12.dp).height(pageHeaderHeight())
        .onGloballyPositioned { extent(with(density) { it.boundsInWindow().bottom.toDp() }) }, ViewerHeaderShape) {
        Row(Modifier.fillMaxSize().padding(horizontal = 8.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically, content = content)
    }
}

// The caption, in the pill the image viewer uses for its bar.
@Composable internal fun ViewerCaption(caption: String, modifier: Modifier = Modifier) {
    Surface(modifier.padding(bottom = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding() + 12.dp).widthIn(max = 680.dp).padding(horizontal = 16.dp), shape = ViewerCaptionShape,
        color = MaterialTheme.colorScheme.surfaceContainerHigh.copy(alpha = .94f), contentColor = MaterialTheme.colorScheme.onSurface) {
        Text(caption, Modifier.padding(horizontal = 18.dp, vertical = 12.dp), style = MaterialTheme.typography.bodyLarge, maxLines = 4, overflow = TextOverflow.Ellipsis)
    }
}

// Zoom as two squircles stacked in the page's bottom corner, the same on every platform.
@Composable fun ZoomControls(zoomIn: () -> Unit, zoomOut: () -> Unit, canZoomIn: Boolean, canZoomOut: Boolean, modifier: Modifier = Modifier) {
    val scheme = MaterialTheme.colorScheme
    Column(modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        listOf(Triple("zoom_in", zoomIn, canZoomIn), Triple("zoom_out", zoomOut, canZoomOut)).forEach { (glyph, action, enabled) ->
            Box(Modifier.size(48.dp).background(scheme.surfaceContainerHigh.copy(alpha = .94f), SquircleShape).clip(SquircleShape).then(if (enabled) Modifier.combinedClickable(onClick = action) else Modifier), contentAlignment = Alignment.Center) {
                CompositionLocalProvider(LocalContentColor provides scheme.onSurface.copy(alpha = if (enabled) 1f else .38f)) { Glyph(glyph, 24, if (glyph == "zoom_in") "Zoom in" else "Zoom out") }
            }
        }
    }
}

// The image viewer's shape for a file: the timeline header naming and offering it, floating over the page itself.
@Composable fun DocumentViewerChrome(name: String, kind: String, bytes: Long, close: () -> Unit, download: (() -> Unit)?, downloading: Boolean = false, caption: String? = null,
    actions: @Composable RowScope.() -> Unit = {}, content: @Composable BoxScope.() -> Unit) {
    val scheme = MaterialTheme.colorScheme
    val backdrop = rememberChromeBackdrop()
    var reach by remember { mutableStateOf(112.dp) }
    val top = reach + 12.dp
    val bottom = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding() + if (caption.isNullOrBlank()) 12.dp else 84.dp
    Box(Modifier.fillMaxSize()) {
        CompositionLocalProvider(LocalContentColor provides scheme.onBackground) {
            Box(Modifier.fillMaxSize().captureBackdrop(backdrop).background(scheme.background)) { Box(Modifier.fillMaxSize().padding(top = top, bottom = bottom), content = content) }
            ViewerHeader(backdrop, Modifier.align(Alignment.TopCenter), { reach = it }) {
                Symbol("chevron_left", "Back", close)
                Column(Modifier.weight(1f).padding(start = 10.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(name, style = MaterialTheme.typography.titleLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text("$kind · ${attachmentSize(bytes)}", style = MaterialTheme.typography.labelMedium, color = scheme.onSurfaceVariant, maxLines = 1)
                }
                actions()
                download?.let { Symbol("download", if (downloading) "Saving file" else "Save file", it) }
            }
            if (!caption.isNullOrBlank()) ViewerCaption(caption, Modifier.align(Alignment.BottomCenter))
        }
    }
}

// Plain text or Markdown, selectable, on the reader's ground.
@Composable fun TextDocumentView(text: String, markdown: Boolean, modifier: Modifier = Modifier) {
    SelectionContainer(modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
        Text(if (markdown) markdownPreview(text, 1.3f) else AnnotatedString(text), Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 16.dp),
            style = if (markdown) MaterialTheme.typography.bodyLarge else MaterialTheme.typography.bodyMedium.copy(fontFamily = FontFamily.Monospace))
    }
}

// The sheet as a window onto the grid: numbered rows and columns, zebra rows, cells that scroll both ways.
// A tap marks a cell with a border in the theme's colour; holding it offers a copy.
@Composable fun TableDocumentView(cells: List<List<String>>, modifier: Modifier = Modifier, firstRow: Int = 0, footer: (@Composable () -> Unit)? = null) {
    val columns = cells.maxOfOrNull { it.size } ?: 0
    val scheme = MaterialTheme.colorScheme
    val clipboard = LocalClipboardManager.current
    var selected by remember(cells) { mutableStateOf<Pair<Int, Int>?>(null) }
    var menu by remember(cells) { mutableStateOf(false) }
    Box(modifier.fillMaxSize().horizontalScroll(rememberScrollState())) {
        LazyColumn(Modifier.width((48 + columns * 160).dp)) {
            stickyHeader {
                Row(Modifier.background(scheme.surfaceContainer)) {
                    Text("#", Modifier.width(48.dp).padding(8.dp), style = MaterialTheme.typography.labelLarge)
                    repeat(columns) { Text("${it + 1}", Modifier.width(160.dp).padding(8.dp), style = MaterialTheme.typography.labelLarge) }
                }
            }
            itemsIndexed(cells) { r, row ->
                Row(Modifier.background(scheme.onSurface.copy(alpha = if (r % 2 == 0) .025f else .055f))) {
                    Text("${firstRow + r + 1}", Modifier.width(48.dp).padding(8.dp), style = MaterialTheme.typography.labelMedium)
                    for (c in 0 until columns) {
                        val value = row.getOrNull(c).orEmpty()
                        val picked = selected == r to c
                        Box(Modifier.width(160.dp).heightIn(min = 56.dp).then(if (picked) Modifier.border(2.dp, scheme.primary) else Modifier)
                            .combinedClickable(onClick = { selected = r to c; menu = false }, onLongClick = { selected = r to c; menu = true })) {
                            Text(value, Modifier.padding(12.dp), maxLines = 4, style = MaterialTheme.typography.bodyMedium)
                            if (picked) DropdownMenu(menu, { menu = false }) {
                                DropdownMenuItem({ Text("Copy") }, { clipboard.setText(AnnotatedString(value)); menu = false }, leadingIcon = { Glyph("content_copy", 20) })
                            }
                        }
                    }
                }
            }
            footer?.let { item { Box(Modifier.padding(12.dp)) { it() } } }
        }
    }
}
