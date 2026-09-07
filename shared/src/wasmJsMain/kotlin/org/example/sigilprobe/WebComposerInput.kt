@file:OptIn(androidx.compose.ui.ExperimentalComposeUiApi::class)

package org.sigil

import androidx.compose.runtime.Composable
import androidx.compose.ui.platform.InterceptPlatformTextInput
import androidx.compose.ui.platform.PlatformTextInputMethodRequest
import androidx.compose.ui.text.input.*

// Present the same text to the browser input and Compose. Convert native edits
// back to source coordinates without exposing hidden Markdown to the browser.
@Composable
fun WebComposerInput(sourceMode: Boolean, content: @Composable () -> Unit) {
    if (sourceMode) { content(); return }
    InterceptPlatformTextInput(interceptor = { request, next ->
        fun projection() = request.value().let { EditorProjection(it.text, spans(rustAnalyze(it.text))) }
        next.startInputMethod(object : PlatformTextInputMethodRequest by request {
            override val value = { projection().visible(request.value()) }
            override val onEditCommand: (List<EditCommand>) -> Unit = { commands ->
                val old = request.value()
                val projected = projection()
                val processor = EditProcessor()
                val visible = projected.visible(old)
                processor.reset(visible, null)
                // reset treats a new buffer as an external edit and clears composition.
                visible.composition?.let { processor.apply(listOf(SetComposingRegionCommand(it.start, it.end))) }
                val edited = processor.apply(commands)
                var start = 0
                while (start < minOf(projected.text.length, edited.text.length) && projected.text[start] == edited.text[start]) start++
                var oldEnd = projected.text.length
                var newEnd = edited.text.length
                while (oldEnd > start && newEnd > start && projected.text[oldEnd - 1] == edited.text[newEnd - 1]) { oldEnd--; newEnd-- }
                var rawStart = projected.sourceOffset(start)
                var rawEnd = projected.sourceOffset(oldEnd)
                val inserted = edited.text.substring(start, newEnd)
                if (start == oldEnd && old.selection.collapsed && projected.offsets[old.selection.start] == start) {
                    // Keep the source caret's side of a hidden delimiter when typing.
                    rawStart = old.selection.start; rawEnd = rawStart
                }
                if (start == 0 && oldEnd == projected.text.length && oldEnd > 0) {
                    rawStart = 0; rawEnd = old.text.length
                }
                if (inserted.isEmpty() && oldEnd > start) {
                    spans(rustAnalyze(old.text)).forEach {
                        if (start <= projected.offsets[it.start] && oldEnd >= projected.offsets[it.end]) {
                            rawStart = minOf(rawStart, it.start); rawEnd = maxOf(rawEnd, it.end)
                        }
                    }
                }
                fun rawPosition(position: Int): Int = when {
                    edited.selection.min == 0 && edited.selection.max == edited.text.length && !edited.selection.collapsed && position == 0 -> 0
                    edited.selection.min == 0 && edited.selection.max == edited.text.length && !edited.selection.collapsed && position == edited.text.length -> old.text.length + inserted.length - (rawEnd - rawStart)
                    position < start -> projected.sourceOffset(position)
                    position <= newEnd -> rawStart + position - start
                    else -> projected.sourceOffset(position - (newEnd - oldEnd)) + inserted.length - (rawEnd - rawStart)
                }
                val edits = mutableListOf<EditCommand>(FinishComposingTextCommand())
                if (oldEnd != start || inserted.isNotEmpty()) {
                    edits += SetSelectionCommand(rawStart, rawEnd)
                    edits += CommitTextCommand(inserted, 1)
                }
                edited.composition?.let { edits += SetComposingRegionCommand(rawPosition(it.start), rawPosition(it.end)) }
                edits += SetSelectionCommand(rawPosition(edited.selection.start), rawPosition(edited.selection.end))
                request.onEditCommand(edits)
            }
        })
    }, content = content)
}
