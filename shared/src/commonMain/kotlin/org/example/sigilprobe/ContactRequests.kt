package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp

@Composable
internal fun RequestChip(label: String, icon: String = "person_add") {
    Surface(shape = RoundedCornerShape(8.dp), color = MaterialTheme.colorScheme.primaryContainer, contentColor = MaterialTheme.colorScheme.onPrimaryContainer) {
        Row(Modifier.padding(horizontal = 9.dp, vertical = 5.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(5.dp)) {
            Glyph(icon, 15); Text(label, style = MaterialTheme.typography.labelSmall)
        }
    }
}

@Composable
internal fun ContactRequestPanel(chat: ChatSummary, busy: Boolean, command: Command) {
    fun act(action: String) = command("contact_request", mapOf("peer" to chat.id, "action" to action))
    Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 24.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(14.dp)) {
        if (chat.identityReview != null && chat.request == "accepted") {
            Text("${chat.name}’s encryption identity changed. This can happen after account recovery or reinstalling.", textAlign = TextAlign.Center, style = MaterialTheme.typography.bodyMedium)
            SigilButton({ command("identity_accept", mapOf("peer" to chat.id, "review" to chat.identityReview)) }, enabled = !busy) { Text("Continue with new identity") }
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally)) {
                SigilOutlinedButton({ command("contact_qr", mapOf("action" to "scan", "peer" to chat.id, "review" to chat.identityReview)) }, enabled = !busy) { Text("Scan QR to confirm") }
                SigilTextButton({ act("block") }, enabled = !busy) { Text("Block") }
            }
            return@Column
        }
        Text(when(chat.request) {
            "incoming" -> "${chat.name} would like to connect."
            "sending" -> "Your request is waiting to reach the server. Your message stays in your drafts."
            "pending" -> "Waiting for ${chat.name} to accept your request."
            "resolving" -> "Saving your decision…"
            "blocked" -> "This contact is blocked."
            "declined_incoming" -> "You declined this request. You can still accept it."
            "declined" -> "Your request was declined. ${chat.name} can still choose to accept it."
            "expired" -> "This request expired. You can send a new one."
            "accepted" -> "Request accepted. Preparing your encrypted conversation…"
            else -> "Send ${chat.name} a request to start an encrypted conversation."
        }, textAlign = TextAlign.Center, style = MaterialTheme.typography.bodyMedium)
        when (chat.request) {
            "incoming" -> FlowRow(horizontalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterHorizontally)) {
                SigilOutlinedButton({ act("decline") }, enabled = !busy) { Text("Decline") }
                SigilButton({ act("accept") }, enabled = !busy) { Text("Accept") }
            }
            "declined_incoming" -> SigilButton({ act("accept") }, enabled = !busy) { Text("Accept instead") }
            "blocked" -> SigilOutlinedButton({ command("block", mapOf("peer" to chat.id, "active" to false)) }, enabled = !busy) { Text("Unblock") }
            "none", "expired" -> SigilButton({ act("send") }, enabled = !busy) { Text("Send request") }
            "pending" -> RequestChip("Pending", "schedule")
            "sending", "resolving", "accepted" -> RequestChip(if (chat.request == "accepted") "Connecting" else "Waiting to sync", "sync")
        }
    }
}
