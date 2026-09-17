package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.takeOrElse
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.*
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.*

internal fun visibleCodeBlocks(value: RichText) = value.blocks.filter { block ->
    block.kind == "code" && block.start >= 0 && block.end <= value.text.length && block.start < block.end &&
        value.spans.none { it.reveal.isNotEmpty() && it.start < block.end && it.end > block.start }
}
internal fun richSlice(value: RichText, start: Int, end: Int) = RichText(value.text.substring(start, end),
    value.spans.filter { it.start < end && it.end > start }.map { it.copy(start = maxOf(it.start, start) - start, end = minOf(it.end, end) - start) },
    value.blocks.filter { it.start < end && it.end > start }.map { it.copy(start = maxOf(it.start, start) - start, end = minOf(it.end, end) - start) },
    value.codeTokens.filter { it.start >= start && it.end <= end }.map { it.copy(start = it.start - start, end = it.end - start) },
    value.motion.mapNotNull { run ->run.copy(units=run.units.filter {it.first>=start && it.second<=end}.map {it.first-start to it.second-start}).takeIf {it.units.isNotEmpty()} })

@Composable
internal fun CodeBlock(value: RichText, language: String) {
    var expanded by remember(value) { mutableStateOf(false) }
    var wrap by remember { mutableStateOf(false) }
    val clipboard = LocalClipboardManager.current
    val lines = remember(value.text) { value.text.count { it == '\n' } + if (value.text.endsWith('\n')) 0 else 1 }
    val copy = { clipboard.setText(AnnotatedString(value.text)) }
    val previewCap = with(LocalDensity.current) { MaterialTheme.typography.bodyMedium.lineHeight.toDp() * 8 }
    val count = listOfNotNull(language.takeIf { it.isNotEmpty() }?.replaceFirstChar { it.uppercase() }, "$lines ${if (lines == 1) "line" else "lines"}").joinToString(" · ")
    Column(Modifier.widthIn(min = 200.dp, max = 280.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Glyph("code", 20); Text("Code", Modifier.weight(1f), style = MaterialTheme.typography.labelMedium)
        }
        Box(Modifier.fillMaxWidth().heightIn(max = previewCap).clip(RoundedCornerShape(12.dp)).horizontalScroll(rememberScrollState())
            .clickable(role = Role.Button, onClickLabel = "Open code") { expanded = true }) {
            CodeText(value, false, 8)
        }
        Text(count, style = MaterialTheme.typography.labelSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
        SigilTextButton({ expanded = true }) { Glyph("open_in_full", 18); Spacer(Modifier.width(8.dp)); Text("Open code") }
    }
    if (expanded) Dialog({ expanded = false }, DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            Column(Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    SigilIconButton({ expanded = false }) { Glyph("close", 24, "Close code") }
                    Text("Code", Modifier.weight(1f), style = MaterialTheme.typography.titleLarge)
                    SigilIconButton(copy) { Glyph("content_copy", 24, "Copy code") }
                }
                Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).toggleable(wrap, role = Role.Checkbox) { wrap = it }.heightIn(min = 48.dp).padding(horizontal = 4.dp),
                    verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) { Checkbox(wrap, null); Text("Wrap lines", style = MaterialTheme.typography.bodyMedium) }
                SelectionContainer(Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()).then(if (wrap) Modifier else Modifier.horizontalScroll(rememberScrollState()))) {
                    CompositionLocalProvider(LocalMessageSurface provides MaterialTheme.colorScheme.surface) { CodeText(value, wrap, Int.MAX_VALUE) }
                }
            }
        }
    }
}

@Composable
private fun CodeText(value: RichText, wrap: Boolean, maxLines: Int) {
    val background = LocalMessageSurface.current.takeOrElse { MaterialTheme.colorScheme.surface }
    val foreground = LocalContentColor.current
    val text = remember(value, background, foreground) {
        buildAnnotatedString {
            append(value.text)
            value.codeTokens.forEach { token ->
                if (token.start >= 0 && token.end <= length && token.start < token.end) {
                    val color = when (token.role) { "keyword" -> "purple2"; "number" -> "blue2"; "string" -> "green2"; "comment" -> "gray2"; else -> null }
                    if (color != null) addStyle(SpanStyle(color = textColor(color, background)), token.start, token.end)
                }
            }
        }
    }
    Text(text, style = MaterialTheme.typography.bodyMedium.copy(fontFamily = LocalCodeFont.current), color = foreground, softWrap = wrap, maxLines = maxLines, overflow = TextOverflow.Clip)
}
