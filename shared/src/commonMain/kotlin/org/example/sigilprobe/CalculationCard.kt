package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.unit.dp

// Grouped thousands and a true minus; copy keeps the plain wire digits.
internal fun readableNumber(raw: String): String {
    val match = Regex("""([+-]?)(\d+)(\.\d+)?""").matchEntire(raw.trim()) ?: return raw
    val (sign, whole, fraction) = match.destructured
    val grouped = whole.reversed().chunked(3).joinToString(",").reversed()
    val zero = whole.all { it == '0' } && fraction.all { it == '.' || it == '0' }
    return (if (sign == "-" && !zero) "−" else "") + grouped + fraction
}

// Two decimals at a glance; a tiny value keeps two significant digits instead of reading as 0.
internal fun glanceNumber(raw: String): String {
    val value = raw.trim().toDoubleOrNull() ?: return readableNumber(raw)
    val size = kotlin.math.abs(value)
    if (size >= 1e12 || size == 0.0) return readableNumber(raw)
    val places = if (size >= .01) 2 else minOf(6, kotlin.math.ceil(-kotlin.math.log10(size)).toInt() + 1)
    var scale = 1L; repeat(places) { scale *= 10 }
    val scaled = kotlin.math.round(size * scale).toLong()
    val fraction = (scaled % scale).toString().padStart(places, '0').trimEnd('0')
    return readableNumber((if (value < 0 && scaled > 0) "-" else "") + (scaled / scale) + (if (fraction.isEmpty()) "" else ".$fraction"))
}

// Long figures step down a size rather than clip; digits wrap only past the smallest step.
@Composable
internal fun heroStyle(figure: String): TextStyle = with(MaterialTheme.typography) {
    when { figure.length <= 12 -> displaySmall; figure.length <= 18 -> headlineMedium; else -> headlineSmall }
}.copy(fontFeatureSettings = "tnum, lnum")

// Source on top in quiet code, the answer under it as the figure; 8dp because both lines trim their leading.
@Composable
internal fun FigureCard(spoken: String, figure: String, source: @Composable () -> Unit) {
    val ink = LocalContentColor.current
    Column(Modifier.widthIn(min = MessageCardMinWidth, max = MessageCardMaxWidth).padding(vertical = 4.dp)
        .clearAndSetSemantics { contentDescription = spoken }, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        CompositionLocalProvider(LocalContentColor provides ink.copy(alpha = .68f)) { source() }
        Text(figure, style = heroStyle(figure), color = ink)
    }
}

// Typeset like mathematics: × ÷ − with operator spacing and raised exponents; plain source only if it fails to parse.
@Composable
internal fun CalculationCard(value: UtilityContent) {
    val ink = LocalContentColor.current
    val figure = readableNumber(value.display)
    val source = value.rich?.text.orEmpty()
    val tree = remember(source) { parseArith(source).takeIf { value.rich?.spans.isNullOrEmpty() } }
    val tokens = remember(tree) { tree?.let(::arithTokens) }
    // An unbreakable run wider than the card falls back to the wrapped source rather than clip.
    var overflow by remember(source) { mutableStateOf(false) }
    val spoken = "Calculation. ${tree?.let { arithSpoken(it) } ?: source} equals ${spokenFigure(figure)}"
    val hero = heroStyle("= $figure")
    Column(Modifier.widthIn(min = MessageCardMinWidth, max = MessageCardMaxWidth).padding(vertical = 4.dp)
        .clearAndSetSemantics { contentDescription = spoken }, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        CompositionLocalProvider(LocalContentColor provides ink.copy(alpha = .68f)) {
            if (tokens != null && !overflow) ArithmeticLine(tokens, MaterialTheme.typography.bodyLarge.copy(fontFeatureSettings = "lnum"), onOverflow = { overflow = true })
            else value.rich?.let { RichMessageText(it, style = MaterialTheme.typography.labelMedium.copy(fontFamily = LocalCodeFont.current)) }
        }
        // "=" is its own run so the sans face loads on web; the figure hangs beside it and breaks only after a comma.
        Row {
            Text("=", Modifier.alignByBaseline().padding(end = 8.dp), style = hero.copy(fontFamily = operatorFamily()), color = ink.copy(alpha = .68f))
            Text(figure.replace(",", ",\u200B"), Modifier.alignByBaseline(), style = hero, color = ink)
        }
    }
}
