package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

@Composable
internal fun ConversationSettings(chat: ChatSummary, busy: Boolean, command: Command, back: () -> Unit) {
    var verify by remember { mutableStateOf(false) }
    if (verify) VerificationDialog(chat, busy, command) { verify = false }
    fun preference(name: String, value: Boolean) = command("organize", mapOf("peer" to chat.id, "value" to mapOf(name to value)))
    SettingsDetailLayout("Conversation settings", back, continuous = true) {
        Column(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            PresenceAvatar(chat, 64)
            Text(chat.name, style = MaterialTheme.typography.headlineSmall, maxLines = 2, overflow = TextOverflow.Ellipsis)
            if (chat.address.isNotEmpty() && chat.id != "self") Text(chat.address, style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
        SettingsSection("Only this conversation") {
            SettingsNote("These switches customize this conversation. Other conversations keep their settings.")
            if (chat.id != "self") {
                SettingsToggle("Read receipts", "Let this contact see when you have read a message", chat.readReceipts) { preference("ReadReceipts", it) }
                SettingsToggle("Typing indicators", "Let this contact see when you are writing", chat.typingIndicators) { preference("TypingIndicators", it) }
                if (!chat.group) SettingsToggle("Share activity status", "Let this contact see when you were last active", chat.presenceSharing) { preference("PresenceSharing", it) }
            }
            SettingsToggle("Pin conversation", "Keep this conversation at the top of the inbox", chat.pinned) { preference("ConversationPin", it) }
        }
        SettingsSection("Manage") {
            if (chat.devices.isNotEmpty()) SettingsLink("verified_user", "Device verification", "Compare encryption identities") { verify = true }
            if (chat.id != "self") SettingsLink("notifications_paused", if (chat.snoozed) "Snoozed" else "Snooze", "Pause message notifications") { command("snooze_picker", mapOf("peers" to listOf(chat.id))) }
            if (!chat.group && chat.id != "self") SettingsLink("block", "Block contact", "Stop messages from this contact") { command("block_picker", mapOf("peers" to listOf(chat.id))) }
            DestructiveLink("delete", "Delete conversation", "Clear your history on this account") { command("delete_picker", mapOf("peers" to listOf(chat.id))) }
        }
    }
}

@Composable
private fun DestructiveLink(icon: String, title: String, detail: String, click: () -> Unit) {
    val error = MaterialTheme.colorScheme.error
    Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(18.dp)).clickable(role = Role.Button, onClick = click)
        .heightIn(min = 72.dp).padding(horizontal = 12.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp)) {
        CompositionLocalProvider(LocalContentColor provides error) { Glyph(icon, 24) }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(title, style = MaterialTheme.typography.titleMedium, color = error, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}
