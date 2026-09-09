package org.sigil

import androidx.compose.animation.*
import androidx.compose.animation.core.*
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.input.key.*
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp

internal const val MotionMillis = 240
internal typealias Command = (String, Map<String, Any?>) -> Unit
private val LocalBackActions = staticCompositionLocalOf<androidx.compose.runtime.snapshots.SnapshotStateList<() -> Unit>?> { null }
@Composable
internal fun BackAction(enabled: Boolean, action: () -> Unit) {
    val actions = LocalBackActions.current
    val current by rememberUpdatedState(action)
    DisposableEffect(actions, enabled) {
        val callback = { current() }
        if (enabled) actions?.add(callback)
        onDispose { actions?.remove(callback) }
    }
}

@Composable
fun SigilApp(palette: (Int, Boolean) -> String, analyze: (String) -> String, state: MessengerState, command: Command,
    read: (String) -> String? = { null }, write: (String, String) -> Unit = { _, _ -> }, dynamicAccent: Int? = null,
    onBackAvailable: (Boolean, () -> Unit) -> Unit = { _, _ -> }, overlay: @Composable () -> Unit = {}) {
    var followAccount by remember { mutableStateOf(read("follow_account_theme") != "false") }
    var pendingAppearance by remember { mutableStateOf<String?>(null) }
    var appearance by remember { mutableStateOf(decodeAppearance(if (followAccount) state.ui["appearance"] ?: read("account_appearance") ?: read("appearance") else read("device_appearance") ?: read("appearance"))) }
    val sharedRead: (String) -> String? = { key ->
        if (key.startsWith("note_pin.")) state.chats.find { it.id == key.removePrefix("note_pin.") }?.ui?.get("notes_pinned") ?: read(key)
        else state.ui[key] ?: read(if (key == "appearance") "account_appearance" else key) ?: read(key)
    }
    val sharedWrite: (String, String) -> Unit = { key, value ->
        write(if (key == "appearance") "account_appearance" else key, value)
        val peer = if (key.startsWith("note_pin.")) key.removePrefix("note_pin.") else null
        command("organize", mapOf("peer" to peer, "value" to mapOf("UiSetting" to mapOf("key" to if (peer != null) "notes_pinned" else key, "value" to value))))
    }
    LaunchedEffect(state.ui["appearance"], followAccount) {
        val remote = sharedRead("appearance")
        if (!followAccount) appearance = decodeAppearance(read("device_appearance") ?: read("appearance"))
        else if (pendingAppearance == null || remote == pendingAppearance) { appearance = decodeAppearance(remote); pendingAppearance = null }
    }
    var page by rememberSaveable { mutableStateOf("inbox") }
    var goingBack by remember { mutableStateOf(false) }
    var callMinimized by remember(state.call?.call?.id) { mutableStateOf(false) }
    var chatTheme by remember { mutableStateOf(ChatTheme()) }
    var pendingChatTheme by remember { mutableStateOf<Pair<String, String>?>(null) }
    var conversationPage by rememberSaveable { mutableStateOf("") }
    var selected by remember { mutableStateOf(setOf<String>()) }
    var query by rememberSaveable { mutableStateOf("") }
    var category by rememberSaveable { mutableStateOf("") }
    var collection by rememberSaveable { mutableStateOf("") }
    var collectionSheet by remember { mutableStateOf(false) }
    var actionSheet by remember { mutableStateOf("") }
    var actionFields by remember { mutableStateOf(emptyMap<String, Any?>()) }
    val dispatch: Command = { name, fields ->
        if (name == "history_open") page = "history"
        else if (name in listOf("snooze_picker", "forward_picker", "block_picker", "delete_picker")) { actionSheet = name; actionFields = fields }
        else command(name, fields)
    }
    val drafts = remember { mutableMapOf<String, TextFieldState>() }
    val chat = state.chats.find { it.id == state.selected }
    var thread by remember(chat?.id) { mutableStateOf(state.threadTarget) }
    val focus = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    val backActions = remember { mutableStateListOf<() -> Unit>() }
    val navigate: (String) -> Unit = { target ->
        goingBack = false; focus.clearFocus(); keyboard?.hide(); selected = emptySet(); page = target; query = ""; category = ""
    }
    val back: () -> Unit = {
        when {
            state.call != null && !callMinimized -> callMinimized = true
            backActions.isNotEmpty() -> backActions.last()()
            selected.isNotEmpty() -> selected = emptySet()
            conversationPage.isNotEmpty() -> conversationPage = ""
            page in listOf("appearance", "device", "profile", "privacy", "notifications", "storage", "about") -> navigate("settings")
            chat != null -> { command("close", emptyMap()); conversationPage = "" }
            page == "history" -> navigate("storage")
            page != "inbox" -> navigate("inbox")
        }
        goingBack = true
    }
    SideEffect { onBackAvailable(state.call != null || backActions.isNotEmpty() || selected.isNotEmpty() || page != "inbox" || chat != null, back) }
    LaunchedEffect(chat?.id, chat?.ui?.get("chat_theme")) {
        val remote = chat?.ui?.get("chat_theme") ?: read("chat.${chat?.id}")
        if (pendingChatTheme?.first != chat?.id || remote == pendingChatTheme?.second) { chatTheme = decodeChat(remote); pendingChatTheme = null }
    }
    LaunchedEffect(page, query, category, conversationPage, state.sent) {
        if (chat == null && page in listOf("search", "notes")) {
            kotlinx.coroutines.delay(180)
            command("search", mapOf("query" to if (page == "notes") "" else query, "category" to if (page == "notes") "Notes" else category))
        }
    }
    val open: (String) -> Unit = { peer -> goingBack = false; focus.clearFocus(); keyboard?.hide(); command("open", mapOf("peer" to peer)) }
    SigilTheme(appearance, if (chat != null) chatTheme else null, dynamicAccent, palette) {
      CompositionLocalProvider(LocalBackActions provides backActions) {
        val mainHeader = chat == null && (state.call == null || callMinimized) && page in listOf("inbox", "calls", "settings", "search", "notes")
        Surface(color = if (mainHeader) MaterialTheme.colorScheme.background else MaterialTheme.colorScheme.surface) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.BottomCenter) {
                Spacer(Modifier.fillMaxWidth().windowInsetsBottomHeight(WindowInsets.navigationBars).background(MaterialTheme.colorScheme.surface))
            }
            Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.systemBars.union(WindowInsets.displayCutout)).onPreviewKeyEvent {
                if (it.type == KeyEventType.KeyDown && it.key == Key.Escape) { back(); true } else false
            }) {
                if (state.issue != null) Row(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surfaceVariant).padding(start = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(state.issue, Modifier.weight(1f), style = MaterialTheme.typography.bodySmall)
                    Symbol("close", "Dismiss notice") { command("dismiss", emptyMap()) }
                }
                when {
                    state.phase == "loading" -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
                    state.phase == "unavailable" -> Box(Modifier.fillMaxSize().padding(32.dp), contentAlignment = Alignment.Center) { Text("Connected messaging is currently available in the Android development build.") }
                    state.phase != "connected" -> Box(Modifier.imePadding()) { SignIn(state, command) }
                    else -> {
                        if (state.accountAccess?.let { it.linked && it.retiring && !it.acknowledged } == true && page != "profile" && state.call == null) TextButton({ command("close", emptyMap()); conversationPage = ""; navigate("profile") }, Modifier.fillMaxWidth()) { Text("Your server’s sign-in is changing · Review") }
                        val destination = when { state.call != null && !callMinimized -> "call"; chat?.archived == true -> "saved-conversation"; chat != null -> when (conversationPage) { "Chat theme" -> "theme"; "Settings" -> "chat-settings"; else -> "conversation" }; page in listOf("inbox", "search", "notes", "calls", "settings") -> "home"; else -> page }
                        if (state.call != null && callMinimized) TextButton({ callMinimized = false }, Modifier.fillMaxWidth()) { Glyph("call", 18); Spacer(Modifier.width(8.dp)); Text("Return to call") }
                        if (destination in listOf("home", "conversation")) {
                            val radius by animateDpAsState(if (chat == null) 0.dp else 24.dp, tween(MotionMillis), label = "Header corners")
                            val color by animateColorAsState(if (chat == null) MaterialTheme.colorScheme.background else MaterialTheme.colorScheme.surface, tween(MotionMillis), label = "Header surface")
                            val shape = RoundedCornerShape(bottomStart = radius, bottomEnd = radius)
                            val height = maxOf(76.dp, with(LocalDensity.current) { MaterialTheme.typography.displaySmall.lineHeight.toDp() } + 24.dp)
                            Surface(Modifier.fillMaxWidth().height(height).testTag("main-header").then(if (chat != null) Modifier.headerShadow(shape) else Modifier), shape = shape, color = color) {
                                AnimatedContent(chat, contentKey = { it?.id }, transitionSpec = {
                                    (slideInHorizontally(tween(160, delayMillis = 80)) { if (goingBack) -it else it } + fadeIn(tween(160, delayMillis = 80))) togetherWith fadeOut(tween(100))
                                }, label = "Header items") { current ->
                                    if (current == null) MainHeader(page, goingBack, query, { query = it }, selected, state, dispatch, { selected = emptySet() },
                                        { collectionSheet = true }, { navigate("search") }, { navigate("notes") }, back)
                                    else ConversationHeader(current, conversationPage, thread != null, dispatch, back) { goingBack = false; conversationPage = it }
                                }
                            }
                        }
                        AnimatedContent(destination, Modifier.weight(1f).background(MaterialTheme.colorScheme.background), transitionSpec = {
                            (fadeIn(tween(MotionMillis)) + slideInHorizontally(tween(MotionMillis)) { if (goingBack) -it else it }) togetherWith
                                (slideOutHorizontally(tween(MotionMillis)) { if (goingBack) it else -it } + fadeOut(tween(120)))
                        }, label = "Page") { target ->
                            when (target) {
                                "call" -> state.call?.let { CallPage(it, state.chats, dispatch, state.profileAvatar) { callMinimized = true } }
                                "conversation" -> chat?.let { ConversationPage(it, state, drafts.getOrPut(it.id) { TextFieldState(it.draft) }, analyze, dispatch, conversationPage, chatTheme.gradient, thread, { thread = it }) }
                                "theme" -> chat?.let { current -> ChatAppearance(chatTheme, analyze, current.id, command, { goingBack = true; conversationPage = "" }) { chatTheme = it; pendingChatTheme = current.id to it.encode(); write("chat.${current.id}", it.encode()); command("organize", mapOf("peer" to current.id, "value" to mapOf("UiSetting" to mapOf("key" to "chat_theme", "value" to it.encode())))) } }
                                "chat-settings" -> chat?.let { ConversationSettings(it, state.busy, dispatch, back) }
                                "appearance" -> AppearancePage(appearance, analyze, dynamicAccent != null, back, state.collectionsEnabled,
                                    { enabled -> command("organize", mapOf("peer" to null, "value" to mapOf("CollectionsEnabled" to enabled))) },
                                    sharedRead("collection_labels") != "false", { sharedWrite("collection_labels", it.toString()) }, followAccount,
                                    { if (!it) write("device_appearance", appearance.encode()); followAccount = it; write("follow_account_theme", it.toString()) }) { appearance = it; if (followAccount) { pendingAppearance = it.encode(); sharedWrite("appearance", it.encode()) } else write("device_appearance", it.encode()) }
                                "device", "profile", "privacy", "notifications", "storage", "about" -> PersonalPage(target, state, dispatch, back)
                                "history" -> SavedHistoryPage(state, command, back, open)
                                "saved-conversation" -> SavedConversationPage(state, analyze, command, back)
                                "new" -> NewConversation(state, command, back, open)
                                else -> Column(Modifier.fillMaxSize()) {
                                    Box(Modifier.weight(1f)) {
                                      AnimatedContent(page, transitionSpec = {
                                          (slideInHorizontally(tween(MotionMillis)) { if (goingBack) -it else it } + fadeIn(tween(MotionMillis))) togetherWith
                                              (slideOutHorizontally(tween(MotionMillis)) { if (goingBack) it else -it } + fadeOut(tween(120)))
                                      }, label = "Inbox panel") { panel ->
                                        when (panel) {
                                            "settings" -> SettingsPage(state, navigate)
                                            "calls" -> CallHistoryPage(state, command)
                                            "search" -> SearchPage(state, query, category, { category = it }, open, dispatch)
                                            "notes" -> NotesGrid(state, query, sharedRead, sharedWrite, { peer -> open(peer); conversationPage = "Notes" }, dispatch)
                                            else -> Inbox(state, collection, { collection = it }, selected, { id -> selected = if (id in selected) selected - id else selected + id }, open, sharedRead)
                                        }
                                      }
                                      InboxFab(page == "inbox", Modifier.align(Alignment.BottomEnd)) { navigate("new") }
                                    }
                                }
                            }
                        }
                        AnimatedVisibility(chat == null && page in listOf("inbox", "calls", "settings") && (state.call == null || callMinimized), enter = slideInVertically(tween(MotionMillis)) { it } + fadeIn(), exit = slideOutVertically(tween(MotionMillis)) { it } + fadeOut()) {
                            Surface(Modifier.footerShadow().testTag("main-navigation"), shape = RoundedCornerShape(topStart = 24.dp, topEnd = 24.dp)) {
                                Row(Modifier.fillMaxWidth().padding(vertical = 8.dp), horizontalArrangement = Arrangement.SpaceEvenly) {
                                    listOf(Triple("inbox", "chat_bubble", "Messages"), Triple("calls", "call", "Calls"), Triple("settings", "settings", "Settings")).forEach { (tab, icon, label) ->
                                        Surface(shape = RoundedCornerShape(16.dp), color = if (page == tab) MaterialTheme.colorScheme.inverseSurface else MaterialTheme.colorScheme.surface,
                                            contentColor = if (page == tab) MaterialTheme.colorScheme.inverseOnSurface else MaterialTheme.colorScheme.onSurfaceVariant) { Symbol(icon, label) { navigate(tab) } }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if (collectionSheet) CollectionSheet(state, selected, command, { collectionSheet = false; selected = emptySet() })
            if (actionSheet.isNotEmpty()) ConversationActionSheet(actionSheet, actionFields, state, command) { actionSheet = ""; selected = emptySet() }
        }
        overlay()
      }
    }
}
