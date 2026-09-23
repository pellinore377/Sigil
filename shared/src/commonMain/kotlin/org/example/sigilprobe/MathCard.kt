package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.unit.dp

// Text and inline formulas only: the formulas join the sentence instead of stacking under it.
internal fun ChatMessage.inlineMathLine() = parts.any { it.utility?.kind == "math" && !it.utility.block } &&
    parts.all { it.kind == "text" || (it.utility?.kind == "math" && !it.utility.block) }

@Composable
private fun Formula(value: UtilityContent, modifier: Modifier = Modifier) {
    val render = LocalMathContent.current
    val style = if (value.block) MaterialTheme.typography.headlineSmall else MaterialTheme.typography.bodyLarge
    Box(modifier.clearAndSetSemantics { contentDescription = "Formula. ${value.display}" }, contentAlignment = Alignment.Center) {
        CompositionLocalProvider(LocalTextStyle provides style) {
            // The cap bounds Android's WebView, which has no intrinsic height; the web renderer sizes itself within it.
            if (render != null && value.mathml != null) render(value.mathml, value.display, Modifier.heightIn(max = if (value.block) 160.dp else 52.dp))
            // No renderer: the TeX source, whole and wrapping, in quiet code.
            else Text(value.display, style = MaterialTheme.typography.bodyMedium.copy(fontFamily = LocalCodeFont.current))
        }
    }
}

@Composable
internal fun MathCard(value: UtilityContent) {
    if (!value.block) { Formula(value, Modifier.widthIn(max = MessageCardMaxWidth)); return }
    Formula(value, Modifier.widthIn(min = MessageCardMinWidth, max = MessageCardMaxWidth).padding(vertical = 8.dp))
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun InlineMathMessage(message: ChatMessage, analyze: (String) -> String) {
    FlowRow(Modifier.widthIn(max = MessageCardMaxWidth), horizontalArrangement = Arrangement.spacedBy(4.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        message.parts.forEach { part ->
            val formula = part.utility
            val middle = Modifier.align(Alignment.CenterVertically)
            if (formula != null) Formula(formula, middle)
            else Box(middle) { if (part.rich != null) RichMessageText(part.rich) else MessageText(part.text, analyze) }
        }
    }
}
