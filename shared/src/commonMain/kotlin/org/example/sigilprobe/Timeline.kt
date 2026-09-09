@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
package org.sigil

import androidx.compose.animation.*
import androidx.compose.animation.core.*
import androidx.compose.foundation.*
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.foundation.shape.*
import androidx.compose.foundation.text.input.*
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.*
import androidx.compose.ui.geometry.*
import androidx.compose.ui.graphics.*
import androidx.compose.ui.input.pointer.*
import androidx.compose.ui.layout.*
import androidx.compose.ui.platform.*
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.*
import androidx.compose.ui.window.*
import org.jetbrains.compose.resources.Font
import sigil.shared.generated.resources.*
import kotlinx.coroutines.launch
import kotlin.math.*

@Composable
fun Glyph(name: String, size: Int = 24, label: String? = null, filled: Boolean = true) {
    val glyphSize = with(LocalDensity.current) { size.dp.toSp() }
    Text(name, fontFamily = if (name.any { it.code > 127 }) null else FontFamily(Font(if (filled) Res.font.material_symbols else Res.font.material_symbols_outline)), fontSize = glyphSize, lineHeight = glyphSize, maxLines = 1,
        modifier = Modifier.clearAndSetSemantics { if (label != null) contentDescription = label })
}
internal fun showsReceipt(index: Int, messages: List<ChatMessage>) = index == 0 && messages.firstOrNull()?.mine == true
internal fun showSeparator(message: ChatMessage, older: ChatMessage?) = older == null || message.timestamp - older.timestamp >= 900
internal fun swipeAction(mine: Boolean, horizontal: Float) = if ((horizontal > 0) != mine) "reply" else "thread"

@Composable
internal fun ConversationHeader(chat: ChatSummary, page: String, threaded: Boolean, command: Command, back: () -> Unit, navigate: (String) -> Unit) {
    var menu by remember(chat.id) { mutableStateOf(false) }
    Row(Modifier.fillMaxSize().padding(horizontal = 8.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        Symbol("chevron_left", "Back", back)
        PresenceAvatar(chat, 42)
        Column(Modifier.weight(1f).padding(start = 10.dp)) {
            Text(if (threaded) "Thread" else chat.name, style = MaterialTheme.typography.titleLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (page.isNotEmpty()) Text(page, style = MaterialTheme.typography.labelSmall)
        }
        Symbol("call", "Start audio call") { command("call_start", mapOf("peer" to chat.id, "video" to false)) }
        Symbol("videocam", "Start video call") { command("call_start", mapOf("peer" to chat.id, "video" to true)) }
        Box {
            Symbol("more_vert", "Conversation menu") { menu = true }
            DropdownMenu(menu, { menu = false }) {
                listOf("Search" to "search", "Notes" to "description", "Threads" to "forum", "Pins" to "push_pin", "Chat theme" to "palette", "Settings" to "settings").forEach { (label, icon) ->
                    DropdownMenuItem({ Text(label) }, { menu = false; navigate(label) }, leadingIcon = { Glyph(icon) })
                }
            }
        }
    }
}
@Composable
internal fun ConversationPage(chat: ChatSummary, state: MessengerState, draft: TextFieldState, analyze: (String) -> String, command: Command,
    page: String, gradient: Boolean, thread: ThreadTarget?, setThread: (ThreadTarget?) -> Unit) {
    var reply by remember(chat.id) { mutableStateOf<ChatMessage?>(null) }
    var editing by remember(chat.id) { mutableStateOf<ChatMessage?>(null) }
    var selected by remember(chat.id) { mutableStateOf<Pair<ChatMessage, Rect>?>(null) }
    var returnBounds by remember(chat.id) { mutableStateOf(Rect.Zero) }
    var details by remember(chat.id) { mutableStateOf<Pair<String, String>?>(null) }
    var submitted by remember { mutableStateOf<String?>(null) }
    var localQuery by remember(page) { mutableStateOf("") }
    val scheme = MaterialTheme.colorScheme
    val list = rememberLazyListState()
    val clipboard = LocalClipboardManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    val focus = LocalFocusManager.current
    LaunchedEffect(state.sent) { submitted?.let { if (draft.text.toString() == it) draft.clearText(); submitted = null; reply = null; editing = null } }
    val atLatest by remember { derivedStateOf { list.firstVisibleItemIndex == 0 && list.firstVisibleItemScrollOffset < 80 } }
    LaunchedEffect(state.typing, state.messages.firstOrNull()?.id) { if (atLatest) list.animateScrollToItem(0) }
    LaunchedEffect(chat.id, page, localQuery, thread?.id, thread?.author) {
        if (page == "Search") kotlinx.coroutines.delay(180)
        command("timeline_filter", mapOf("peer" to chat.id, "category" to page.takeIf { it in listOf("Notes", "Pins", "Threads", "Search") }.orEmpty().ifEmpty { "Timeline" },
            "query" to localQuery.takeIf { page == "Search" }, "thread_author" to thread?.author, "thread_message" to thread?.id))
    }
    val threadsOverview = page == "Threads" && thread == null
    val messages = if (threadsOverview) state.messages.filter { it.threadAuthor != null && it.threadMessage != null }.distinctBy { it.threadAuthor to it.threadMessage } else state.messages
    BackAction(thread != null) { setThread(null); if (state.historical) command("latest", emptyMap()) }
    fun respond(message: ChatMessage, threaded: Boolean) {
        if (threaded) { setThread(ThreadTarget(message.threadAuthor ?: message.author, message.threadMessage ?: message.id)); reply = null } else reply = message
        selected = null
    }
    Box(Modifier.fillMaxSize()) {
        if (LocalHeaderInset.current == 0.dp) LocalWallpaper.current(chat.id, Modifier.matchParentSize())
        Column(Modifier.fillMaxSize().then(if (gradient && LocalHeaderInset.current == 0.dp) Modifier.background(Brush.verticalGradient(listOf(scheme.background.copy(alpha = .7f), scheme.primaryContainer.copy(alpha = .7f)))) else Modifier)) {
            val motion = LocalPageMotion.current
            val goingBack = LocalNavigationBack.current
            val headerInset = LocalHeaderInset.current
            val banner = !chat.group && (!chat.verified || chat.request == "incoming")
            val controls = page == "Search" || state.historical
            val timelineMotion = if (motion == null) Modifier else with(motion) { Modifier.animateEnterExit(
                enter = if (goingBack) fadeIn(tween(MotionMillis)) else slideInVertically(tween(MotionMillis)) { it }, exit = slideOutVertically(tween(MotionMillis)) { it }) }
            Box(Modifier.weight(1f).fillMaxWidth().behindFooter(LocalFooterCover.current)) {
            Column(Modifier.fillMaxSize().then(timelineMotion).testTag("timeline-body")) {
            if (controls) Spacer(Modifier.height(headerInset))
                if (page == "Search") OutlinedTextField(localQuery, { localQuery = it }, Modifier.fillMaxWidth().padding(12.dp), placeholder = { Text("Search this conversation") }, singleLine = true)
            if (state.historical) SigilTextButton({ command("latest", emptyMap()) }, Modifier.align(Alignment.CenterHorizontally)) { Text("Return to latest messages") }
            LazyColumn(Modifier.weight(1f).fillMaxWidth().testTag("timeline"), state = list, reverseLayout = true, contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = (if (controls) 0.dp else headerInset) + 12.dp, bottom = 12.dp)) {
                item("typing") { AnimatedVisibility(!threadsOverview && state.typing.isNotEmpty(), enter = expandVertically(tween(MotionMillis)) + fadeIn(), exit = shrinkVertically(tween(MotionMillis)) + fadeOut()) { TypingRow(state.typing.map { state.people[it] ?: if (chat.group) "Member" else chat.name }, chat.name, state.typing) } }
                itemsIndexed(messages, key = { _, it -> it.author + it.id }) { index, message ->
                    if (threadsOverview) {
                        Surface(Modifier.fillMaxWidth().padding(vertical = 6.dp).clip(RoundedCornerShape(20.dp)).clickable { setThread(ThreadTarget(message.threadAuthor!!, message.threadMessage!!)) }, shape = RoundedCornerShape(20.dp), color = scheme.surfaceVariant) {
                            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                                Text(message.threadPreview ?: "Earlier message", maxLines = 3, overflow = TextOverflow.Ellipsis)
                                Row(verticalAlignment = Alignment.CenterVertically) { Glyph("forum", 18); Spacer(Modifier.width(8.dp)); Text(message.text, Modifier.weight(1f), maxLines = 2, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodySmall); Glyph("chevron_right", 20) }
                            }
                        }
                        return@itemsIndexed
                    }
                    val older = messages.getOrNull(index + 1)
                    val newer = messages.getOrNull(index - 1)
                    val grouped = older?.author == message.author && !showSeparator(message, older)
                    var bounds by remember { mutableStateOf(Rect.Zero) }
                    var drag by remember { mutableFloatStateOf(0f) }
                    val offset by animateFloatAsState(drag, tween(90), label = "Reply swipe")
                    val density = LocalDensity.current
                    Column(Modifier.fillMaxWidth().animateItem().padding(top = if (grouped) 3.dp else 12.dp)) {
                        if (showSeparator(message, older)) Text(message.separator.ifEmpty { message.time }, Modifier.align(Alignment.CenterHorizontally).padding(top = 6.dp, bottom = 14.dp), style = MaterialTheme.typography.labelMedium, color = scheme.onSurfaceVariant)
                        if (chat.group && !message.mine && !grouped) Row(Modifier.padding(bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) { val name = state.people[message.author] ?: "Former member"; Avatar(name, 20, message.author); Text(name, Modifier.padding(start = 6.dp), style = MaterialTheme.typography.bodySmall) }
                        Row(Modifier.fillMaxWidth().combinedClickable(interactionSource = remember { androidx.compose.foundation.interaction.MutableInteractionSource() }, indication = null, onClick = { details = message.author to message.id }, onLongClick = { selected = message to bounds })
                            .pointerInput(message.id, message.mine) { detectHorizontalDragGestures(onDragEnd = {
                                if (abs(drag) > with(density) { 52.dp.toPx() }) respond(message, swipeAction(message.mine, drag) == "thread")
                                drag = 0f
                            }, onDragCancel = { drag = 0f }) { change, amount -> change.consume(); val limit = with(density) { 110.dp.toPx() }; drag = (drag + amount).coerceIn(-limit, limit) } },
                            horizontalArrangement = if (message.mine) Arrangement.End else Arrangement.Start) {
                            Column(Modifier.widthIn(max = 330.dp).fillMaxWidth(.88f), horizontalAlignment = if (message.mine) Alignment.End else Alignment.Start) {
                                val lifted = selected?.first?.let { it.id == message.id && it.author == message.author } == true
                                Box(Modifier.graphicsLayer { translationX = offset; alpha = if (lifted) 0f else 1f }.then(if (lifted) Modifier.clearAndSetSemantics { } else Modifier).onGloballyPositioned { bounds = it.boundsInWindow(); if (lifted) returnBounds = bounds }
                                    .pointerInput(message.id) { awaitPointerEventScope { while (true) { val event = awaitPointerEvent(); if (event.type == PointerEventType.Press && event.buttons.isSecondaryPressed) selected = message to bounds } } }) {
                                    MessageBubble(message, grouped, newer?.author == message.author, analyze, if (chat.verified && !state.busy) command else null)
                                }
                                MessageDetails(message, details == (message.author to message.id), !state.historical && page.isEmpty() && thread == null && showsReceipt(index, messages), chat, state.people)
                            }
                        }
                    }
                    if (!message.mine && !message.readByMe && chat.verified && selected == null) LaunchedEffect(message.id) {
                        command("read", mapOf("peer" to chat.id, "author" to message.author, "message" to message.id))
                    }
                }
                if (state.more) item { SigilTextButton({ command("older", emptyMap()) }, Modifier.fillMaxWidth(), enabled = !state.busy) { Text("Earlier messages") } }
            }
            if (banner) ContactRequestPanel(chat, state.busy, command)
            }
            }
            val composerMotion = if (motion == null) Modifier else with(motion) { Modifier.animateEnterExit(
                enter = if (goingBack) fadeIn(tween(MotionMillis)) else slideInHorizontally(tween(160, delayMillis = 80)) { it } + fadeIn(tween(160, delayMillis = 80)),
                exit = slideOutHorizontally(tween(MotionMillis)) { it } + fadeOut(tween(160))) }
            Column(Modifier.fillMaxWidth().then(composerMotion)) {
            val context = editing?.let { "Editing: ${it.text}" } ?: reply?.let { "Replying to ${it.text}" } ?: thread?.let { "Reply in thread" }
            state.transfers.filter { it.peer == chat.id }.forEach { transfer ->
                Row(Modifier.fillMaxWidth().padding(start = 20.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) { Text(transfer.name, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodySmall); Text(if (transfer.phase == "Staging") "Importing…" else "Sending attachment…", style = MaterialTheme.typography.labelSmall) }
                    Symbol("close", "Cancel attachment") { command("file_cancel", mapOf("request" to transfer.request)) }
                }
            }
            context?.let { Row(Modifier.fillMaxWidth().padding(start = 20.dp), verticalAlignment = Alignment.CenterVertically) { Text(it, Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodySmall); Symbol("close", "Cancel reply or edit") { reply = null; editing = null; setThread(null) } } }
            val inputCommand: Command = { action, fields ->
                command(action, if (action in listOf("attachment_pick", "record_start")) fields + mapOf("reply_author" to reply?.author, "reply_message" to reply?.id, "thread_author" to thread?.author, "thread_message" to thread?.id) else fields)
            }
            if (!threadsOverview) ComposerPanel(draft, analyze, chat.verified && !state.busy, page == "Notes", inputCommand, chat.id, state.voice, state.sent, state.sentText, requestContact = if (!chat.verified && !chat.group && !state.busy && chat.request in listOf("none", "expired")) ({ command("contact_request", mapOf("peer" to chat.id, "action" to "send")) }) else null) { text, rich ->
                submitted = draft.text.toString()
                if (editing != null) command("edit", mapOf("peer" to chat.id, "author" to editing!!.author, "message" to editing!!.id, "text" to text))
                else command("post", mapOf("peer" to chat.id, "text" to text, "rich" to rich,
                    "reply_author" to reply?.author, "reply_message" to reply?.id, "thread_author" to thread?.author, "thread_message" to thread?.id))
            }
        }
        }
        selected?.let { (message, bounds) ->
            val index = messages.indexOfFirst { it.id == message.id && it.author == message.author }
            val older = messages.getOrNull(index + 1)
            MessageMenu(message, bounds, returnBounds.takeIf { it != Rect.Zero } ?: bounds, older?.author == message.author && !showSeparator(message, older), messages.getOrNull(index - 1)?.author == message.author, analyze, { selected = null; returnBounds = Rect.Zero }) { action, value ->
                when (action) {
                    "reply" -> respond(message, false)
                    "thread" -> respond(message, true)
                    "copy" -> { clipboard.setText(AnnotatedString(message.text)); selected = null }
                    "edit" -> { editing = message; draft.edit { replace(0, length, message.text) }; selected = null }
                    "forward" -> { command("forward_picker", mapOf("peer" to chat.id, "author" to message.author, "message" to message.id)); selected = null }
                    else -> {
                        val fields = mutableMapOf<String, Any?>("peer" to chat.id, "author" to message.author, "message" to message.id)
                        when (action) { "pin" -> fields["active"] = !message.pinned; "note" -> fields["active"] = !message.noted; "react" -> { fields["emoji"] = value; fields["active"] = value !in message.myReactions } }
                        command(action, fields); selected = null
                    }
                }
            }
            LaunchedEffect(message.id) { focus.clearFocus(); keyboard?.hide() }
        }
    }
}
@Composable
internal fun MessageBubble(message: ChatMessage, grouped: Boolean, followed: Boolean, analyze: (String) -> String, command: Command? = null) {
    val scheme = MaterialTheme.colorScheme
    val emoji = remember(message.text, message.kind, message.reply, message.parts) { if (message.kind == "Text" && message.reply == null && message.parts.all { it.kind == "text" && it.rich?.spans.orEmpty().isEmpty() }) animatedEmoji(message.text) else null }
    Box(Modifier.padding(top = if (message.reactions.isNotEmpty() || message.pinned) 8.dp else 0.dp)) {
        if (emoji != null) EmojiMessage(emoji)
        else
        Surface(shape = RoundedCornerShape(topStart = if (!message.mine && grouped) 5.dp else 20.dp, topEnd = if (message.mine && grouped) 5.dp else 20.dp,
            bottomStart = if (!message.mine && followed) 5.dp else 20.dp, bottomEnd = if (message.mine && followed) 5.dp else 20.dp),
            color = if (message.mine) scheme.primary else scheme.surfaceVariant, contentColor = if (message.mine) scheme.onPrimary else scheme.onSurfaceVariant) {
            CompositionLocalProvider(LocalMessageSurface provides if (message.mine) scheme.primary else scheme.surfaceVariant) {
            Column(Modifier.padding(horizontal = 14.dp, vertical = 10.dp)) {
                message.reply?.let { Surface(shape = RoundedCornerShape(12.dp), color = (if (message.mine) scheme.onPrimary else scheme.onSurface).copy(alpha = .09f)) { Text(it, Modifier.padding(9.dp), style = MaterialTheme.typography.bodySmall, maxLines = 2, overflow = TextOverflow.Ellipsis) }; Spacer(Modifier.height(6.dp)) }
                if (message.attachment != null) LocalAttachmentContent.current(message) else if (message.parts.isNotEmpty()) MessageCards(message, analyze, command) else MessageText(message.text, analyze)
            }
            }
        }
        if (message.reactions.isNotEmpty()) Text(message.reactions.distinct().joinToString(""), Modifier.align(if (message.mine) Alignment.TopStart else Alignment.TopEnd).offset(y = (-10).dp), fontSize = 20.sp)
        if (message.pinned) Box(Modifier.align(if (message.mine) Alignment.TopEnd else Alignment.TopStart).offset(y = (-8).dp).background(scheme.background, CircleShape).padding(2.dp)) { Glyph("push_pin", 13, "Pinned message") }
    }
}
@Composable
internal fun MessageDetails(message: ChatMessage, expanded: Boolean, receipt: Boolean, chat: ChatSummary, people: Map<String, String>) {
    Row(Modifier.animateContentSize(tween(MotionMillis)).padding(top = if (expanded || receipt) 4.dp else 0.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(5.dp)) {
        if (receipt) DeliveryReceipt(message, chat, people)
        AnimatedVisibility(expanded, enter = fadeIn(tween(180)) + expandHorizontally(tween(MotionMillis), expandFrom = Alignment.End), exit = fadeOut(tween(100)) + shrinkHorizontally(tween(MotionMillis))) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                if (receipt) Text("·", style = MaterialTheme.typography.labelSmall)
                Text(message.time, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Glyph("lock", 11, "Encrypted message")
            }
        }
    }
}
@Composable
private fun DeliveryReceipt(message: ChatMessage, chat: ChatSummary, people: Map<String, String>) {
    if (message.delivery == "Read") AvatarStack(message.readers.map { people[it] ?: if (chat.group) "Member" else chat.name }.ifEmpty { listOf(chat.name) }, 17, message.readers.ifEmpty { listOf(chat.avatar) })
    else if (message.delivery in listOf("Queued", "Sending")) {
        val transition = rememberInfiniteTransition(label = "Sending")
        val angle by transition.animateFloat(0f, 360f, infiniteRepeatable(tween(900, easing = LinearEasing)), label = "Sending dots")
        val color = MaterialTheme.colorScheme.onSurfaceVariant
        Canvas(Modifier.size(17.dp).semantics { contentDescription = "Sending" }) {
            repeat(8) { index -> val r = (index * 45f + angle) * PI / 180; drawCircle(color, 1.dp.toPx(), Offset(center.x + cos(r).toFloat() * size.width * .36f, center.y + sin(r).toFloat() * size.height * .36f)) }
        }
    } else Surface(Modifier.size(17.dp).semantics { contentDescription = message.delivery }, shape = CircleShape,
        color = if (message.delivery in listOf("Failed", "Expired", "Cancelled")) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
        contentColor = MaterialTheme.colorScheme.background) { Box(contentAlignment = Alignment.Center) { Glyph(if (message.delivery in listOf("Failed", "Expired", "Cancelled")) "priority_high" else "check", 12) } }
}
@Composable
internal fun AvatarStack(people: List<String>, size: Int = 22, photos: List<String> = emptyList()) {
    Box(Modifier.width((size + (people.take(5).size - 1).coerceAtLeast(0) * size * .7f).dp).height(size.dp)) {
        people.take(5).forEachIndexed { i, name -> Box(Modifier.offset(x = (i * size * .7f).dp).border(1.dp, MaterialTheme.colorScheme.background, CircleShape)) { Avatar(name, size, photos.getOrElse(i) { "" }) } }
    }
}
@Composable
private fun TypingRow(people: List<String>, name: String, photos: List<String>) {
    val animation = rememberInfiniteTransition(label = "Typing")
    val phase by animation.animateFloat(0f, 2f * PI.toFloat(), infiniteRepeatable(tween(1000, easing = LinearEasing)), label = "Typing dots")
    Row(Modifier.padding(top = 8.dp, bottom = 4.dp).semantics { contentDescription = "$name is typing" }, verticalAlignment = Alignment.CenterVertically) {
        AvatarStack(people, 22, photos); Spacer(Modifier.width(10.dp))
        repeat(3) { i -> Box(Modifier.padding(horizontal = 3.dp).offset(y = (-3 * max(0f, sin(phase - i * .8f))).dp).size(5.dp).background(MaterialTheme.colorScheme.onSurfaceVariant, CircleShape)) }
    }
}
@Composable
private fun MessageMenu(message: ChatMessage, origin: Rect, returnTo: Rect, grouped: Boolean, followed: Boolean, analyze: (String) -> String, dismiss: () -> Unit, action: (String, String) -> Unit) {
    var target by remember { mutableStateOf(Rect.Zero) }
    val progress = remember { Animatable(0f) }
    var emojiPicker by remember { mutableStateOf(false) }
    var emoji by remember { mutableStateOf("") }
    var closing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val density = LocalDensity.current
    fun finish(after: () -> Unit = {}) {
        if (closing) return
        closing = true
        scope.launch { progress.animateTo(0f, tween(MotionMillis)); dismiss(); after() }
    }
    fun choose(name: String, value: String) { finish { action(name, value) } }
    LaunchedEffect(Unit) { progress.animateTo(1f, tween(MotionMillis)) }
    Dialog({ finish() }, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.scrim.copy(alpha = .48f * progress.value)).clickable { finish() }.safeDrawingPadding(), contentAlignment = Alignment.Center) {
            Column(Modifier.widthIn(max = 360.dp).fillMaxWidth().padding(16.dp).verticalScroll(rememberScrollState()), horizontalAlignment = if (message.mine) Alignment.End else Alignment.Start, verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Surface(Modifier.alpha(progress.value), shape = RoundedCornerShape(28.dp), color = MaterialTheme.colorScheme.surfaceVariant) {
                    Row(Modifier.padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                        listOf("👍", "❤️", "😂", "😮", "😢", "😡").forEach { e -> SigilTextButton({ choose("react", e) }, Modifier.weight(1f), contentPadding = PaddingValues(0.dp)) { Text(e, fontSize = 22.sp) } }
                        Symbol("add_reaction", "Choose reaction") { emojiPicker = true }
                    }
                }
                Box(Modifier.width(with(density) { origin.width.toDp() }).onGloballyPositioned { target = it.boundsInWindow() }) {
                    Box(Modifier.graphicsLayer {
                        val source = if (closing) returnTo else origin
                        if (target != Rect.Zero && source != Rect.Zero) { translationX = (source.left - target.left) * (1f - progress.value); translationY = (source.top - target.top) * (1f - progress.value) }
                    }) { MessageBubble(message, grouped, followed, analyze) }
                }
                Surface(Modifier.width(232.dp).alpha(progress.value), shape = RoundedCornerShape(20.dp), color = MaterialTheme.colorScheme.surfaceVariant) {
                    Column(Modifier.padding(vertical = 6.dp)) {
                        val entries = listOf("reply" to "Reply", "forward" to "Forward", "copy" to "Copy", "thread" to "Reply in thread", "pin" to if (message.pinned) "Unpin" else "Pin", "note" to if (message.noted) "Remove from notes" else "Add to notes") +
                            (if (message.mine && message.editable) listOf("edit" to "Edit") else emptyList()) + (if (message.mine) listOf("delete" to "Delete") else emptyList())
                        entries.forEach { (key, label) -> DropdownMenuItem({ Text(label, color = if (key == "delete") MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface) }, { choose(key, "") },
                            leadingIcon = { Glyph(when(key) { "thread" -> "forum"; "pin" -> "push_pin"; "note" -> "description"; "copy" -> "content_copy"; else -> key }, 20) }) }
                    }
                }
            }
        }
    }
    if (emojiPicker) AlertDialog({ emojiPicker = false }, title = { Text("React with an emoji") }, text = { OutlinedTextField(emoji, { emoji = it }, singleLine = true) }, confirmButton = { SigilTextButton({ if (emoji.isNotBlank()) { emojiPicker = false; choose("react", emoji) } }) { Text("React") } })
}
@Composable
internal fun VerificationDialog(chat: ChatSummary, busy: Boolean, command: Command, close: () -> Unit) {
    AlertDialog(close, title = { Text("Verify ${chat.name}'s devices") }, text = {
        Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Text("Optional: compare fingerprints in person or through another trusted channel. You can already chat after accepting a request.")
            chat.devices.forEach { device ->
                SelectionContainer { Text(device.fingerprint.chunked(4).joinToString(" "), fontFamily = LocalCodeFont.current, style = MaterialTheme.typography.bodySmall) }
                if (device.changed || device.blocked) Text("This device changed or is blocked. Review its identity before using it.")
                else SigilTextButton({ command("confirm", mapOf("peer" to device.id, "fingerprint" to device.fingerprint)) }, enabled = !busy) { Text(if (device.verified) "Fingerprint verified" else "Fingerprints match · approve") }
            }
        }
    }, confirmButton = { SigilTextButton(close) { Text("Done") } })
}
