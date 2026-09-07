package org.sigil

import androidx.compose.runtime.Composable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue

val LocalComposerInput = staticCompositionLocalOf<@Composable (Boolean, @Composable () -> Unit) -> Unit> {
    { _, content -> content() }
}

internal class EditorProjection(val source: String, formats: List<FormatSpan>) {
    val hidden = BooleanArray(source.length)
    val offsets = IntArray(source.length + 1)
    val text: String
    private val anchors: IntArray
    init {
        formats.forEach {
            for (i in it.start until it.start + it.prefix) hidden[i] = true
            for (i in it.end - it.suffix until it.end) hidden[i] = true
        }
        text = buildString {
            for (i in source.indices) {
                if (!hidden[i]) append(source[i])
                offsets[i + 1] = length
            }
        }
        anchors = IntArray(text.length + 1)
        for (i in source.indices) if (!hidden[i]) anchors[offsets[i]] = i
        anchors[text.length] = source.length
        formats.forEach {
            anchors[offsets[it.start + it.prefix]] = it.start + it.prefix
            anchors[offsets[it.end - it.suffix]] = it.end - it.suffix
        }
    }
    fun sourceOffset(position: Int) = anchors[position.coerceIn(0, text.length)]
    fun visible(value: TextFieldValue) = TextFieldValue(text,
        TextRange(offsets[value.selection.start], offsets[value.selection.end]),
        value.composition?.let { TextRange(offsets[it.start], offsets[it.end]) })
}
