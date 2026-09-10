package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.*
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.unit.dp

@Composable
internal fun MessageCards(message: ChatMessage, analyze: (String) -> String, command: Command?) {
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        message.parts.forEach { part ->
            if (part.kind == "text") { if (part.rich != null) RichMessageText(part.rich) else MessageText(part.text, analyze) }
            else if (part.kind == "location") LocalLocationContent.current(message, part, command)
            else if (part.table != null) TableCard(part.table)
            else if (part.recipe != null) RecipeCard(message, part)
            else if (part.chart != null) ChartCard(part.chart)
            else if (part.diagram != null) DiagramCard(part.diagram)
            else if (part.utility != null) UtilityCard(part.utility)
            else if (part.service != null) ServiceCard(part.service)
            else Column(Modifier.widthIn(min = 180.dp, max = 280.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                val icon = when (part.kind) { "note" -> "description"; "checklist" -> "checklist"; "task" -> "assignment"; "poll" -> "ballot"; "reminder" -> "notifications_active"; "timer", "countdown", "ago" -> "timer"; else -> "article" }
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) { Glyph(icon, 18); Text(part.kind.replaceFirstChar { it.uppercase() }, style = MaterialTheme.typography.labelMedium) }
                if (part.rich != null) RichMessageText(part.rich) else MessageText(part.text, analyze)
                if (part.date.isNotEmpty()) Text(part.date, style = MaterialTheme.typography.bodySmall)
                part.items.forEach { item ->
                    val act: () -> Unit = {
                        val fields = mutableMapOf<String, Any?>("peer" to message.peer, "author" to message.author, "message" to message.id, "card" to part.id)
                        if (part.kind == "poll") fields["choices"] = if (part.multiple) part.items.filter { if (it.id == item.id) !it.checked else it.checked }.map { it.id } else if (item.checked) emptyList<String>() else listOf(item.id)
                        else { fields["item"] = item.id; fields["checked"] = !item.checked }
                        command?.invoke("card_action", fields)
                    }
                    Row(Modifier.fillMaxWidth().heightIn(min = 44.dp).background(LocalContentColor.current.copy(alpha = if (item.checked) .13f else .04f), RoundedCornerShape(10.dp))
                        .clickable(enabled = command != null && item.enabled, role = if (part.kind == "poll" && !part.multiple) Role.RadioButton else Role.Checkbox, onClick = act)
                        .semantics { if (part.kind == "poll" && !part.multiple) selected = item.checked else toggleableState = ToggleableState(item.checked) }
                        .padding(horizontal = 8.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Glyph(if (part.kind == "poll" && !part.multiple) if (item.checked) "radio_button_checked" else "radio_button_unchecked" else if (item.checked) "check_box" else "check_box_outline_blank", 20)
                        if (item.rich != null) RichMessageText(item.rich, Modifier.weight(1f), MaterialTheme.typography.bodyMedium)
                        else Text(item.text, Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium)
                        item.count?.let { Text(it.toString(), style = MaterialTheme.typography.labelMedium) }
                    }
                }
                if (part.kind == "poll") Text(if (part.closed) "Voting closed" else part.voters?.let { "$it ${if (it == 1L) "voter" else "voters"}" } ?: "Vote to see results", style = MaterialTheme.typography.labelSmall)
            }
        }
    }
}
