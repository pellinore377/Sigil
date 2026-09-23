package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

// The only five kinds the owner kept an expanded view for; everything else reads in place.
internal fun ChatMessage.detailsPart() = parts.firstOrNull { it.chart != null || it.diagram != null || it.table != null || it.recipe != null || it.utility?.qr != null }

@Composable
internal fun CardDetails(message: ChatMessage, dismiss: () -> Unit) {
    val part = message.detailsPart() ?: return
    if (part.chart != null) ChartDetails(part.chart, dismiss)
    else if (part.diagram != null) DiagramDetails(part.diagram, dismiss)
    else if (part.table != null) TableDetails(part.table, dismiss)
    else if (part.recipe != null) RecipeDetails(message, part, dismiss)
    else if (part.utility?.qr != null) QrDetails(part.utility, message.mine, dismiss = dismiss)
}

internal fun ChatMessage.bareRandomizers() = reply == null && attachment == null &&
    parts.any { it.utility?.motion?.kind in listOf("dice", "coin", "choice") } &&
    parts.all { it.kind == "text" || it.utility?.motion?.kind in listOf("dice", "coin", "choice") }

// Opens the shared account's conversation; only the recipient gets the action.
private fun contactOpen(message: ChatMessage, part: MessagePart, command: Command?): (() -> Unit)? =
    if (message.mine || command == null || (part.contact == null && part.utility?.qr?.kind != "contact")) null
    else ({ command("contact_open", mapOf("peer" to message.peer, "author" to message.author, "message" to message.id, "card" to part.id)) })

// The bubble draws a card's end cue around itself, so a ring can travel outside the clip a card lives in.
internal val LocalBubbleCue=staticCompositionLocalOf<MutableFloatState?> {null}

@Composable
internal fun MessageCards(message: ChatMessage, analyze: (String) -> String, command: Command?, bareObjects: Boolean = false) {
    if (!bareObjects && message.inlineMathLine()) { InlineMathMessage(message, analyze); return }
    if (!bareObjects && message.inlineKeysLine()) { InlineKeysMessage(message, analyze); return }
    Column(verticalArrangement = Arrangement.spacedBy(12.dp), horizontalAlignment = if (bareObjects && message.mine) Alignment.End else Alignment.Start) {
        message.parts.forEachIndexed { index,part ->
            val palette = swatchRun(message.parts, index)
            if (palette != null) { if (palette.isNotEmpty()) SwatchPalette(palette) }
            else if (part.kind == "text") {
                val text: @Composable () -> Unit = { if (part.rich != null) RichMessageText(part.rich) else MessageText(part.text, analyze) }
                if (bareObjects) Surface(shape = RoundedCornerShape(16.dp), color = if (message.mine) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceVariant,
                    contentColor = if (message.mine) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurfaceVariant) {
                    Box(Modifier.padding(horizontal = 14.dp, vertical = 10.dp)) { text() }
                } else text()
            }
            else if (part.kind == "location") LocationCard(message, part, analyze, command, bareObjects)
            else if (part.table != null) TableCard(part.table)
            else if (part.recipe != null) RecipeCard(message, part)
            else if (part.chart != null) ChartCard(part.chart)
            else if (part.diagram != null) DiagramCard(part.diagram)
            else if (part.utility != null) CompositionLocalProvider(LocalMaterialOrdinal provides message.parts.take(index).count {it.utility?.motion!=null}) {UtilityCard(part.utility,message.mine,contactOpen(message,part,command))}
            else if (part.service != null) ServiceCard(part.service)
            else if (part.contact != null) ContactCard(part.contact, contactOpen(message,part,command))
            else if (part.kind == "checklist" || part.kind == "task" || part.kind == "recurring") ChecklistCard(message,part,analyze,command)
            else if (part.kind == "poll") PollCard(message,part,analyze,command)
            else if (part.kind == "note") NoteCard(part,analyze)
            else if (part.kind == "reminder") ReminderCard(part,analyze)
            else if (part.kind == "countdown") CountdownCard(part,analyze)
            else if (part.kind == "ago") AgoCard(part,analyze)
            else if (part.kind == "timer") TimerCard(part,preview=message.id=="preview")
            else StandardCard(part,analyze)
        }
    }
}
