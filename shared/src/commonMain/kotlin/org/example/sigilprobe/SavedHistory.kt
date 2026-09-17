package org.sigil

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

@Composable
internal fun SavedHistoryPage(state: MessengerState, command: Command, back: () -> Unit, open: (String) -> Unit) {
    LaunchedEffect(Unit) { command("search", mapOf("query" to "", "category" to "History")) }
    val compact = LocalAppearance.current.compact
    Column(Modifier.fillMaxSize(), horizontalAlignment = Alignment.CenterHorizontally) {
        Header("Saved history", back)
        Column(Modifier.widthIn(max = 680.dp).fillMaxWidth().weight(1f)) {
            Text("History available on this device, including restored conversations.", Modifier.padding(horizontal = 20.dp, vertical = 12.dp),
                style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            LazyColumn(Modifier.weight(1f)) {
                items(state.searchHits.distinctBy { it.peer }, key = { it.peer }) { hit ->
                    Row(itemMotion().fillMaxWidth().padding(horizontal = 12.dp).clip(RoundedCornerShape(18.dp))
                        .heightIn(min = if (compact) 64.dp else 88.dp).clickable(role = Role.Button) { open(hit.peer) }
                        .padding(horizontal = 8.dp, vertical = if (compact) 6.dp else 16.dp), verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            Text(state.chats.find { it.id == hit.peer }?.name ?: if (hit.peer == "self") "Note to Self" else "Saved conversation",
                                style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Text(hit.text, maxLines = 2, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            Text(hit.time, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                    }
                }
                if (state.searching) item("progress") { LinearProgressIndicator(Modifier.fillMaxWidth()) }
                else if (state.searchMore) item("more") { SigilTextButton({ command("search_more", emptyMap()) }, Modifier.fillMaxWidth(), enabled = !state.searching) { Text("Load older history") } }
                else if (state.searchHits.isEmpty()) item("empty") {
                    Box(Modifier.fillMaxWidth().padding(32.dp), contentAlignment = Alignment.Center) { Text("No saved history yet.", color = MaterialTheme.colorScheme.onSurfaceVariant) }
                }
            }
        }
    }
}

@Composable
internal fun SavedConversationPage(state: MessengerState, analyze: (String) -> String, command: Command, back: () -> Unit) {
    var details by remember(state.selected) { mutableStateOf<String?>(null) }
    // MessageDetails only reads the summary for receipts, which this page never shows.
    val chat = state.chats.find { it.id == state.selected } ?: ChatSummary(state.selected.orEmpty(), "", "", "", false, emptyList())
    Column(Modifier.fillMaxSize(), horizontalAlignment = Alignment.CenterHorizontally) {
        Header("Saved conversation", back)
        Column(Modifier.widthIn(max = 920.dp).fillMaxWidth().weight(1f)) {
            Text("Saved history. Verify contacts or rejoin the group to resume messaging.", Modifier.padding(horizontal = 20.dp, vertical = 10.dp),
                style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            LazyColumn(Modifier.weight(1f), reverseLayout = true, contentPadding = PaddingValues(16.dp)) {
                itemsIndexed(state.messages, key = { _, message -> message.author + message.id }) { index, message ->
                    val older = state.messages.getOrNull(index + 1)
                    val grouped = older?.author == message.author && !showSeparator(message, older)
                    val key = message.author + message.id
                    Column(itemMotion().fillMaxWidth().padding(top = if (grouped) 3.dp else 12.dp)) {
                        if (showSeparator(message, older)) Text(message.separator.ifEmpty { message.time }, Modifier.align(Alignment.CenterHorizontally).padding(top = 6.dp, bottom = 14.dp),
                            style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        Box(Modifier.align(if (message.mine) Alignment.End else Alignment.Start).widthIn(max = MessageBubbleMaxWidth.dp).clickable { details = if (details == key) null else key }) {
                            SelectionContainer { MessageBubble(message, grouped, state.messages.getOrNull(index - 1)?.author == message.author, analyze) }
                        }
                        Box(Modifier.align(if (message.mine) Alignment.End else Alignment.Start)) { MessageDetails(message, details == key, false, chat, state.people) }
                    }
                }
                if (state.more) item("more") { SigilTextButton({ command("older", emptyMap()) }, Modifier.fillMaxWidth(), enabled = !state.busy) { Text("Load older messages") } }
            }
        }
    }
}
