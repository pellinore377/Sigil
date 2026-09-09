package org.sigil

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

@Composable
internal fun SavedHistoryPage(state: MessengerState, command: Command, back: () -> Unit, open: (String) -> Unit) {
    LaunchedEffect(Unit) { command("search", mapOf("query" to "", "category" to "History")) }
    Column(Modifier.fillMaxSize()) {
        Header("Saved history", back)
        Text("History available on this device, including restored conversations.", Modifier.padding(20.dp), style = MaterialTheme.typography.bodySmall)
        LazyColumn(Modifier.weight(1f), contentPadding = PaddingValues(horizontal = 20.dp)) {
            items(state.searchHits.distinctBy { it.peer }, key = { it.peer }) { hit ->
                Column(Modifier.fillMaxWidth().clickable { open(hit.peer) }.padding(vertical = 16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(state.chats.find { it.id == hit.peer }?.name ?: if (hit.peer == "self") "Note to Self" else "Saved conversation", style = MaterialTheme.typography.titleMedium)
                    Text(hit.text, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    Text(hit.time, style = MaterialTheme.typography.labelSmall)
                }
                HorizontalDivider()
            }
            if (state.searching) item { LinearProgressIndicator(Modifier.fillMaxWidth()) }
            else if (state.searchMore) item { TextButton({ command("search_more", emptyMap()) }) { Text("Load older history") } }
            else if (state.searchHits.isEmpty()) item { Text("No saved history yet.", Modifier.padding(vertical = 24.dp)) }
        }
    }
}

@Composable
internal fun SavedConversationPage(state: MessengerState, analyze: (String) -> String, command: Command, back: () -> Unit) {
    var details by remember(state.selected) { mutableStateOf<String?>(null) }
    Column(Modifier.fillMaxSize()) {
        Header("Saved conversation", back)
        Text("Saved history. Verify contacts or rejoin the group to resume messaging.", Modifier.padding(horizontal = 20.dp, vertical = 10.dp), style = MaterialTheme.typography.bodySmall)
        LazyColumn(Modifier.weight(1f), reverseLayout = true, contentPadding = PaddingValues(16.dp)) {
            itemsIndexed(state.messages, key = { _, message -> message.author + message.id }) { index, message ->
                val older = state.messages.getOrNull(index + 1)
                Column(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
                    if (showSeparator(message, older)) Text(message.separator, Modifier.align(Alignment.CenterHorizontally).padding(vertical = 12.dp), style = MaterialTheme.typography.labelMedium)
                    Box(Modifier.align(if (message.mine) Alignment.End else Alignment.Start).widthIn(max = 320.dp).clickable { details = message.author + message.id }) {
                        SelectionContainer { MessageBubble(message, older?.author == message.author, state.messages.getOrNull(index - 1)?.author == message.author, analyze) }
                    }
                    if (details == message.author + message.id) Row(Modifier.align(if (message.mine) Alignment.End else Alignment.Start).padding(6.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) { Text(message.time, style = MaterialTheme.typography.labelSmall); Glyph("lock", 12, "Restored encrypted message") }
                }
            }
            if (state.more) item { TextButton({ command("older", emptyMap()) }) { Text("Load older messages") } }
        }
    }
}
