@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)

package org.sigil

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.input.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.FocusDirection
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.input.key.*
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.font.*
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.lerp

val LocalEditorAnalysis = staticCompositionLocalOf<((String) -> String)?> { null }

internal data class FormatSpan(val start: Int, val end: Int, val prefix: Int, val suffix: Int, val style: Int, val argument: String = "")

internal fun spans(wire: String) = wire.lineSequence().filter { it.isNotEmpty() }.map {
    val v = it.split(',', limit = 6)
    FormatSpan(v[0].toInt(), v[1].toInt(), v[2].toInt(), v[3].toInt(), v[4].toInt(), v.getOrElse(5) { "" })
}.toList()

// Keep unsupported presentation visible as source, including its modifier chain.
internal fun editorFormats(wire: String): List<FormatSpan> {
    val all = spans(wire)
    val sourceRanges = all.filter { it.style in listOf(7, 8, 12) }.map { it.start to it.end }.toSet()
    return all.filter { it.start to it.end !in sourceRanges }
}

internal fun presentation(analyze: (String) -> String, codeFont: FontFamily = FontFamily.Monospace, surface: Color = Color.White, foreground: Color = Color.Black) = OutputTransformation {
    val formats = editorFormats(analyze(toString()))
    val projection = EditorProjection(toString(), formats)
    val hidden = projection.hidden
    val offsets = projection.offsets
    var end = hidden.size
    while (end > 0) {
        if (!hidden[end - 1]) { end--; continue }
        var start = end - 1
        while (start > 0 && hidden[start - 1]) start--
        delete(start, end)
        end = start
    }
    formats.filter { it.style != 3 && it.style != 5 }.forEach {
        val style = when (it.style) {
            1 -> SpanStyle(fontWeight = FontWeight.Bold)
            2 -> SpanStyle(fontStyle = FontStyle.Italic)
            4 -> SpanStyle(fontFamily = codeFont)
            6 -> SpanStyle(background = foreground.copy(alpha = .16f))
            9 -> SpanStyle(fontSize = (1f + (it.argument.toIntOrNull() ?: 0).coerceIn(-3, 3) * .12f).em)
            10, 11 -> {
                val colors = it.argument.substringBefore('|').split(':').map { name -> textColor(name, surface) }
                if (it.style == 11) SpanStyle(background = colors.first().copy(alpha = .16f))
                else SpanStyle(color = colors.first())
            }
            else -> SpanStyle()
        }
        addStyle(style, offsets[it.start], offsets[it.end])
        if (it.style == 10 && '|' in it.argument) {
            val colors = it.argument.substringBefore('|').split(':').map { name -> textColor(name, surface) }
            val bounds = it.argument.substringAfter('|').split(':').map { at -> offsets[at.toInt()] }.distinct()
            bounds.zipWithNext().forEachIndexed { index, (start, end) ->
                val position = index.toFloat() / (bounds.size - 2).coerceAtLeast(1) * (colors.size - 1)
                val stop = position.toInt().coerceAtMost(colors.lastIndex)
                addStyle(SpanStyle(color = lerp(colors[stop], colors[minOf(stop + 1, colors.lastIndex)], position - stop)), start, end)
            }
        }
    }
    val decorations = formats.filter { it.style == 3 || it.style == 5 }
    decorations.flatMap { listOf(it.start, it.end) }.distinct().sorted().zipWithNext().forEach { (start, end) ->
        val active = decorations.filter { it.start <= start && it.end >= end }.map { if (it.style == 3) TextDecoration.LineThrough else TextDecoration.Underline }.distinct()
        if (active.isNotEmpty()) addStyle(SpanStyle(textDecoration = TextDecoration.combine(active)), offsets[start], offsets[end])
    }
}

// OutputTransformation can include an invisible delimiter in a deletion range.
// Restore delimiters when editing only part of their formatted content.
internal fun preserveBoundaries(analyze: (String) -> String) = InputTransformation {
    if (changes.changeCount > 0) {
        // Some platform bridges replace the whole buffer; recover the actual edit.
        val originalValue = originalText.toString()
        val currentValue = toString()
        var start = 0
        while (start < minOf(originalValue.length, currentValue.length) && originalValue[start] == currentValue[start]) start++
        var oldEnd = originalValue.length
        var newEnd = currentValue.length
        while (oldEnd > start && newEnd > start && originalValue[oldEnd - 1] == currentValue[newEnd - 1]) { oldEnd--; newEnd-- }
        val old = TextRange(start, oldEnd)
        val changed = TextRange(start, newEnd)
        val escape = if (old.length == 0) editorFormats(analyze(originalValue)).firstOrNull { it.style == 0 && old.min == it.start + it.prefix } else null
        if (escape != null && changed.length > 0) {
            val inserted = currentValue.substring(changed.min, changed.max)
            replace(escape.start, changed.max, inserted + originalValue.substring(escape.start, old.min))
            selection = TextRange(escape.start + inserted.length)
        } else if (old.length > 0) {
            val original = originalText.toString()
            val prefix = BooleanArray(original.length)
            val suffix = BooleanArray(original.length)
            editorFormats(analyze(original)).forEach {
                val bodyStart = it.start + it.prefix
                val bodyEnd = it.end - it.suffix
                if (old.min > bodyStart || old.max < bodyEnd) {
                    val restore = if (it.style == 0 && old.max <= bodyStart) suffix else prefix
                    for (i in it.start until bodyStart) restore[i] = true
                    for (i in bodyEnd until it.end) suffix[i] = true
                }
            }
            val before = buildString { for (i in old.min until old.max) if (prefix[i]) append(original[i]) }
            val after = buildString { for (i in old.min until old.max) if (suffix[i]) append(original[i]) }
            if (before.isNotEmpty() || after.isNotEmpty()) {
                val inserted = toString().substring(changed.min, changed.max)
                replace(changed.min, changed.max, before + inserted + after)
                selection = TextRange(changed.min + before.length + inserted.length)
            }
        }
    }
}

internal fun TextFieldState.format(marker: String, range: TextRange = selection) {
    edit {
        val start = range.min
        val end = range.max
        insert(end, marker)
        insert(start, marker)
        selection = TextRange(start + marker.length, end + marker.length)
    }
}

@Composable
fun Composer(state: TextFieldState, analyze: (String) -> String, modifier: Modifier = Modifier, showTools: Boolean = true, focusRequester: FocusRequester? = null, onFocus: () -> Unit = {}, enabled: Boolean = true, namedFormatting: Boolean = true, showSource: Boolean? = null) {
    var internalSourceMode by remember { mutableStateOf(false) }
    val sourceMode = showSource ?: internalSourceMode
    var editorFocused by remember { mutableStateOf(false) }
    var formattingSelection by remember { mutableStateOf(state.selection) }
    val editorFocus = focusRequester ?: remember { FocusRequester() }
    val focusManager = LocalFocusManager.current
    SideEffect { if (editorFocused) formattingSelection = state.selection }
    val source = state.text.toString()
    val editorAnalysis = if (namedFormatting) LocalEditorAnalysis.current ?: analyze else analyze
    val formats = remember(source, editorAnalysis) { spans(editorAnalysis(source)) }
    val codeFont = LocalCodeFont.current
    val surface = MaterialTheme.colorScheme.background
    val foreground = MaterialTheme.colorScheme.onSurface
    val output = remember(editorAnalysis, codeFont, surface, foreground) { presentation(editorAnalysis, codeFont, surface, foreground) }
    val active = formats.filter { state.selection.start in it.start until it.end }
        .mapNotNull { listOf("", "bold", "italic", "strike", "code", "underline", "highlight", "spoiler", "scratch", "size", "color", "highlight", "animation").getOrNull(it.style)?.takeIf(String::isNotEmpty) }.distinct()
    Column(modifier, verticalArrangement = Arrangement.spacedBy(4.dp)) {
        if (showTools) {
        Row {
            listOf("B" to "**", "I" to "*", "Strike" to "~~", "Code" to "`").forEach { (label, marker) ->
                SigilTextButton({ state.format(marker, formattingSelection); editorFocus.requestFocus() }) {
                    Text(label, Modifier.clearAndSetSemantics {
                        contentDescription = when (label) { "B" -> "Bold"; "I" -> "Italic"; else -> label }
                    })
                }
            }
        }
        Row {
            SigilTextButton({ state.undoState.undo() }, enabled = state.undoState.canUndo) { Text("Undo") }
            SigilTextButton({ state.undoState.redo() }, enabled = state.undoState.canRedo) { Text("Redo") }
            SigilTextButton({ internalSourceMode = !sourceMode }) { Text(if (sourceMode) "Formatted" else "Source") }
        }
        Text(if (active.isEmpty()) "Composer" else "Formatting: ${active.joinToString()}")
        }
        LocalComposerInput.current(sourceMode) {
        BasicTextField(state, enabled = enabled, cursorBrush = androidx.compose.ui.graphics.SolidColor(MaterialTheme.colorScheme.primary),
            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp, max = 144.dp).testTag("composer")
                .semantics { contentDescription = "Message" }
                .focusRequester(editorFocus).onFocusChanged { editorFocused = it.isFocused; if (it.isFocused) onFocus() }
                .background(MaterialTheme.colorScheme.background, androidx.compose.foundation.shape.RoundedCornerShape(16.dp)).padding(horizontal = 14.dp, vertical = 12.dp)
                .onPreviewKeyEvent {
                    if (it.type != KeyEventType.KeyDown) false
                    else if (it.key == Key.Tab && !it.isCtrlPressed && !it.isAltPressed && !it.isMetaPressed) {
                        focusManager.moveFocus(if (it.isShiftPressed) FocusDirection.Previous else FocusDirection.Next)
                    }
                    else if (!it.isCtrlPressed) false
                    else when (it.key) {
                        Key.B -> { state.format("**"); true }
                        Key.I -> { state.format("*"); true }
                        Key.X -> if (it.isShiftPressed) { state.format("~~"); true } else false
                        Key.M -> if (it.isShiftPressed) { state.format("`"); true } else false
                        else -> false
                    }
                },
            inputTransformation = if (sourceMode) InputTransformation.maxLength(16_384)
                else preserveBoundaries(editorAnalysis).then(InputTransformation.maxLength(16_384)),
            outputTransformation = if (sourceMode) null else output,
            decorator = { inner -> Box(contentAlignment = androidx.compose.ui.Alignment.CenterStart) { if (source.isEmpty()) Text("Message", color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.bodyLarge); inner() } },
            textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
            lineLimits = TextFieldLineLimits.MultiLine(maxHeightInLines = 4))
        }
    }
}
