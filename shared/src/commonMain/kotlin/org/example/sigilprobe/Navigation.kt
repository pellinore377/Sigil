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
import androidx.compose.ui.zIndex
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.input.key.*
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp

internal const val MotionMillis = 240
internal val MainTabs = listOf("inbox", "calls", "settings")
internal fun tabGoesBack(from: String, to: String) = from in MainTabs && to in MainTabs && MainTabs.indexOf(to) < MainTabs.indexOf(from)
private data class Screen(val destination: String, val page: String, val chat: ChatSummary?, val detail: String, val state: MessengerState, val title: String = "", val thread: String? = null)
internal val LocalNavigationBack = staticCompositionLocalOf { false }
internal val LocalPageMotion = staticCompositionLocalOf<AnimatedContentScope?> { null }
internal val LocalHeaderInset = staticCompositionLocalOf { 0.dp }
internal val LocalFooterHeight = staticCompositionLocalOf<((androidx.compose.ui.unit.Dp) -> Unit)?> { null }
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
    var callPanel by remember(state.call?.call?.id) { mutableStateOf("") }
    var callMinimized by remember(state.call?.call?.id) { mutableStateOf(false) }
    var chatTheme by remember { mutableStateOf(ChatTheme()) }
    var pendingChatTheme by remember { mutableStateOf<Pair<String, String>?>(null) }
    var newTitle by remember { mutableStateOf("New conversation") }
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
    val navigate: (String) -> Unit = navigate@{ target ->
        if (target == "contact-code") { command("contact_qr", mapOf("action" to "show")); return@navigate }
        if (chat == null) command("close", emptyMap())
        if (target == "new") newTitle = "New conversation"
        goingBack = tabGoesBack(page, target); focus.clearFocus(); keyboard?.hide(); selected = emptySet(); page = target; query = ""; category = ""
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
    SigilTheme(appearance, if (chat != null) chatTheme else null, dynamicAccent, palette, chatKey = chat?.id) {
      CompositionLocalProvider(LocalBackActions provides backActions) {
        val mainHeader = chat == null && (state.call == null || callMinimized) && page in listOf("inbox", "calls", "settings", "search", "notes")
        Surface(color = if (mainHeader || chat == null) MaterialTheme.colorScheme.background else lerp(MaterialTheme.colorScheme.background, MaterialTheme.colorScheme.surface, LocalChatTint.current)) {
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
                        if (state.accountAccess?.let { it.linked && it.retiring && !it.acknowledged } == true && page != "profile" && state.call == null) SigilTextButton({ command("close", emptyMap()); conversationPage = ""; navigate("profile") }, Modifier.fillMaxWidth()) { Text("Your server’s sign-in is changing · Review") }
                        val destination = when { state.call != null && !callMinimized -> "call"; chat?.archived == true -> "saved-conversation"; chat != null -> when (conversationPage) { "Chat theme" -> "theme"; "Settings" -> "chat-settings"; else -> "conversation" }; page in listOf("inbox", "search", "notes", "calls", "settings") -> "home"; else -> page }
                        if (state.call != null && callMinimized) SigilTextButton({ callMinimized = false }, Modifier.fillMaxWidth()) { Glyph("call", 18); Spacer(Modifier.width(8.dp)); Text("Return to call") }
                        val headerHeight = pageHeaderHeight()
                        val conversation = destination == "conversation"
                        var composerHeight by remember { mutableStateOf(72.dp) }
                        val footerHeight = if (conversation) composerHeight else 64.dp
                        val hasFooter = conversation || (destination == "home" && page in MainTabs)
                        val navigationInset = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()
                        val footerOffset by animateDpAsState(if (hasFooter) 0.dp else footerHeight + navigationInset, tween(MotionMillis), label = "Footer position")
                        val tint = LocalChatTint.current
                        val radius by animateDpAsState(if (conversation) 24.dp else 0.dp, tween(MotionMillis), label = "Header corners")
                        val shape = RoundedCornerShape(bottomStart = radius, bottomEnd = radius)
                        Box(Modifier.weight(1f).fillMaxWidth().background(MaterialTheme.colorScheme.background)) {
                            if (chat != null) {
                                LocalWallpaper.current(chat.id, Modifier.matchParentSize())
                                if (chatTheme.gradient) Spacer(Modifier.matchParentSize().background(Brush.verticalGradient(listOf(MaterialTheme.colorScheme.background.copy(alpha = .7f), MaterialTheme.colorScheme.primaryContainer.copy(alpha = .7f)))))
                            }
                            Surface(Modifier.align(Alignment.BottomCenter).fillMaxWidth().height(footerHeight).offset(y = footerOffset).footerShadow().testTag("footer-surface"), shape = RoundedCornerShape(topStart = 24.dp, topEnd = 24.dp)) {}
                        AnimatedContent(Screen(destination, page, chat, conversationPage, state, newTitle, thread?.id), Modifier.fillMaxSize(), contentKey = { it.destination }, transitionSpec = {
                            val enter = if (targetState.destination == "conversation") EnterTransition.None
                                else if (goingBack) fadeIn(tween(MotionMillis)) else slideInVertically(tween(MotionMillis)) { it } + fadeIn(tween(MotionMillis))
                            val exit = if (initialState.destination == "conversation") ExitTransition.None
                                else if (goingBack) slideOutVertically(tween(MotionMillis)) { it } + fadeOut(tween(MotionMillis)) else fadeOut(tween(MotionMillis))
                            enter togetherWith exit
                            }, label = "Page") { screen ->
                                val target = screen.destination
                                val page = screen.page
                                val chat = screen.chat
                                val state = screen.state
                                val detail = screen.detail
                                CompositionLocalProvider(LocalPageHeader provides true, LocalPageMotion provides this, LocalNavigationBack provides goingBack, LocalHeaderInset provides if (target == "conversation") headerHeight else 0.dp, LocalFooterHeight provides if (target == "conversation") ({ composerHeight = it }) else null) {
                                Box(Modifier.fillMaxSize().then(if (target != "conversation") Modifier.padding(top = headerHeight, bottom = if (target == "home" && page in MainTabs) 64.dp else 0.dp).background(MaterialTheme.colorScheme.background) else Modifier)) {
                            when (target) {
                                "call" -> state.call?.let { CallPage(it, state.chats, dispatch, state.profileAvatar, callPanel) { callPanel = it } }
                                "conversation" -> chat?.let { ConversationPage(it, state, drafts.getOrPut(it.id) { TextFieldState(it.draft) }, analyze, dispatch, detail, chatTheme.gradient, thread, { thread = it }) }
                                "theme" -> chat?.let { current -> ChatAppearance(chatTheme, analyze, current.id, command, { goingBack = true; conversationPage = "" }) { chatTheme = it; pendingChatTheme = current.id to it.encode(); write("chat.${current.id}", it.encode()); command("organize", mapOf("peer" to current.id, "value" to mapOf("UiSetting" to mapOf("key" to "chat_theme", "value" to it.encode())))) } }
                                "chat-settings" -> chat?.let { ConversationSettings(it, state.busy, dispatch, back) }
                                "appearance" -> AppearancePage(appearance, analyze, dynamicAccent != null, back, state.collectionsEnabled,
                                    { enabled -> command("organize", mapOf("peer" to null, "value" to mapOf("CollectionsEnabled" to enabled))) },
                                    sharedRead("collection_labels") != "false", { sharedWrite("collection_labels", it.toString()) }, followAccount,
                                    { if (!it) write("device_appearance", appearance.encode()); followAccount = it; write("follow_account_theme", it.toString()) }) { appearance = it; if (followAccount) { pendingAppearance = it.encode(); sharedWrite("appearance", it.encode()) } else write("device_appearance", it.encode()) }
                                "device", "profile", "privacy", "notifications", "storage", "about" -> PersonalPage(target, state, dispatch, back)
                                "history" -> SavedHistoryPage(state, command, back, open)
                                "saved-conversation" -> SavedConversationPage(state, analyze, command, back)
                                "new" -> NewConversation(state, command, back, open) { newTitle = it }
                                else -> Column(Modifier.fillMaxSize()) {
                                    Box(Modifier.weight(1f)) {
                                      AnimatedContent(page, transitionSpec = {
                                          if (initialState in MainTabs && targetState in MainTabs) {
                                              (slideInHorizontally(tween(MotionMillis)) { if (goingBack) -it else it } + fadeIn(tween(MotionMillis))) togetherWith
                                                  (slideOutHorizontally(tween(MotionMillis)) { if (goingBack) it else -it } + fadeOut(tween(120)))
                                          } else {
                                              (if (goingBack) fadeIn(tween(MotionMillis)) else slideInVertically(tween(MotionMillis)) { it } + fadeIn(tween(MotionMillis))) togetherWith
                                                  (if (goingBack) slideOutVertically(tween(MotionMillis)) { it } + fadeOut(tween(MotionMillis)) else fadeOut(tween(MotionMillis)))
                                          }
                                      }, label = "Inbox panel") { panel ->
                                        Box(Modifier.fillMaxSize().testTag("main-page-$panel")) {
                                        when (panel) {
                                            "settings" -> SettingsPage(state, navigate)
                                            "calls" -> CallHistoryPage(state, command)
                                            "search" -> SearchPage(state, query, category, { category = it }, open, dispatch)
                                            "notes" -> NotesGrid(state, query, sharedRead, sharedWrite, { peer -> open(peer); conversationPage = "Notes" }, dispatch)
                                            else -> Inbox(state, collection, { collection = it }, selected, { id -> selected = if (id in selected) selected - id else selected + id }, open, sharedRead)
                                        }
                                      }
                                      }
                                    }
                                }
                            }
                        }
                            }
                            }
                                Surface(Modifier.fillMaxWidth().height(headerHeight).zIndex(1f).testTag("main-header").headerShadow(shape, if (conversation) (2f * tint).dp else 0.dp), shape = shape,
                                    color = if (conversation) lerp(MaterialTheme.colorScheme.background, MaterialTheme.colorScheme.surface, tint) else MaterialTheme.colorScheme.background) {
                                    AnimatedContent(Screen(destination, page, chat, conversationPage, state, newTitle, thread?.id), contentKey = { if (it.destination == "home") "home" else it.destination + if (it.destination == "conversation") it.detail + (it.thread ?: "") else if (it.destination == "new") it.title else "" }, transitionSpec = {
                                        (slideInHorizontally(tween(160, delayMillis = 80)) { if (goingBack) -48 else 48 } + fadeIn(tween(160, delayMillis = 80))) togetherWith
                                            (if (goingBack) slideOutHorizontally(tween(MotionMillis)) { 48 } + fadeOut(tween(100)) else fadeOut(tween(100)))
                                    }, label = "Header items") { screen ->
                                        when (screen.destination) {
                                            "call" -> screen.state.call?.let { CallHeader(it, screen.state.chats, screen.state.profileAvatar, dispatch, { callMinimized = true }) { callPanel = it } }
                                            "home" -> MainHeader(screen.page, goingBack, query, { query = it }, selected, screen.state, dispatch, { selected = emptySet() },
                                                { collectionSheet = true }, { navigate("search") }, { navigate("notes") }, back)
                                            "conversation" -> screen.chat?.let { ConversationHeader(it, screen.detail, screen.thread != null, dispatch, back) { goingBack = false; conversationPage = it } }
                                            else -> Row(Modifier.fillMaxSize().padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                                                Symbol("chevron_left", "Back", back)
                                                Text(when (screen.destination) {
                                                    "appearance" -> "Appearance"; "theme" -> "Conversation appearance"; "chat-settings" -> "Conversation settings"
                                                    "device" -> "Devices"; "profile" -> "Profile"; "privacy" -> "Privacy"; "notifications" -> "Notifications"
                                                    "storage" -> "Data and storage"; "history" -> "Saved history"; "saved-conversation" -> "Saved conversation"
                                                    "new" -> screen.title; else -> "About"
                                                }, Modifier.padding(start = 8.dp), style = MaterialTheme.typography.headlineMedium)
                                            }
                                        }
                                    }
                                }
                        androidx.compose.animation.AnimatedVisibility(chat == null && page in listOf("inbox", "calls", "settings") && (state.call == null || callMinimized), modifier = Modifier.align(Alignment.BottomCenter).zIndex(2f), enter = slideInVertically(tween(MotionMillis)) { it } + fadeIn(), exit = slideOutVertically(tween(MotionMillis)) { it } + fadeOut()) {
                                Row(Modifier.fillMaxWidth().padding(vertical = 8.dp).testTag("main-navigation"), horizontalArrangement = Arrangement.SpaceEvenly) {
                                    listOf(Triple("inbox", "chat_bubble", "Messages"), Triple("calls", "call", "Calls"), Triple("settings", "settings", "Settings")).forEach { (tab, icon, label) ->
                                        Surface(shape = RoundedCornerShape(16.dp), color = if (page == tab) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surface,
                                            contentColor = if (page == tab) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurfaceVariant) { Symbol(icon, label) { navigate(tab) } }
                                    }
                                }
                        }
                        InboxFab(destination == "home" && page == "inbox", Modifier.align(Alignment.BottomEnd).padding(bottom = 64.dp).zIndex(3f)) { navigate("new") }
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
