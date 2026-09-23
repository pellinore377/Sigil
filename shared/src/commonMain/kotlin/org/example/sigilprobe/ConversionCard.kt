package org.sigil

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.text.style.TextOverflow

// "8.0467 km" → "8.05 km"; the unit rides at the figure's size, as one quantity.
internal fun readableQuantity(raw: String, glance: Boolean): String {
    val number = raw.substringBefore(' ')
    val unit = raw.substringAfter(' ', "")
    val shown = if (glance) glanceNumber(number) else readableNumber(number)
    return if (unit.isEmpty()) shown else "$shown\u00A0$unit"
}

@Composable
internal fun ConversionCard(value: UtilityContent) {
    val source = readableQuantity(value.display, false)
    val figure = readableQuantity(value.alternate, true)
    val code = MaterialTheme.typography.labelMedium.copy(fontFamily = LocalCodeFont.current, fontFeatureSettings = "tnum, lnum")
    FigureCard("Conversion. $source is $figure", figure) {
        Text(source, style = code, maxLines = 2, overflow = TextOverflow.Ellipsis)
    }
}
