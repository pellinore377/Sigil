package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.unit.dp

@Composable
internal fun ConversationSettings(chat: ChatSummary, busy: Boolean, command: Command, back: () -> Unit) {
    var verify by remember { mutableStateOf(false) }
    if (verify) VerificationDialog(chat, busy, command) { verify = false }
    fun preference(name: String, value: Boolean) = command("organize", mapOf("peer" to chat.id, "value" to mapOf(name to value)))
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
        Header("Conversation settings", back)
        Column(Modifier.padding(24.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            PresenceAvatar(chat, 64)
            Text(chat.name, style = MaterialTheme.typography.headlineMedium)
            if (chat.address.isNotEmpty() && chat.id != "self") Text(chat.address, style = MaterialTheme.typography.bodySmall)
            Text("Only this conversation", style = MaterialTheme.typography.titleLarge)
            Text("These switches customize this conversation. Other conversations keep their settings.")
            if (chat.id != "self") {
                Toggle("Read receipts", chat.readReceipts) { preference("ReadReceipts", it) }
                Toggle("Typing indicators", chat.typingIndicators) { preference("TypingIndicators", it) }
                if (!chat.group) Toggle("Share activity status", chat.presenceSharing) { preference("PresenceSharing", it) }
            }
            Toggle("Pin conversation", chat.pinned) { preference("ConversationPin", it) }
        }
        if (chat.devices.isNotEmpty()) SettingRow("verified_user", "Device verification", "Compare encryption identities") { verify = true }
        if (chat.id != "self") SettingRow("notifications_paused", if (chat.snoozed) "Snoozed" else "Snooze", "Pause message notifications") { command("snooze_picker", mapOf("peers" to listOf(chat.id))) }
        if (!chat.group && chat.id != "self") SettingRow("block", "Block contact", "Stop messages from this contact") { command("block_picker", mapOf("peers" to listOf(chat.id))) }
        SettingRow("delete", "Delete conversation", "Clear your history on this account") { command("delete_picker", mapOf("peers" to listOf(chat.id))) }
    }
}
