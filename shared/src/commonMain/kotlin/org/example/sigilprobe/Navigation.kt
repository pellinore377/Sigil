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
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.unit.dp

internal const val MotionMillis = 240
internal val LocalPageMotion = staticCompositionLocalOf<AnimatedVisibilityScope?> { null }
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
    LaunchedEffect(state.ui["appearance"], followAccount) { appearance = decodeAppearance(if (followAccount) sharedRead("appearance") else read("device_appearance") ?: read("appearance")) }
    var page by rememberSaveable { mutableStateOf("inbox") }
    var callMinimized by remember(state.call?.call?.id) { mutableStateOf(false) }
    var chatTheme by remember { mutableStateOf(ChatTheme()) }
    var conversationPage by rememberSaveable { mutableStateOf("") }
    var selected by remember { mutableStateOf(setOf<String>()) }
    var query by rememberSaveable { mutableStateOf("") }
    var category by rememberSaveable { mutableStateOf("") }
    var collection by rememberSaveable { mutableStateOf("") }
    var collectionSheet by remember { mutableStateOf(false) }
    var actionSheet by remember { mutableStateOf("") }
    var actionFields by remember { mutableStateOf(emptyMap<String, Any?>()) }
    val dispatch: Command = { name, fields ->
        if (name in listOf("snooze_picker", "forward_picker", "block_picker", "delete_picker")) { actionSheet = name; actionFields = fields }
        else command(name, fields)
    }
    val drafts = remember { mutableMapOf<String, TextFieldState>() }
    val chat = state.chats.find { it.id == state.selected }
    val focus = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    val backActions = remember { mutableStateListOf<() -> Unit>() }
    val navigate: (String) -> Unit = { target ->
        focus.clearFocus(); keyboard?.hide(); selected = emptySet(); page = target; query = ""; category = ""
    }
    val back: () -> Unit = {
        when {
            state.call != null && !callMinimized -> callMinimized = true
            backActions.isNotEmpty() -> backActions.last()()
            selected.isNotEmpty() -> selected = emptySet()
            conversationPage.isNotEmpty() -> conversationPage = ""
            page in listOf("appearance", "device", "profile", "privacy", "notifications", "storage", "about") -> navigate("settings")
            chat != null -> { command("close", emptyMap()); conversationPage = "" }
            page != "inbox" -> navigate("inbox")
        }
    }
    SideEffect { onBackAvailable(state.call != null || backActions.isNotEmpty() || selected.isNotEmpty() || page != "inbox" || chat != null, back) }
    LaunchedEffect(chat?.id, chat?.ui?.get("chat_theme")) { chatTheme = decodeChat(chat?.ui?.get("chat_theme") ?: read("chat.${chat?.id}")) }
    LaunchedEffect(page, query, category, conversationPage, state.sent) {
        if (chat == null && page in listOf("search", "notes")) {
            kotlinx.coroutines.delay(180)
            command("search", mapOf("query" to if (page == "notes") "" else query, "category" to if (page == "notes") "Notes" else category))
        }
    }
    val open: (String) -> Unit = { peer -> focus.clearFocus(); keyboard?.hide(); command("open", mapOf("peer" to peer)) }
    SigilTheme(appearance, if (chat != null) chatTheme else null, dynamicAccent, palette) {
      CompositionLocalProvider(LocalBackActions provides backActions) {
        Surface(color = MaterialTheme.colorScheme.background) {
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
                        val destination = when { state.call != null && !callMinimized -> "call"; chat != null -> when (conversationPage) { "Chat theme" -> "theme"; "Settings" -> "chat-settings"; else -> "conversation" }; page in listOf("inbox", "search", "notes") -> "home"; else -> page }
                        if (state.call != null && callMinimized) TextButton({ callMinimized = false }, Modifier.fillMaxWidth()) { Glyph("call", 18); Spacer(Modifier.width(8.dp)); Text("Return to call") }
                        AnimatedContent(destination, Modifier.weight(1f), transitionSpec = {
                            (fadeIn(tween(MotionMillis)) + slideInVertically(tween(MotionMillis)) { it / 14 }) togetherWith fadeOut(tween(120))
                        }, label = "Page") { target ->
                            when (target) {
                                "call" -> state.call?.let { CallPage(it, state.chats, dispatch) { callMinimized = true } }
                                "conversation" -> CompositionLocalProvider(LocalPageMotion provides this) { chat?.let { ConversationPage(it, state, drafts.getOrPut(it.id) { TextFieldState(it.draft) }, analyze, dispatch, back,
                                    conversationPage, { conversationPage = it }, chatTheme.gradient) } }
                                "theme" -> chat?.let { current -> ChatAppearance(chatTheme, current.id, command, { conversationPage = "" }) { chatTheme = it; write("chat.${current.id}", it.encode()); command("organize", mapOf("peer" to current.id, "value" to mapOf("UiSetting" to mapOf("key" to "chat_theme", "value" to it.encode())))) } }
                                "settings" -> SettingsPage(state, navigate, back)
                                "chat-settings" -> chat?.let { ConversationSettings(it, state.busy, dispatch, back) }
                                "appearance" -> AppearancePage(appearance, dynamicAccent != null, back, state.collectionsEnabled,
                                    { enabled -> command("organize", mapOf("peer" to null, "value" to mapOf("CollectionsEnabled" to enabled))) },
                                    sharedRead("collection_labels") != "false", { sharedWrite("collection_labels", it.toString()) }, followAccount,
                                    { if (!it) write("device_appearance", appearance.encode()); followAccount = it; write("follow_account_theme", it.toString()) }) { appearance = it; if (followAccount) sharedWrite("appearance", it.encode()) else write("device_appearance", it.encode()) }
                                "device", "profile", "privacy", "notifications", "storage", "about" -> PersonalPage(target, state, command, back)
                                "new" -> NewConversation(state, command, back, open)
                                "calls" -> CallHistoryPage(state, command, back)
                                else -> Column(Modifier.fillMaxSize()) {
                                    InboxHeader(page, query, { query = it }, selected, state, dispatch, { selected = emptySet() },
                                        { collectionSheet = true }, { navigate("search") }, { navigate("notes") }, back)
                                    Box(Modifier.weight(1f)) {
                                      AnimatedContent(page, transitionSpec = {
                                          (slideInVertically(tween(MotionMillis)) { if (targetState == "inbox") -it / 12 else it } + fadeIn(tween(MotionMillis))) togetherWith
                                              (slideOutVertically(tween(MotionMillis)) { if (targetState == "inbox") it else -it / 12 } + fadeOut(tween(120)))
                                      }, label = "Inbox panel") { panel ->
                                        when (panel) {
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
                        AnimatedVisibility(chat == null && page == "inbox" && (state.call == null || callMinimized), enter = slideInVertically(tween(MotionMillis)) { it } + fadeIn(), exit = slideOutVertically(tween(MotionMillis)) { it } + fadeOut()) {
                            Surface(shape = RoundedCornerShape(topStart = 24.dp, topEnd = 24.dp), border = BorderStroke(0.5.dp, MaterialTheme.colorScheme.outlineVariant)) {
                                Row(Modifier.fillMaxWidth().padding(vertical = 8.dp), horizontalArrangement = Arrangement.SpaceEvenly) {
                                    Surface(shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.inverseSurface, contentColor = MaterialTheme.colorScheme.inverseOnSurface) { Symbol("chat_bubble", "Messages") { navigate("inbox") } }
                                    Symbol("call", "Calls") { navigate("calls") }
                                    Symbol("settings", "Settings") { navigate("settings") }
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
