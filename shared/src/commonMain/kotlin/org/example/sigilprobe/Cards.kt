package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.runtime.*
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
            else if (part.contact != null) ContactCard(part.contact, command?.let { action -> {
                action("contact_open",mapOf("peer" to message.peer,"author" to message.author,"message" to message.id,"card" to part.id))
            } })
            else StandardCard(message,part,analyze,command)
        }
    }
}
