package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

/** One vCard property as it is shown: a leading glyph, the value, and the TYPE= label under it ("cell", "home"). */
data class VcardField(val glyph: String, val value: String, val label: String = "")

/** Maps a vCard property name onto a glyph that exists in the bundled Material Symbols subset. */
internal fun vcardGlyph(property: String) = when (property.uppercase()) {
    "TEL" -> "call"; "EMAIL" -> "mail"; "ADR" -> "place"; "URL" -> "link"; "BDAY", "ANNIVERSARY" -> "cake"
    "ORG", "TITLE", "ROLE" -> "work"; "NICKNAME" -> "badge"; "LANG" -> "language"; "IMPP" -> "devices"; else -> "info"
}

@Composable
fun VcardCard(name: String, fields: List<VcardField>, modifier: Modifier = Modifier) {
    val clipboard = LocalClipboardManager.current
    val quiet = MaterialTheme.colorScheme.onSurfaceVariant
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Avatar(name, 56)
            Text(name, Modifier.weight(1f), style = MaterialTheme.typography.titleLarge, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
        fields.forEach { field ->
            Row(Modifier.fillMaxWidth().heightIn(min = 56.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                CompositionLocalProvider(LocalContentColor provides quiet) { Glyph(field.glyph, 20) }
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(field.value, style = MaterialTheme.typography.bodyLarge, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    if (field.label.isNotEmpty()) Text(field.label, style = MaterialTheme.typography.labelSmall, color = quiet, maxLines = 1)
                }
                SigilIconButton({ clipboard.setText(AnnotatedString(field.value)) }) {
                    CompositionLocalProvider(LocalContentColor provides quiet) { Glyph("content_copy", 20, "Copy ${field.label.ifEmpty { "value" }}") }
                }
            }
        }
    }
}
