@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class, androidx.compose.material3.ExperimentalMaterial3Api::class)
package org.sigil

import androidx.compose.animation.*
import androidx.compose.animation.core.*
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.foundation.lazy.grid.*
import androidx.compose.foundation.shape.*
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.*
import androidx.compose.ui.focus.*
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.*
import kotlinx.coroutines.delay

@Composable
internal fun InboxFab(visible: Boolean, modifier: Modifier, create: () -> Unit) {
    AnimatedVisibility(visible, modifier, enter = slideInVertically(tween(MotionMillis)) { it } + fadeIn(), exit = slideOutVertically(tween(MotionMillis)) { it } + fadeOut()) {
        FloatingActionButton(create, Modifier.padding(20.dp), shape = RoundedCornerShape(18.dp), containerColor = MaterialTheme.colorScheme.inverseSurface, contentColor = MaterialTheme.colorScheme.inverseOnSurface) { Glyph("edit_square", 27, "New conversation") }
    }
}

@Composable
internal fun InboxHeader(page: String, query: String, update: (String) -> Unit, selected: Set<String>, state: MessengerState, command: Command,
    clear: () -> Unit, collections: () -> Unit, search: () -> Unit, notes: () -> Unit, back: () -> Unit) {
    val opened = page in listOf("search", "notes")
    val progress by animateFloatAsState(if (opened) 1f else 0f, tween(MotionMillis), label = "Header transformation")
    val focus = remember { FocusRequester() }
    val keyboard = LocalSoftwareKeyboardController.current
    LaunchedEffect(page) { if (page == "search") { delay(MotionMillis.toLong()); focus.requestFocus(); keyboard?.show() } }
    AnimatedContent(selected.isNotEmpty(), transitionSpec = {
        (slideInHorizontally(tween(MotionMillis)) { it } + fadeIn()) togetherWith (slideOutHorizontally(tween(MotionMillis)) { -it } + fadeOut())
    }, label = "Selection toolbar") { selecting ->
        if (selecting) {
            Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Symbol("close", "Cancel selection", clear); Text(selected.size.toString(), Modifier.padding(end = 8.dp))
                fun change(value: Map<String, Any?>) { selected.forEach { command("organize", mapOf("peer" to it, "value" to value)) }; clear() }
                Symbol("push_pin", "Pin or unpin conversations") { change(mapOf("ConversationPin" to !state.chats.filter { it.id in selected }.all { it.pinned })) }
                Symbol("drive_file_move", "Add to collection", collections)
                Symbol("snooze", "Snooze or unsnooze") { command("snooze_picker", mapOf("peers" to selected.toList())) }
                Symbol("mark_chat_unread", "Mark read or unread") {
                    if (state.chats.filter { it.id in selected }.all { it.unread > 0 }) { selected.forEach { command("mark_read", mapOf("peer" to it)) }; clear() }
                    else change(mapOf("Unread" to true))
                }
                if (state.chats.any { it.id in selected && !it.group && it.id != "self" }) Symbol("block", "Block selected contacts") { command("block_picker", mapOf("peers" to selected.toList())) }
                Symbol("delete", "Delete conversations") { command("delete_picker", mapOf("peers" to selected.toList())) }
            }
        } else BoxWithConstraints(Modifier.fillMaxWidth().height(76.dp).padding(horizontal = 12.dp)) {
            Text("Sigil", Modifier.align(Alignment.CenterStart).padding(start = 8.dp).alpha(1f - progress), style = MaterialTheme.typography.displaySmall)
            val x = (maxWidth - 96.dp) * (1f - progress)
            Box(Modifier.offset(x = x).align(Alignment.CenterStart)) {
                Crossfade(opened, animationSpec = tween(MotionMillis), label = "Search to back") { backIcon ->
                    if (backIcon) Symbol("chevron_left", "Back", back) else Symbol("search", "Search conversations", search)
                }
            }
            AnimatedVisibility(!opened, Modifier.align(Alignment.CenterEnd), enter = fadeIn(tween(MotionMillis)), exit = fadeOut(tween(120))) { Symbol("description", "Conversation notes", notes) }
            AnimatedVisibility(opened, Modifier.align(Alignment.CenterStart).padding(start = 56.dp, end = 8.dp).fillMaxWidth(), enter = fadeIn(tween(MotionMillis)), exit = fadeOut(tween(100))) {
              BasicTextField(query, update, Modifier.fillMaxWidth().focusRequester(focus),
                singleLine = true, textStyle = MaterialTheme.typography.titleMedium.copy(color = MaterialTheme.colorScheme.onBackground),
                decorationBox = { inner -> Box { if (query.isEmpty()) Text(if (page == "notes") "Search notes" else "Search all conversations", color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.titleMedium); inner() } })
            }
        }
    }
}
@Composable
internal fun Inbox(state: MessengerState, collection: String, choose: (String) -> Unit, selected: Set<String>, select: (String) -> Unit,
    open: (String) -> Unit, read: (String) -> String?) {
    Box(Modifier.fillMaxSize()) {
        Column {
            if (state.collectionsEnabled) LazyRow(contentPadding = PaddingValues(horizontal = 20.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                items(listOf(CollectionItem("", "All", "apps")) + state.collections, key = { it.id }) { item ->
                    val active = item.id == collection
                    Surface(Modifier.widthIn(min = 64.dp).clickable { choose(item.id) }, shape = RoundedCornerShape(10.dp),
                        color = if (active) MaterialTheme.colorScheme.inverseSurface else MaterialTheme.colorScheme.background,
                        contentColor = if (active) MaterialTheme.colorScheme.inverseOnSurface else MaterialTheme.colorScheme.onSurfaceVariant,
                        border = if (active) null else BorderStroke(0.5.dp, MaterialTheme.colorScheme.outlineVariant)) {
                        Column(Modifier.padding(12.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            Glyph(item.icon, 25); if (read("collection_labels") != "false") Text(item.name, style = MaterialTheme.typography.bodyMedium)
                        }
                    }
                }
            }
            LazyColumn(contentPadding = PaddingValues(bottom = 92.dp)) {
                items(state.chats.filter { !it.hidden && !it.contactOnly && (collection.isEmpty() || collection in it.collections) }, key = { it.id }) { chat ->
                    ChatRow(chat, chat.id in selected, Modifier.animateItem(), { if (selected.isNotEmpty()) select(chat.id) else open(chat.id) }, { select(chat.id) })
                }
            }
        }
        if (state.chats.none { !it.hidden && !it.contactOnly }) Column(Modifier.align(Alignment.Center).padding(32.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            Text("Your correspondence starts here.", style = MaterialTheme.typography.headlineSmall)
            Spacer(Modifier.height(12.dp)); Text("Add someone by their Sigil address.")
        }
    }
}
@Composable
internal fun ChatRow(chat: ChatSummary, selected: Boolean = false, modifier: Modifier = Modifier, open: () -> Unit, hold: () -> Unit = {}) {
    Row(modifier.fillMaxWidth().combinedClickable(onClick = open, onLongClick = hold).padding(horizontal = 20.dp, vertical = 11.dp), verticalAlignment = Alignment.CenterVertically) {
        if (selected) Surface(Modifier.size(48.dp), shape = CircleShape, color = MaterialTheme.colorScheme.inverseSurface, contentColor = MaterialTheme.colorScheme.inverseOnSurface) { Box(contentAlignment = Alignment.Center) { Glyph("check", 26) } }
        else PresenceAvatar(chat)
        Column(Modifier.weight(1f).padding(horizontal = 12.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(chat.name, style = MaterialTheme.typography.titleLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(if (chat.typing.isNotEmpty()) "Typing…" else chat.preview, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Column(horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(chat.time, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                if (chat.unread > 0) Surface(shape = RoundedCornerShape(6.dp), color = MaterialTheme.colorScheme.inverseSurface, contentColor = MaterialTheme.colorScheme.inverseOnSurface) { Text(chat.unread.toString(), Modifier.padding(horizontal = 6.dp), style = MaterialTheme.typography.labelMedium) }
                if (chat.snoozed) Glyph("snooze", 17, "Snoozed")
                if (chat.pinned) Glyph("push_pin", 17, "Pinned conversation")
            }
        }
    }
}
@Composable
internal fun PresenceAvatar(chat: ChatSummary, size: Int = 48, presence: Boolean = true) {
    Box {
        Avatar(chat.name, size)
        if (presence && !chat.group && chat.id != "self") Box(Modifier.align(Alignment.BottomEnd).size((size / 4 + 2).dp)
            .background(MaterialTheme.colorScheme.background, CircleShape).padding(2.dp)
            .background(when (chat.presence) { "active" -> Color(0xff4dba50); "away" -> Color(0xffe7ab37); "busy" -> Color(0xffce4545); else -> Color.Gray }, CircleShape)
            .semantics { contentDescription = chat.presence })
    }
}
private val searchCategories = listOf("Unread" to "mark_chat_unread", "Conversations" to "chat", "Requests" to "person_add", "Pinned" to "push_pin", "Images" to "image", "Videos" to "movie", "Places" to "location_on", "Links" to "link")
@Composable
internal fun SearchPage(state: MessengerState, query: String, category: String, choose: (String) -> Unit, open: (String) -> Unit, command: Command) {
    LaunchedEffect(category) { if (category == "Requests") command("contact_refresh", emptyMap()) }
    Column(Modifier.fillMaxSize()) {
        if (query.isEmpty() && category.isEmpty()) LazyVerticalGrid(GridCells.Fixed(2), contentPadding = PaddingValues(20.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            items(searchCategories) { (label, icon) -> Surface(Modifier.clickable { choose(label) }, shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.surfaceVariant) {
                Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) { Glyph(icon, 24); Text(label, style = MaterialTheme.typography.bodyLarge) }
            } }
        } else {
            if (category.isNotEmpty()) InputChip(true, { choose("") }, { Text(category) }, Modifier.padding(horizontal = 20.dp), trailingIcon = { Glyph("close", 16) })
            if (state.searching) LinearProgressIndicator(Modifier.fillMaxWidth())
            LazyColumn(contentPadding = PaddingValues(bottom = 16.dp)) {
                if (category == "Requests") items(state.chats.filter { it.request == "incoming" && (it.name.contains(query, true) || it.address.contains(query, true)) }, key = { "request:${it.id}" }) { chat -> ChatRow(chat, open = { open(chat.id) }) }
                if (category == "Requests") items(state.invitations, key = { "invite:${it.id}" }) { invitation ->
                    Column(Modifier.fillMaxWidth().padding(20.dp)) {
                        Text("Group invitation", style = MaterialTheme.typography.titleMedium)
                        Text("From ${state.chats.find { it.id == invitation.peer }?.name ?: "a contact"}")
                        Row { TextButton({ command("group_invitation", mapOf("invitation" to invitation.id, "accept" to false)) }) { Text("Decline") }; Button({ command("group_invitation", mapOf("invitation" to invitation.id, "accept" to true)) }) { Text("Accept") } }
                    }
                }
                if (category in listOf("", "Conversations", "Unread", "Pinned")) items(state.chats.filter { chat -> !chat.hidden && chat.name.contains(query, true) && when(category) { "Unread" -> chat.unread > 0; "Pinned" -> chat.pinned; else -> true } }, key = { "chat:${it.id}" }) { chat -> ChatRow(chat, open = { open(chat.id) }) }
                items(state.searchHits.filter { hit -> when(category) { "Pinned" -> hit.pinned; "Images", "Videos", "Places", "Links" -> hit.kind == category; "Unread" -> state.chats.any { it.id == hit.peer && it.unread > 0 }; "Requests", "Conversations" -> false; else -> true } }, key = { it.author + it.id }) { hit ->
                    Column(Modifier.fillMaxWidth().clickable { command("open", mapOf("peer" to hit.peer, "author" to hit.author, "message" to hit.id, "thread_author" to hit.threadTarget?.author, "thread_message" to hit.threadTarget?.id)) }.padding(horizontal = 24.dp, vertical = 12.dp)) {
                        Text(state.chats.find { it.id == hit.peer }?.name ?: "Conversation", style = MaterialTheme.typography.titleMedium)
                        Text(hit.text, maxLines = 3, overflow = TextOverflow.Ellipsis); Text(hit.time, style = MaterialTheme.typography.labelSmall)
                    }
                }
                if (state.searchMore) item { TextButton({ command("search_more", emptyMap()) }, Modifier.fillMaxWidth(), enabled = !state.searching) { Text("More results") } }
            }
        }
    }
}
@Composable
internal fun NotesGrid(state: MessengerState, query: String, read: (String) -> String?, write: (String, String) -> Unit, open: (String) -> Unit, command: Command) {
    var revision by remember { mutableIntStateOf(0) }
    val notes = state.searchHits.filter { it.noted }.groupBy { it.peer }
    val chats = state.chats.filter { (it.id in notes || it.id == "self") && (it.name.contains(query, true) || notes[it.id].orEmpty().any { n -> n.text.contains(query, true) }) }
        .sortedWith(compareByDescending<ChatSummary> { it.id == "self" }.thenByDescending { revision; read("note_pin.${it.id}") == "true" })
    BoxWithConstraints(Modifier.fillMaxSize()) {
        LazyVerticalGrid(GridCells.Fixed(if (maxWidth >= 840.dp) 4 else 2), contentPadding = PaddingValues(16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            items(chats, key = { it.id }) { chat ->
                val pinned = read("note_pin.${chat.id}") == "true"
                Surface(Modifier.animateItem().combinedClickable(onClick = { open(chat.id) }, onLongClick = { write("note_pin.${chat.id}", (!pinned).toString()); revision++ }), shape = RoundedCornerShape(16.dp), border = BorderStroke(0.5.dp, MaterialTheme.colorScheme.outlineVariant)) {
                    Column(Modifier.padding(12.dp).heightIn(min = 150.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                        Row(verticalAlignment = Alignment.CenterVertically) { PresenceAvatar(chat, 28, false); Text(chat.name, Modifier.weight(1f).padding(start = 8.dp), style = MaterialTheme.typography.titleSmall, maxLines = 2); if (pinned) Glyph("push_pin", 16) }
                        Text(notes[chat.id]?.firstOrNull()?.text ?: chat.preview, maxLines = 6, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyMedium)
                    }
                }
            }
            if (state.searchMore) item(span = { GridItemSpan(maxLineSpan) }) { TextButton({ command("search_more", emptyMap()) }, enabled = !state.searching) { Text("More notes") } }
        }
        if (chats.isEmpty()) Text(if (state.searching) "Finding notes…" else "Your conversation notes will appear here.", Modifier.align(Alignment.Center).padding(32.dp))
    }
}
@Composable
internal fun CollectionSheet(state: MessengerState, selected: Set<String>, command: Command, close: () -> Unit) {
    var creating by remember { mutableStateOf(false) }; var name by remember { mutableStateOf("") }; var icon by remember { mutableStateOf("folder") }
    ModalBottomSheet(close, containerColor = MaterialTheme.colorScheme.background) {
        Text("Add to collection", Modifier.padding(horizontal = 24.dp), style = MaterialTheme.typography.headlineSmall)
        if (creating) Column(Modifier.padding(24.dp)) {
            OutlinedTextField(name, { name = it }, label = { Text("Collection name") }, singleLine = true)
            LazyVerticalGrid(GridCells.Fixed(5), Modifier.height(160.dp)) { items(listOf("folder", "group", "work", "home", "menu_book", "favorite", "school", "music_note", "sports_esports", "pets")) { symbol -> IconButton({ icon = symbol }) { Glyph(symbol, 26) } } }
            OutlinedTextField(icon, { icon = it }, label = { Text("Icon or emoji") }, singleLine = true)
            Button({ command("create_collection", mapOf("name" to name.trim(), "icon" to icon, "peers" to selected.toList())); close() }, enabled = name.isNotBlank()) { Text("Create collection") }
        } else {
            LazyVerticalGrid(GridCells.Fixed(3), Modifier.heightIn(max = 320.dp).padding(16.dp)) { items(state.collections) { c -> TextButton({ selected.forEach { command("organize", mapOf("peer" to it, "value" to mapOf("CollectionMember" to mapOf("id" to c.id, "present" to true)))) }; close() }) { Column(horizontalAlignment = Alignment.CenterHorizontally) { Glyph(c.icon, 28); Text(c.name) } } } }
            TextButton({ creating = true }, Modifier.fillMaxWidth().padding(16.dp)) { Text("New collection") }
        }
    }
}
