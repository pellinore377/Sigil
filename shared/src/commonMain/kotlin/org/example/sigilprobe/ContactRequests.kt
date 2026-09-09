package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

@Composable
internal fun ContactRequestPanel(chat: ChatSummary, busy: Boolean, command: Command, verify: () -> Unit) {
    fun act(action: String) = command("contact_request", mapOf("peer" to chat.id, "action" to action))
    Column(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 8.dp)) {
        Text(when(chat.request) {
            "incoming" -> "${chat.name} would like to connect. Accept, then compare device fingerprints before messaging."
            "sending" -> "Your request is saved and waiting to reach the server. Your message stays in your drafts."
            "pending" -> "Request sent. Waiting for ${chat.name} to accept."
            "resolving" -> "Your decision is saved and waiting for the server."
            "blocked" -> "This contact is blocked."
            "declined" -> "This request was declined."
            "expired" -> "This request expired."
            "accepted" -> "Request accepted. Compare fingerprints with your contact before messaging."
            else -> if (chat.devices.isEmpty()) "Send a request to connect. Your message stays in your drafts until verification." else "Compare fingerprints before messaging."
        }, style = MaterialTheme.typography.bodySmall)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            if (chat.request == "incoming") {
                TextButton({ act("decline") }, enabled = !busy) { Text("Decline") }
                TextButton({ act("block") }, enabled = !busy) { Text("Block") }
                Button({ act("accept") }, enabled = !busy) { Text("Accept") }
            } else {
                if (chat.request == "blocked") TextButton({ command("block", mapOf("peer" to chat.id, "active" to false)) }, enabled = !busy) { Text("Unblock") }
                if (chat.devices.isEmpty() && chat.request in listOf("none", "expired")) Button({ act("send") }, enabled = !busy) { Text("Send request") }
                if (chat.request in listOf("pending", "sending", "accepted", "resolving")) TextButton({ act("refresh") }, enabled = !busy) { Text("Refresh") }
                if (chat.devices.isNotEmpty() && chat.request !in listOf("blocked", "declined", "resolving")) TextButton(verify, enabled = !busy) { Text("Verify devices") }
            }
        }
    }
}
