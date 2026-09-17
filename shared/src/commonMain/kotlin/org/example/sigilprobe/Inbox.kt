@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class, androidx.compose.material3.ExperimentalMaterial3Api::class)
package org.sigil

import androidx.compose.animation.*
import androidx.compose.animation.core.*
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.foundation.lazy.grid.*
import androidx.compose.foundation.lazy.staggeredgrid.LazyVerticalStaggeredGrid
import androidx.compose.foundation.lazy.staggeredgrid.StaggeredGridCells
import androidx.compose.foundation.lazy.staggeredgrid.items
import androidx.compose.foundation.shape.*
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.*
import androidx.compose.ui.focus.*
import androidx.compose.ui.layout.FirstBaseline
import androidx.compose.ui.layout.AlignmentLine
import androidx.compose.ui.layout.layout
import kotlin.math.roundToInt
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.*
import kotlinx.coroutines.delay

@Composable
internal fun mainHeaderHeight() = with(LocalDensity.current) {
    maxOf(68.dp, MaterialTheme.typography.headlineMedium.lineHeight.toDp() + 24.dp)
}

@Composable
internal fun MainHeaderTitle(text: String, modifier: Modifier = Modifier) {
    val style = MaterialTheme.typography.headlineMedium
    // Cap-height ratios of the bundled fonts align titles by visible capitals.
    val capHeight = with(LocalDensity.current) { style.fontSize.toPx() } * if (LocalAppearance.current.font == "Newsreader") .67f else .716f
    Text(text, modifier.layout { measurable, constraints ->
        val title = measurable.measure(constraints)
        val baseline = title[FirstBaseline]
        layout(title.width, title.height) {
            title.placeRelative(0, if (baseline == AlignmentLine.Unspecified) 0 else ((title.height + capHeight) / 2f - baseline).roundToInt())
        }
    }, style = style, maxLines = 1, overflow = TextOverflow.Ellipsis)
}

@Composable
internal fun MainHeader(page: String, goingBack: Boolean, query: String, update: (String) -> Unit, selected: Set<String>, state: MessengerState, command: Command,
    clear: () -> Unit, collections: () -> Unit, search: () -> Unit, create: () -> Unit, back: () -> Unit, newCall: () -> Unit = {}, callDetail: Boolean = false, closeCallDetail: () -> Unit = {}) {
    val motionPolicy = LocalMotion.current
    val height = mainHeaderHeight()
    Box(Modifier.fillMaxWidth().height(height).clipToBounds()) {
        AnimatedContent(if (page in listOf("calls", "notes", "settings")) page else "inbox", transitionSpec = {
            (slideInHorizontally(motionPolicy.enter(MotionQuick, delayMillis = MotionStagger)) { if (goingBack) -it else it } + fadeIn(motionPolicy.enter(MotionQuick, delayMillis = MotionStagger))) togetherWith
                (if (goingBack) slideOutHorizontally(motionPolicy.exit(MotionMillis)) { it } + fadeOut(motionPolicy.exit(MotionExit)) else fadeOut(motionPolicy.exit(MotionExit)))
        }, label = "Main header items") { tab ->
            if (tab == "inbox") InboxHeader(page, height, query, update, selected, state, command, clear, collections, search, create, back)
            else if (tab == "notes") NotesHeader(height, query, update)
            else Row(Modifier.fillMaxWidth().height(height).padding(start = if (tab == "calls" && callDetail) 8.dp else 20.dp, end = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                if (tab == "calls" && callDetail) Symbol("chevron_left", "Back to calls", closeCallDetail)
                MainHeaderTitle(if (tab == "calls") { if (callDetail) "Call details" else "Calls" } else "Settings", Modifier.weight(1f))
                if (tab == "calls" && !callDetail) SigilIconButton(newCall, enabled = !state.busy && state.call == null) { Glyph("add", 24, "New call", filled = false) }
            }
        }
    }
}
@Composable
private fun InboxHeader(page: String, height: Dp, query: String, update: (String) -> Unit, selected: Set<String>, state: MessengerState, command: Command,
    clear: () -> Unit, collections: () -> Unit, search: () -> Unit, create: () -> Unit, back: () -> Unit) {
    val motionPolicy = LocalMotion.current
    val opened = page == "search"
    val progress by animateFloatAsState(if (opened) 1f else 0f, motionPolicy.tween(MotionMillis), label = "Header transformation")
    val focus = remember { FocusRequester() }
    val keyboard = LocalSoftwareKeyboardController.current
    LaunchedEffect(page) { if (page == "search") { delay(motionPolicy.delay(MotionMillis.toLong())); focus.requestFocus(); keyboard?.show() } }
    AnimatedContent(selected.isNotEmpty(), transitionSpec = {
        (slideInHorizontally(motionPolicy.enter(MotionMillis)) { it } + fadeIn(motionPolicy.enter(MotionMillis))) togetherWith (slideOutHorizontally(motionPolicy.exit(MotionMillis)) { -it } + fadeOut(motionPolicy.exit(MotionExit)))
    }, label = "Selection toolbar") { selecting ->
        if (selecting) {
            var menu by remember { mutableStateOf(false) }
            val chats = state.chats.filter { it.id in selected }
            val pinned = chats.isNotEmpty() && chats.all { it.pinned }
            val unread = chats.isNotEmpty() && chats.all { it.unread > 0 }
            fun change(value: Map<String, Any?>) { selected.forEach { command("organize", mapOf("peer" to it, "value" to value)) }; clear() }
            Row(Modifier.fillMaxWidth().height(height).padding(horizontal = 8.dp).testTag("conversation-selection-header"), verticalAlignment = Alignment.CenterVertically) {
                Symbol("close", "Cancel selection", clear)
                Text("${selected.size} selected", Modifier.weight(1f).padding(horizontal = 4.dp), style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                SigilIconButton({ change(mapOf("ConversationPin" to !pinned)) }) { Glyph("push_pin", 24, if (pinned) "Unpin conversations" else "Pin conversations", filled = pinned) }
                Symbol("drive_file_move", "Add to collection", collections)
                Box {
                    Symbol("more_vert", "More conversation actions") { menu = true }
                    DropdownMenu(menu, { menu = false }) {
                        DropdownMenuItem({ Text("Snooze or unsnooze") }, { menu = false; command("snooze_picker", mapOf("peers" to selected.toList())) }, leadingIcon = { Glyph("snooze", 22) })
                        DropdownMenuItem({ Text(if (unread) "Mark read" else "Mark unread") }, {
                            menu = false
                            if (unread) { selected.forEach { command("mark_read", mapOf("peer" to it)) }; clear() }
                            else change(mapOf("Unread" to true))
                        }, leadingIcon = { Glyph(if (unread) "mark_chat_read" else "mark_chat_unread", 22) })
                        if (chats.any { !it.group && it.id != "self" }) DropdownMenuItem({ Text("Block contacts") }, { menu = false; command("block_picker", mapOf("peers" to selected.toList())) }, leadingIcon = { Glyph("block", 22) })
                        DropdownMenuItem({ Text("Delete conversations") }, { menu = false; command("delete_picker", mapOf("peers" to selected.toList())) }, leadingIcon = { Glyph("delete", 22) })
                    }
                }
            }
        } else BoxWithConstraints(Modifier.fillMaxWidth().height(height).padding(horizontal = 12.dp)) {
            MainHeaderTitle("Sigil", Modifier.align(Alignment.CenterStart).padding(start = 8.dp).alpha(1f - progress))
            val x = (maxWidth - 96.dp) * (1f - progress)
            Box(Modifier.offset(x = x).align(Alignment.CenterStart)) {
                Crossfade(opened, animationSpec = motionPolicy.tween(MotionMillis), label = "Search to back") { backIcon ->
                    if (backIcon) Symbol("chevron_left", "Back", back) else Symbol("search", "Search conversations", search)
                }
            }
            AnimatedVisibility(!opened, Modifier.align(Alignment.CenterEnd), enter = fadeIn(motionPolicy.enter(MotionMillis)), exit = fadeOut(motionPolicy.exit(MotionExit))) { SigilIconButton(create) { Glyph("edit_square", 24, "New conversation", filled = false) } }
            AnimatedVisibility(opened, Modifier.align(Alignment.CenterStart).padding(start = 56.dp, end = 8.dp).fillMaxWidth(), enter = fadeIn(motionPolicy.enter(MotionMillis)), exit = fadeOut(motionPolicy.exit(MotionExit))) {
              BasicTextField(query, update, Modifier.fillMaxWidth().focusRequester(focus),
                cursorBrush = androidx.compose.ui.graphics.SolidColor(MaterialTheme.colorScheme.primary), singleLine = true, textStyle = MaterialTheme.typography.titleMedium.copy(color = MaterialTheme.colorScheme.onBackground),
                decorationBox = { inner -> Box { if (query.isEmpty()) Text("Search all conversations", color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.titleMedium); inner() } })
            }
        }
    }
}
@Composable
private fun NotesHeader(height: Dp, query: String, update: (String) -> Unit) {
    var searching by remember { mutableStateOf(false) }
    val focus = remember { FocusRequester() }
    val keyboard = LocalSoftwareKeyboardController.current
    fun closeSearch() { searching = false; update(""); keyboard?.hide() }
    BackAction(searching, ::closeSearch)
    LaunchedEffect(searching) { if (searching) { focus.requestFocus(); keyboard?.show() } }
    Row(Modifier.fillMaxWidth().height(height).padding(start = if (searching) 8.dp else 20.dp, end = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        if (searching) {
            Symbol("chevron_left", "Close notes search", ::closeSearch)
            BasicTextField(query, update, Modifier.weight(1f).focusRequester(focus), singleLine = true,
                textStyle = MaterialTheme.typography.titleMedium.copy(color = MaterialTheme.colorScheme.onSurface),
                cursorBrush = androidx.compose.ui.graphics.SolidColor(MaterialTheme.colorScheme.primary),
                decorationBox = { inner -> Box { if (query.isEmpty()) Text("Search notes", style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onSurfaceVariant); inner() } })
        } else {
            MainHeaderTitle("Notes", Modifier.weight(1f))
            Symbol("search", "Search notes") { searching = true }
        }
    }
}

@Composable
internal fun Inbox(state: MessengerState, collection: String, choose: (String) -> Unit, selected: Set<String>, select: (String) -> Unit,
    open: (String) -> Unit, read: (String) -> String?) {
    val insets = LocalHomeContentPadding.current
    val chats = state.chats.filter { !it.hidden && !it.contactOnly && (!state.collectionsEnabled || collection.isEmpty() || collection in it.collections) }
    val labels = read("collection_labels") != "false"
    LazyColumn(Modifier.fillMaxSize().testTag("inbox-list"), contentPadding = PaddingValues(
        top = insets.calculateTopPadding(), bottom = maxOf(92.dp, insets.calculateBottomPadding()))) {
        if (state.collectionsEnabled) item(key = "collections") {
            LazyRow(Modifier.testTag("inbox-collections"), contentPadding = PaddingValues(horizontal = 16.dp, vertical = 12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                items(listOf(CollectionItem("", "All", "apps")) + state.collections, key = { it.id }) { item ->
                    val active = item.id == collection
                    Column(Modifier.width(72.dp).selectable(active, onClick = { choose(item.id) }, role = Role.Tab)
                        .semantics(mergeDescendants = true) { contentDescription = item.name }.padding(vertical = 4.dp),
                        horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        Surface(Modifier.size(56.dp), shape = RoundedCornerShape(18.dp),
                            color = if (active) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceContainer,
                            contentColor = if (active) MaterialTheme.colorScheme.onPrimaryContainer else MaterialTheme.colorScheme.onSurfaceVariant) {
                            Box(contentAlignment = Alignment.Center) { Glyph(item.icon, 25, filled = active) }
                        }
                        if (labels) Text(item.name, Modifier.fillMaxWidth().clearAndSetSemantics {}, style = MaterialTheme.typography.labelMedium,
                            textAlign = androidx.compose.ui.text.style.TextAlign.Center, minLines = 2, maxLines = 2, overflow = TextOverflow.Ellipsis,
                            color = if (active) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
        }
        items(chats, key = { "chat:${it.id}" }) { chat ->
            ChatRow(chat, chat.id in selected, itemMotion(), { if (selected.isNotEmpty()) select(chat.id) else open(chat.id) }, { select(chat.id) })
        }
        if (chats.isEmpty()) item(key = "empty") {
            Box(Modifier.fillMaxWidth().padding(horizontal = 32.dp, vertical = 64.dp), contentAlignment = Alignment.Center) {
                Text(if (!state.collectionsEnabled || collection.isEmpty()) "No conversations yet" else "No conversations in this collection", style = MaterialTheme.typography.titleLarge)
            }
        }
    }
}

@Composable
internal fun ChatRow(chat: ChatSummary, selected: Boolean = false, modifier: Modifier = Modifier, open: () -> Unit, hold: () -> Unit = {}) {
    val appearance = LocalAppearance.current
    Row(modifier.fillMaxWidth().padding(horizontal = 12.dp).clip(RoundedCornerShape(18.dp)).semantics { this.selected = selected }.background(if (selected) MaterialTheme.colorScheme.primaryContainer else Color.Transparent).heightIn(min = if (appearance.compact) 64.dp else 88.dp).combinedClickable(onClick = open, onLongClick = hold).padding(horizontal = 8.dp, vertical = if (appearance.compact) 6.dp else 16.dp), verticalAlignment = Alignment.CenterVertically) {
        if (selected) Surface(Modifier.size(if (appearance.compact) 48.dp else 56.dp), shape = CircleShape, color = MaterialTheme.colorScheme.surfaceContainerHigh, contentColor = MaterialTheme.colorScheme.onSurface) { Box(contentAlignment = Alignment.Center) { Glyph("check", 26) } }
        else PresenceAvatar(chat, if (appearance.compact) 48 else 56)
        Column(Modifier.weight(1f).padding(horizontal = 12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(chat.name, style = MaterialTheme.typography.titleLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (appearance.previewLines > 0 || chat.typing.isNotEmpty()) Text(if (chat.typing.isNotEmpty()) "Typing…" else chat.preview, maxLines = appearance.previewLines.coerceAtLeast(1), overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Column(horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(4.dp)) {
            if (chat.request == "incoming") RequestChip("Request")
            else if (chat.request in listOf("pending", "sending")) RequestChip("Pending", "schedule")
            Text(chat.time, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                if (chat.unread > 0) Surface(shape = RoundedCornerShape(6.dp), color = MaterialTheme.colorScheme.primary, contentColor = MaterialTheme.colorScheme.onPrimary) { Text(chat.unread.toString(), Modifier.padding(horizontal = 6.dp), style = MaterialTheme.typography.labelMedium) }
                if (chat.snoozed) Glyph("snooze", 17, "Snoozed")
                if (chat.pinned) Glyph("push_pin", 17, "Pinned conversation", filled=true)
            }
        }
    }
}
@Composable
internal fun PresenceAvatar(chat: ChatSummary, size: Int = 48, presence: Boolean = true) {
    Box {
        Avatar(chat.name, size, chat.avatar)
        if (presence && !chat.group && chat.id != "self") Box(Modifier.align(Alignment.BottomEnd).size((size / 4 + 2).dp)
            .background(MaterialTheme.colorScheme.background, CircleShape).padding(2.dp)
            .background(when (chat.presence) { "active" -> Color(0xff4dba50); "away" -> Color(0xffe7ab37); "busy" -> Color(0xffce4545); else -> Color.Gray }, CircleShape)
            .semantics { contentDescription = chat.presence })
    }
}
private val searchCategories = listOf("Unread" to "mark_chat_unread", "Conversations" to "chat", "Requests" to "person_add", "Pinned" to "push_pin", "Images" to "image", "Videos" to "movie", "Places" to "place", "Links" to "link")
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
                        Row { SigilTextButton({ command("group_invitation", mapOf("invitation" to invitation.id, "accept" to false)) }) { Text("Decline") }; SigilButton({ command("group_invitation", mapOf("invitation" to invitation.id, "accept" to true)) }) { Text("Accept") } }
                    }
                }
                if (category in listOf("", "Conversations", "Unread", "Pinned")) items(state.chats.filter { chat -> !chat.hidden && chat.name.contains(query, true) && when(category) { "Unread" -> chat.unread > 0; "Pinned" -> chat.pinned; else -> true } }, key = { "chat:${it.id}" }) { chat -> ChatRow(chat, open = { open(chat.id) }) }
                items(state.searchHits.filter { hit -> when(category) { "Pinned" -> hit.pinned; "Images", "Videos", "Places", "Links" -> hit.kind == category; "Unread" -> state.chats.any { it.id == hit.peer && it.unread > 0 }; "Requests", "Conversations" -> false; else -> true } }, key = { it.author + it.id }) { hit ->
                    Column(Modifier.fillMaxWidth().clickable { command("open", mapOf("peer" to hit.peer, "author" to hit.author, "message" to hit.id, "thread_author" to hit.threadTarget?.author, "thread_message" to hit.threadTarget?.id)) }.padding(horizontal = 24.dp, vertical = 12.dp)) {
                        Text(state.chats.find { it.id == hit.peer }?.name ?: "Conversation", style = MaterialTheme.typography.titleMedium)
                        Text(hit.text, maxLines = 3, overflow = TextOverflow.Ellipsis); Text(hit.time, style = MaterialTheme.typography.labelSmall)
                    }
                }
                if (state.searchMore) item { SigilTextButton({ command("search_more", emptyMap()) }, Modifier.fillMaxWidth(), enabled = !state.searching) { Text("More results") } }
            }
        }
    }
}
@Composable
internal fun NotesGrid(state: MessengerState, query: String, read: (String) -> String?, write: (String, String) -> Unit, open: (String) -> Unit, command: Command) {
    var revision by remember { mutableIntStateOf(0) }
    val notes = state.searchHits.filter { it.noted }.groupBy { it.peer }
    val chats = state.chats.filter { (it.id in notes) && (it.name.contains(query, true) || notes[it.id].orEmpty().any { n -> n.text.contains(query, true) }) }
        .sortedWith(compareByDescending<ChatSummary> { it.id == "self" }.thenByDescending { revision; read("note_pin.${it.id}") == "true" })
    BoxWithConstraints(Modifier.fillMaxSize().imePadding().testTag("notes-grid")) {
        LazyVerticalStaggeredGrid(columns = StaggeredGridCells.Fixed(if (maxWidth >= 840.dp) 4 else if (maxWidth >= 600.dp) 3 else 2), contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = LocalHomeContentPadding.current.calculateTopPadding() + 16.dp, bottom = LocalHomeContentPadding.current.calculateBottomPadding() + 16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalItemSpacing = 12.dp) {
            items(chats, key = { it.id }) { chat ->
                val pinned = read("note_pin.${chat.id}") == "true"
                Surface(itemMotion().combinedClickable(onClick = { open(chat.id) }, onLongClick = { write("note_pin.${chat.id}", (!pinned).toString()); revision++ }), shape = RoundedCornerShape(18.dp), color = MaterialTheme.colorScheme.surfaceContainerHigh) {
                    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        Row(verticalAlignment = Alignment.CenterVertically) { PresenceAvatar(chat, 28, false); Text(chat.name, Modifier.weight(1f).padding(start = 8.dp), style = MaterialTheme.typography.titleSmall, maxLines = 2); if (pinned) Glyph("push_pin", 16, filled=true) }
                        Text(notes[chat.id]?.firstOrNull()?.text.orEmpty(), maxLines = 9, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyMedium)
                    }
                }
            }
            if (state.searchMore) item { SigilTextButton({ command("search_more", emptyMap()) }, enabled = !state.searching) { Text("More notes") } }
        }
        if (chats.isEmpty()) Text(if (state.searching) "Finding notes…" else "No notes yet", Modifier.align(Alignment.Center).padding(32.dp))
    }
}
@Composable
internal fun CollectionSheet(state: MessengerState, selected: Set<String>, command: Command, close: () -> Unit) {
    var creating by remember { mutableStateOf(false) }; var name by remember { mutableStateOf("") }; var icon by remember { mutableStateOf("folder") }
    ModalBottomSheet(close, containerColor = MaterialTheme.colorScheme.background) {
        Text("Add to collection", Modifier.padding(horizontal = 24.dp), style = MaterialTheme.typography.headlineSmall)
        if (creating) Column(Modifier.padding(24.dp)) {
            OutlinedTextField(name, { name = it }, label = { Text("Collection name") }, singleLine = true)
            LazyVerticalGrid(GridCells.Fixed(5), Modifier.height(160.dp)) { items(listOf("folder", "group", "work", "home", "menu_book", "favorite", "school", "music_note", "sports_esports", "pets")) { symbol -> SigilIconButton({ icon = symbol }) { Glyph(symbol, 26) } } }
            OutlinedTextField(icon, { icon = it }, label = { Text("Icon or emoji") }, singleLine = true)
            SigilButton({ command("create_collection", mapOf("name" to name.trim(), "icon" to icon, "peers" to selected.toList())); close() }, enabled = name.isNotBlank()) { Text("Create collection") }
        } else {
            LazyVerticalGrid(GridCells.Fixed(3), Modifier.heightIn(max = 320.dp).padding(16.dp)) { items(state.collections) { c -> SigilTextButton({ selected.forEach { command("organize", mapOf("peer" to it, "value" to mapOf("CollectionMember" to mapOf("id" to c.id, "present" to true)))) }; close() }) { Column(horizontalAlignment = Alignment.CenterHorizontally) { Glyph(c.icon, 28); Text(c.name) } } } }
            SigilTextButton({ creating = true }, Modifier.fillMaxWidth().padding(16.dp)) { Text("New collection") }
        }
    }
}
