@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

@Composable
internal fun ConversationActionSheet(kind: String, fields: Map<String, Any?>, state: MessengerState, command: Command, close: () -> Unit) {
    if (kind in listOf("delete_picker", "block_picker")) {
        val chats = state.chats.filter { it.id in (fields["peers"] as? List<*>).orEmpty() }
        val blocking = kind == "block_picker"
        var leave by remember { mutableStateOf(false) }
        AlertDialog(close, title = { Text(if (blocking) "Block contacts?" else "Delete conversations?") }, text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(if (blocking) "Messages and calls from the selected contacts will be blocked. Group memberships stay unchanged."
                    else "Delete existing history for your account, including linked devices. Other participants keep their copies. New messages can reopen the conversation.")
                if (!blocking && chats.any { it.group }) Toggle("Also leave selected groups", leave) { leave = it }
            }
        }, dismissButton = { SigilTextButton(close) { Text("Cancel") } }, confirmButton = {
            SigilTextButton({
                if (blocking) chats.filter { !it.group && it.id != "self" }.forEach { command("block", mapOf("peer" to it.id, "active" to true)) }
                else chats.forEach { command("delete_conversation", mapOf("peer" to it.id, "leave" to (leave && it.group))) }
                close()
            }, enabled = !state.busy) { Text(if (blocking) "Block" else "Delete") }
        })
        return
    }
    ModalBottomSheet(close, containerColor = MaterialTheme.colorScheme.background) {
        Text(if (kind == "snooze_picker") "Snooze notifications" else "Forward to", Modifier.padding(24.dp), style = MaterialTheme.typography.headlineSmall)
        if (kind == "snooze_picker") {
            listOf("Resume notifications" to null, "For one hour" to 3600L, "For eight hours" to 28800L, "For one day" to 86400L, "For one week" to 604800L).forEach { (label, seconds) ->
                SigilTextButton({ (fields["peers"] as? List<*>)?.filterIsInstance<String>()?.forEach { command("snooze", mapOf("peer" to it, "seconds" to seconds)) }; close() }, Modifier.fillMaxWidth()) { Text(label) }
            }
        } else {
            Text("Send a copy. Interactive cards and mixed messages are sent as text snapshots.", Modifier.padding(horizontal = 24.dp, vertical = 8.dp), style = MaterialTheme.typography.bodySmall)
            LazyColumn(Modifier.heightIn(max = 440.dp)) {
                item { SettingRow("edit_note", "Note to Self", "") { command("forward", mapOf("source" to fields["peer"], "peer" to "self", "author" to fields["author"], "message" to fields["message"])); close() } }
                items(state.chats.filter { it.verified && !it.hidden && it.id != "self" }, key = { it.id }) { chat -> ChatRow(chat, open = {
                    command("forward", mapOf("source" to fields["peer"], "peer" to chat.id, "author" to fields["author"], "message" to fields["message"])); close()
                }) }
            }
        }
        Spacer(Modifier.height(16.dp))
    }
}
