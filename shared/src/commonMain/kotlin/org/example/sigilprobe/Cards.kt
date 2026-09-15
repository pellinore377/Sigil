package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

internal fun ChatMessage.bareRandomizers() = reply == null && attachment == null &&
    parts.any { it.utility?.motion?.kind in listOf("dice", "coin", "choice") } &&
    parts.all { it.kind == "text" || it.utility?.motion?.kind in listOf("dice", "coin", "choice") }

@Composable
internal fun MessageCards(message: ChatMessage, analyze: (String) -> String, command: Command?, bareObjects: Boolean = false) {
    Column(verticalArrangement = Arrangement.spacedBy(12.dp), horizontalAlignment = if (bareObjects && message.mine) Alignment.End else Alignment.Start) {
        message.parts.forEachIndexed { index,part ->
            if (part.kind == "text") {
                val text: @Composable () -> Unit = { if (part.rich != null) RichMessageText(part.rich) else MessageText(part.text, analyze) }
                if (bareObjects) Surface(shape = RoundedCornerShape(16.dp), color = if (message.mine) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceVariant,
                    contentColor = if (message.mine) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurfaceVariant) {
                    Box(Modifier.padding(horizontal = 14.dp, vertical = 10.dp)) { text() }
                } else text()
            }
            else if (part.kind == "location") {
                LocalLocationContent.current(message, part, command)
                if(part.text.isNotBlank() && part.text !in listOf("My location","Dropped pin")) Surface(shape=RoundedCornerShape(16.dp),color=if(message.mine)MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceVariant,contentColor=if(message.mine)MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurfaceVariant) {
                    Box(Modifier.padding(horizontal=14.dp,vertical=10.dp)) {if(part.rich!=null)RichMessageText(part.rich) else MessageText(part.text,analyze)}
                }
            }
            else if (part.table != null) TableCard(part.table)
            else if (part.recipe != null) RecipeCard(message, part)
            else if (part.chart != null) ChartCard(part.chart)
            else if (part.diagram != null) DiagramCard(part.diagram)
            else if (part.utility != null) CompositionLocalProvider(LocalMaterialOrdinal provides message.parts.take(index).count {it.utility?.motion!=null}) {UtilityCard(part.utility)}
            else if (part.service != null) ServiceCard(part.service)
            else if (part.contact != null) ContactCard(part.contact, command?.let { action -> {
                action("contact_open",mapOf("peer" to message.peer,"author" to message.author,"message" to message.id,"card" to part.id))
            } })
            else StandardCard(message,part,analyze,command)
        }
    }
}
