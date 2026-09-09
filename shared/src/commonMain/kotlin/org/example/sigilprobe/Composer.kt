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

internal data class FormatSpan(val start: Int, val end: Int, val prefix: Int, val suffix: Int, val style: Int)

internal fun spans(wire: String) = wire.lineSequence().filter { it.isNotEmpty() }.map {
    val v = it.split(',').map(String::toInt)
    FormatSpan(v[0], v[1], v[2], v[3], v[4])
}.toList()

internal fun presentation(analyze: (String) -> String, codeFont: FontFamily = FontFamily.Monospace) = OutputTransformation {
    val formats = spans(analyze(toString()))
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
    formats.forEach {
        val style = when (it.style) {
            1 -> SpanStyle(fontWeight = FontWeight.Bold)
            2 -> SpanStyle(fontStyle = FontStyle.Italic)
            3 -> SpanStyle(textDecoration = TextDecoration.LineThrough)
            else -> SpanStyle(fontFamily = codeFont)
        }
        addStyle(style, offsets[it.start], offsets[it.end])
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
        if (old.length > 0) {
            val original = originalText.toString()
            val prefix = BooleanArray(original.length)
            val suffix = BooleanArray(original.length)
            spans(analyze(original)).forEach {
                val bodyStart = it.start + it.prefix
                val bodyEnd = it.end - it.suffix
                if (old.min > bodyStart || old.max < bodyEnd) {
                    for (i in it.start until bodyStart) prefix[i] = true
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
fun Composer(state: TextFieldState, analyze: (String) -> String, modifier: Modifier = Modifier, showTools: Boolean = true, focusRequester: FocusRequester? = null, onFocus: () -> Unit = {}) {
    var sourceMode by remember { mutableStateOf(false) }
    var editorFocused by remember { mutableStateOf(false) }
    var formattingSelection by remember { mutableStateOf(state.selection) }
    val editorFocus = focusRequester ?: remember { FocusRequester() }
    val focusManager = LocalFocusManager.current
    SideEffect { if (editorFocused) formattingSelection = state.selection }
    val source = state.text.toString()
    val formats = remember(source, analyze) { spans(analyze(source)) }
    val codeFont = LocalCodeFont.current
    val output = remember(analyze, codeFont) { presentation(analyze, codeFont) }
    val active = formats.filter { state.selection.start in it.start until it.end }
        .map { listOf("", "bold", "italic", "strike", "code")[it.style] }.distinct()
    Column(modifier, verticalArrangement = Arrangement.spacedBy(4.dp)) {
        if (showTools) {
        Row {
            listOf("B" to "**", "I" to "*", "Strike" to "~~", "Code" to "`").forEach { (label, marker) ->
                TextButton({ state.format(marker, formattingSelection); editorFocus.requestFocus() }) {
                    Text(label, Modifier.clearAndSetSemantics {
                        contentDescription = when (label) { "B" -> "Bold"; "I" -> "Italic"; else -> label }
                    })
                }
            }
        }
        Row {
            TextButton({ state.undoState.undo() }, enabled = state.undoState.canUndo) { Text("Undo") }
            TextButton({ state.undoState.redo() }, enabled = state.undoState.canRedo) { Text("Redo") }
            TextButton({ sourceMode = !sourceMode }) { Text(if (sourceMode) "Formatted" else "Source") }
        }
        Text(if (active.isEmpty()) "Composer" else "Formatting: ${active.joinToString()}")
        }
        LocalComposerInput.current(sourceMode) {
        BasicTextField(state,
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
                else preserveBoundaries(analyze).then(InputTransformation.maxLength(16_384)),
            outputTransformation = if (sourceMode) null else output,
            decorator = { inner -> Box(contentAlignment = androidx.compose.ui.Alignment.CenterStart) { if (source.isEmpty()) Text("Message", color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.bodyLarge); inner() } },
            textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
            lineLimits = TextFieldLineLimits.MultiLine(maxHeightInLines = 4))
        }
    }
}
