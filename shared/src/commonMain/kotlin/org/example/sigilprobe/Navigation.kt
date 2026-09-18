package org.sigil

import androidx.compose.ui.geometry.Rect
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
import androidx.compose.ui.zIndex
import androidx.compose.ui.input.key.*
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.positionInWindow
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

internal data class ClientFeatures(val calls:Boolean=true,val files:Boolean=true,val voice:Boolean=true,val locations:Boolean=true,val notifications:Boolean=true,val recovery:Boolean=true,val videoCalls:Boolean=true)
internal val LocalWideLayout=staticCompositionLocalOf {false}
internal val LocalClientFeatures=staticCompositionLocalOf {ClientFeatures()}
internal val MainTabs = listOf("inbox", "calls", "notes", "settings")
internal fun tabGoesBack(from: String, to: String) = from in MainTabs && to in MainTabs && MainTabs.indexOf(to) < MainTabs.indexOf(from)
private data class Screen(val destination: String, val page: String, val chat: ChatSummary?, val detail: String, val state: MessengerState, val title: String = "", val thread: String? = null)
internal val LocalNavigationBack = staticCompositionLocalOf { false }
internal val LocalPageMotion = staticCompositionLocalOf<AnimatedContentScope?> { null }
internal val LocalHomeContentPadding = staticCompositionLocalOf { PaddingValues(0.dp) }
internal val LocalHeaderInset = staticCompositionLocalOf { 0.dp }
internal class FooterHost {
    var content: (@Composable () -> Unit)? by mutableStateOf(null)
    var height by mutableStateOf(68.dp)
    var headerBottom by mutableFloatStateOf(0f)
}
internal val LocalFooterHost = staticCompositionLocalOf<FooterHost?> { null }
@Composable
internal fun FooterContent(content: @Composable () -> Unit) {
    val host = LocalFooterHost.current
    if (host == null) { content(); return }
    val current by rememberUpdatedState(content)
    val slot = remember { movableContentOf { current() } }
    DisposableEffect(host) {
        host.content = slot
        onDispose { if (host.content === slot) host.content = null }
    }
}
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
    onBackAvailable: (Boolean, () -> Unit) -> Unit = { _, _ -> }, wideLayout: Boolean = false, overlay: @Composable () -> Unit = {}) {
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
    var pendingChatTheme by remember { mutableStateOf<Pair<String, String>?>(null) }
    var newTitle by remember { mutableStateOf("New conversation") }
    var conversationPage by rememberSaveable { mutableStateOf("") }
    var selected by remember { mutableStateOf(setOf<String>()) }
    var query by rememberSaveable { mutableStateOf("") }
    var category by rememberSaveable { mutableStateOf("") }
    var collection by rememberSaveable { mutableStateOf("") }
    var collectionSheet by remember { mutableStateOf(false) }
    var newCall by remember { mutableStateOf(false) }
    var callDetail by remember { mutableStateOf<String?>(null) }
    var actionSheet by remember { mutableStateOf("") }
    var actionFields by remember { mutableStateOf(emptyMap<String, Any?>()) }
    val dispatch: Command = { name, fields ->
        if (name == "call_resume" && state.call?.call?.id == fields["call"]) callMinimized = false
        else if (name == "history_open") page = "history"
        else if (name in listOf("snooze_picker", "forward_picker", "block_picker", "delete_picker")) { actionSheet = name; actionFields = fields }
        else command(name, fields)
    }
    val drafts = remember { mutableMapOf<String, TextFieldState>() }
    val chat = state.chats.find { it.id == state.selected }
    var chatTheme by remember(chat?.id) { mutableStateOf(decodeChat(chat?.ui?.get("chat_theme") ?: read("chat.${chat?.id}"))) }
    var thread by remember(chat?.id) { mutableStateOf(state.threadTarget) }
    val focus = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    val backActions = remember { mutableStateListOf<() -> Unit>() }
    val navigate: (String) -> Unit = navigate@{ target ->
        if (target == "contact-code") { command("contact_qr", mapOf("action" to "show")); return@navigate }
        if (target == "new") newTitle = "New conversation"
        callDetail = null
        goingBack = tabGoesBack(page, target); focus.clearFocus(); keyboard?.hide(); selected = emptySet(); page = target; query = ""; category = ""
    }
    val back: () -> Unit = {
        when {
            state.call != null && !callMinimized -> callMinimized = true
            backActions.isNotEmpty() -> backActions.last()()
            selected.isNotEmpty() -> selected = emptySet()
            conversationPage.isNotEmpty() -> conversationPage = ""
            page.startsWith("appearance-") -> navigate("appearance")
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
    val globalQuery = if (page == "notes") "" else query
    val globalCategory = if (page == "notes") "Notes" else category
    LaunchedEffect(page, globalQuery, globalCategory, chat?.id, state.sent) {
        if (chat == null && page in listOf("search", "notes")) {
            if (page == "search") kotlinx.coroutines.delay(180)
            command("search", mapOf("query" to globalQuery, "category" to globalCategory))
        }
    }
    val open: (String) -> Unit = { peer -> goingBack = false; focus.clearFocus(); keyboard?.hide(); command("open", mapOf("peer" to peer)) }
    SigilTheme(appearance, if (chat != null) chatTheme else null, dynamicAccent, palette, chatKey = chat?.id) {
      val motionPolicy = LocalMotion.current
      val footer = remember { FooterHost() }
      val materialOcclusion = remember { MaterialOcclusion() }
      val materialOverlayHost = remember { MaterialOverlayHost() }
      val presentationHost = remember { PresentationHost() }
      CompositionLocalProvider(LocalBackActions provides backActions, LocalFooterHost provides footer, LocalMaterialOcclusion provides materialOcclusion, LocalMaterialOverlayHost provides materialOverlayHost, LocalPresentationHost provides presentationHost) {
        BackAction(callDetail != null) { callDetail = null }
        BoxWithConstraints(Modifier.fillMaxSize()) {
        val wide = wideLayout && maxWidth >= 1000.dp && state.phase == "connected"
        CompositionLocalProvider(LocalWideLayout provides wide) {
        Row(Modifier.fillMaxSize().background(LocalGlobalWorkspace.current).padding(if (wide) 20.dp else 0.dp), horizontalArrangement = Arrangement.spacedBy(if (wide) 20.dp else 0.dp)) {
        if (wide) SigilTheme(appearance, palette = palette) {
            NavigationPane(state, collection, { collection = it }, selected,
                { id -> selected = if (id in selected) selected - id else selected + id }, { selected = emptySet() }, { collectionSheet = true },
                open, { target -> command("close", emptyMap()); conversationPage = ""; navigate(target) }, dispatch, sharedRead, page)
        }
        Surface(Modifier.weight(1f).fillMaxHeight(), shape = RoundedCornerShape(if (wide) 28.dp else 0.dp), color = LocalGlobalBackground.current, contentColor = MaterialTheme.colorScheme.onBackground) {
            Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.displayCutout.only(WindowInsetsSides.Horizontal)).onPreviewKeyEvent {
                if (it.type == KeyEventType.KeyDown && it.key == Key.Escape) { back(); true } else false
            }) {
                when {
                    state.phase == "loading" -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
                    state.phase == "unavailable" -> Box(Modifier.fillMaxSize().padding(32.dp), contentAlignment = Alignment.Center) { Text("Connected messaging is currently available in the Android development build.") }
                    state.phase != "connected" -> Box(Modifier.imePadding()) {
                        SignIn(state, command)
                        state.issue?.let { issue -> Box(Modifier.align(Alignment.BottomCenter).padding(16.dp).widthIn(max = 620.dp)) { SyncNotice(issue) { command("dismiss", emptyMap()) } } }
                    }
                    else -> {
                        val accessNotice = state.accountAccess?.let { it.linked && it.retiring && !it.acknowledged } == true && page != "profile" && state.call == null
                        val destination = when { wide && chat == null && page == "inbox" -> "welcome"; state.call != null && !callMinimized -> "call"; chat?.archived == true -> "saved-conversation"; chat != null -> when (conversationPage) { "Chat theme" -> "theme"; "Settings" -> "chat-settings"; else -> "conversation" }; page in listOf("inbox", "search", "notes", "calls", "settings") -> "home"; else -> page }
                        val floating = destination == "conversation"
                        val statusInset = if (!wide) WindowInsets.statusBars.asPaddingValues().calculateTopPadding() else 0.dp
                        val headerTop = statusInset + 12.dp
                        val headerBase = if (destination == "welcome") 0.dp else if (floating || destination == "call") pageHeaderHeight() + 8.dp else mainHeaderHeight()
                        val headerScreen = Screen(destination, page, chat, conversationPage, state, newTitle, thread?.id)
                        val headerKey = if (destination == "home") "home" else destination + if (destination == "conversation") conversationPage + (thread?.id ?: "") else if (destination == "new") newTitle else ""
                        var mainHeaderKey by remember { mutableStateOf(headerKey) }
                        SideEffect { if (!floating) mainHeaderKey = headerKey }
                        val headerTransition = updateTransition(mainHeaderKey, label = "Header")
                        val headerHeight = headerBase
                        val conversation = destination == "conversation"
                        var conversationLayers by remember { mutableIntStateOf(0) }
                        val backdrop = rememberChromeBackdrop()
                        val navigationInset = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()
                        val shape = RoundedCornerShape(24.dp)
                        var workspaceTop by remember {mutableFloatStateOf(0f)}
                        val settledHeaderExtent=with(LocalDensity.current){(statusInset+12.dp+pageHeaderHeight()+8.dp).toPx()}
                        SideEffect {footer.headerBottom=workspaceTop+settledHeaderExtent}
                        Box(Modifier.weight(1f).fillMaxWidth().background(LocalGlobalBackground.current).onGloballyPositioned {workspaceTop=it.positionInWindow().y}) {
                        AnimatedContent(Screen(destination, page, chat, conversationPage, state, newTitle, thread?.id), Modifier.fillMaxSize().captureBackdrop(backdrop), contentKey = { it.destination }, transitionSpec = {
                            val enter = if (targetState.destination == "conversation") EnterTransition.None
                                else if (goingBack) fadeIn(motionPolicy.enter(MotionMillis)) else slideInVertically(motionPolicy.enter(MotionMillis)) { it } + fadeIn(motionPolicy.enter(MotionMillis))
                            val exit = if (initialState.destination == "conversation") ExitTransition.None
                                else if (goingBack) slideOutVertically(motionPolicy.exit(MotionQuick)) { it } + fadeOut(motionPolicy.exit(MotionExit)) else fadeOut(motionPolicy.exit(MotionExit))
                            (enter togetherWith exit).apply { targetContentZIndex = if(initialState.destination=="conversation" && targetState.destination in listOf("home","welcome"))-1f else 0f }
                            }, label = "Page") { screen ->
                                val target = screen.destination
                                if(target=="conversation")DisposableEffect(Unit) {conversationLayers++;onDispose {conversationLayers--}}
                                val page = screen.page
                                val chat = screen.chat
                                val state = screen.state
                                val detail = screen.detail
                                val scrollingDetail = target.startsWith("appearance") || target in listOf("chat-settings", "device", "profile", "privacy", "notifications", "storage", "about")
                                CompositionLocalProvider(LocalPageHeader provides true, LocalPageMotion provides this, LocalNavigationBack provides goingBack, LocalHomeContentPadding provides PaddingValues(top = headerTop + headerHeight + 12.dp, bottom = if (scrollingDetail) navigationInset + 24.dp else if (!wide) 88.dp + navigationInset else 24.dp), LocalHeaderInset provides if (target == "conversation") pageHeaderHeight() + statusInset + 32.dp else 0.dp) {
                                Box(Modifier.fillMaxSize().then(if (target != "conversation" && !scrollingDetail && !(target == "home" && page in MainTabs)) Modifier.padding(top = headerTop + headerHeight + 12.dp, bottom = navigationInset).imePadding() else Modifier)) {
                            when (target) {
                                "welcome" -> ConversationWelcome()
                                "call" -> state.call?.let { CallPage(it, state.chats, dispatch, state.profileAvatar, callPanel) { callPanel = it } }
                                "conversation" -> chat?.let { ConversationPage(it, state, drafts.getOrPut(it.id) { TextFieldState(it.draft) }, analyze, dispatch, detail, chatTheme.gradient ?: appearance.gradient, thread, { thread = it }) }
                                "theme" -> chat?.let { current -> ChatAppearance(chatTheme, analyze, current.id, command, { goingBack = true; conversationPage = "" }) { chatTheme = it; pendingChatTheme = current.id to it.encode(); write("chat.${current.id}", it.encode()); command("organize", mapOf("peer" to current.id, "value" to mapOf("UiSetting" to mapOf("key" to "chat_theme", "value" to it.encode())))) } }
                                "chat-settings" -> chat?.let { ConversationSettings(it, state.busy, dispatch, back) }
                                "appearance", "appearance-colors", "appearance-type", "appearance-layout", "appearance-media", "appearance-objects" -> AppearancePage(appearance, analyze, dynamicAccent != null, back, state.collectionsEnabled,
                                    { enabled -> command("organize", mapOf("peer" to null, "value" to mapOf("CollectionsEnabled" to enabled))) },
                                    sharedRead("collection_labels") != "false", { sharedWrite("collection_labels", it.toString()) }, followAccount,
                                    { if (!it) write("device_appearance", appearance.encode()); followAccount = it; write("follow_account_theme", it.toString()) }, target, navigate) { appearance = it; if (followAccount) { pendingAppearance = it.encode(); sharedWrite("appearance", it.encode()) } else write("device_appearance", it.encode()) }
                                "device", "profile", "privacy", "notifications", "storage", "about" -> PersonalPage(target, state, dispatch, back)
                                "history" -> SavedHistoryPage(state, command, back, open)
                                "saved-conversation" -> SavedConversationPage(state, analyze, command, back)
                                "new" -> NewConversation(state, command, back, open) { newTitle = it }
                                else -> Column(Modifier.fillMaxSize()) {
                                    Box(Modifier.weight(1f)) {
                                      AnimatedContent(page, transitionSpec = {
                                          if (initialState in MainTabs && targetState in MainTabs) {
                                              (slideInHorizontally(motionPolicy.enter(MotionMillis)) { if (goingBack) -it else it } + fadeIn(motionPolicy.enter(MotionMillis))) togetherWith
                                                  (slideOutHorizontally(motionPolicy.exit(MotionQuick)) { if (goingBack) it else -it } + fadeOut(motionPolicy.exit(MotionExit)))
                                          } else {
                                              (if (goingBack) fadeIn(motionPolicy.enter(MotionMillis)) else slideInVertically(motionPolicy.enter(MotionMillis)) { it } + fadeIn(motionPolicy.enter(MotionMillis))) togetherWith
                                                  (if (goingBack) slideOutVertically(motionPolicy.exit(MotionQuick)) { it } + fadeOut(motionPolicy.exit(MotionExit)) else fadeOut(motionPolicy.exit(MotionExit)))
                                          }
                                      }, label = "Inbox panel") { panel ->
                                        Box(Modifier.fillMaxSize().background(LocalGlobalBackground.current).testTag("main-page-$panel")) {
                                        when (panel) {
                                            "settings" -> SettingsPage(state, navigate)
                                            "calls" -> CallHistoryPage(state, dispatch, callDetail) { callDetail = it }
                                            "search" -> SearchPage(state, query, category, { category = it }, open, dispatch)
                                            "notes" -> NotesGrid(state, query, sharedRead, sharedWrite, { peer -> goingBack = false; focus.clearFocus(); keyboard?.hide(); conversationPage = "Notes"; command("open", mapOf("peer" to peer, "category" to "Notes")) }, dispatch)
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
                                androidx.compose.animation.AnimatedVisibility(!conversation && destination != "welcome" && conversationLayers==0, Modifier.align(Alignment.TopCenter).zIndex(4f), enter = fadeIn(motionPolicy.enter(MotionQuick)), exit = fadeOut(motionPolicy.exit(MotionExit)), label = "Main chrome") {
                                FloatingChrome(backdrop, Modifier.padding(top = headerTop).widthIn(max = 920.dp).fillMaxWidth().padding(horizontal = 12.dp).height(if (mainHeaderKey == "call") pageHeaderHeight() + 8.dp else mainHeaderHeight()).zIndex(4f).testTag("main-header"), shape = shape) {
                                    headerTransition.AnimatedContent(Modifier.fillMaxSize(), contentAlignment = Alignment.Center, transitionSpec = {
                                        (slideInHorizontally(motionPolicy.enter(MotionQuick, delayMillis = MotionStagger)) { if (goingBack) -it else it } + fadeIn(motionPolicy.enter(MotionQuick, delayMillis = MotionStagger))) togetherWith
                                            (slideOutHorizontally(motionPolicy.exit(MotionQuick)) { if (goingBack) it else -it } + fadeOut(motionPolicy.exit(MotionExit)))
                                    }) { key ->
                                        var retained by remember { mutableStateOf(headerScreen) }
                                        SideEffect { if (!conversation && key == headerKey) retained = headerScreen }
                                        val screen = if (!conversation && key == headerKey) headerScreen else retained
                                        when (screen.destination) {
                                            "call" -> screen.state.call?.let { CallHeader(it, screen.state.chats, screen.state.profileAvatar, dispatch, { callMinimized = true }) { callPanel = it } }
                                            "home" -> MainHeader(screen.page, goingBack, query, { query = it }, selected, screen.state, dispatch, { selected = emptySet() },
                                                { collectionSheet = true }, { navigate("search") }, { navigate("new") }, back, { newCall = true }, callDetail != null, { callDetail = null })
                                            else -> Row(Modifier.fillMaxSize().padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                                                Symbol("chevron_left", "Back", back)
                                                MainHeaderTitle(when (screen.destination) {
                                                    "appearance", "appearance-colors", "appearance-type", "appearance-layout", "appearance-media", "appearance-objects" -> appearanceTitle(screen.destination); "theme" -> "Conversation appearance"; "chat-settings" -> "Conversation settings"
                                                    "device" -> "Devices"; "profile" -> "Profile"; "privacy" -> "Privacy"; "notifications" -> "Notifications"
                                                    "storage" -> "Data and storage"; "history" -> "Saved history"; "saved-conversation" -> "Saved conversation"
                                                    "new" -> screen.title; else -> "About"
                                                }, Modifier.weight(1f).padding(start = 8.dp))
                                            }
                                        }
                                    }
                                }
                                }
                        if (!wide) {
                            StatusFade(statusInset, Modifier.align(Alignment.TopCenter).zIndex(3.75f).testTag(if (conversation) "conversation-status-fade" else "main-status-fade"))
                        }
                        var retainedConversation by remember { mutableStateOf(headerScreen) }
                        SideEffect { if (conversation) retainedConversation = headerScreen }
                        androidx.compose.animation.AnimatedVisibility(conversation, Modifier.align(Alignment.TopCenter).zIndex(4f),
                            enter = slideInVertically(motionPolicy.enter(MotionInline, delayMillis = MotionMillis)) { -it } + fadeIn(motionPolicy.enter(MotionInline, delayMillis = MotionMillis)),
                            exit = slideOutVertically(motionPolicy.exit(MotionQuick)) { -it } + fadeOut(motionPolicy.exit(MotionExit)), label = "Conversation header") {
                            val screen = if (conversation) headerScreen else retainedConversation
                            FloatingChrome(backdrop, Modifier.padding(top = if (wide) 12.dp else WindowInsets.statusBars.asPaddingValues().calculateTopPadding() + 12.dp).widthIn(max = 920.dp).fillMaxWidth().padding(horizontal = 12.dp).height(pageHeaderHeight() + 8.dp).testTag("conversation-header").onGloballyPositioned { materialOcclusion.header = it.boundsInWindow() }, RoundedCornerShape(24.dp)) {
                                screen.chat?.let { ConversationHeader(it, screen.detail, screen.thread != null, dispatch, back) { goingBack = false; thread = null; conversationPage = it } }
                            }
                        }
                        androidx.compose.animation.AnimatedVisibility(!wide && chat == null && page in listOf("inbox", "calls", "notes", "settings") && (state.call == null || callMinimized), modifier = Modifier.align(Alignment.BottomCenter).zIndex(3f), enter = slideInVertically(motionPolicy.enter(MotionMillis, if(goingBack)MotionQuick else 0)) { it } + fadeIn(motionPolicy.enter(MotionMillis, if(goingBack)MotionQuick else 0)), exit = slideOutVertically(motionPolicy.exit(MotionQuick)) { it } + fadeOut(motionPolicy.exit(MotionExit)), label = "Main navigation") {
                                FloatingChrome(backdrop, Modifier.padding(bottom = navigationInset + 8.dp).widthIn(max = 920.dp).fillMaxWidth().padding(horizontal = 16.dp).testTag("main-navigation"), RoundedCornerShape(24.dp)) {
                                    Row(Modifier.fillMaxWidth().padding(8.dp), horizontalArrangement = Arrangement.SpaceEvenly) {
                                        listOf(Triple("inbox", "chat_bubble", "Messages"), Triple("calls", "call", "Calls"), Triple("notes", "description", "Notes"), Triple("settings", "settings", "Settings")).filter { it.first != "calls" || LocalClientFeatures.current.calls }.forEach { (tab, icon, label) ->
                                            NavigationIcon(icon, label, page == tab) { navigate(tab) }
                                        }
                                    }
                                }
                        }
                        val density = LocalDensity.current
                        androidx.compose.animation.AnimatedVisibility(conversation, Modifier.align(Alignment.BottomCenter).zIndex(3f),
                            enter = slideInVertically(motionPolicy.enter(MotionInline, delayMillis = MotionMillis)) { it } + fadeIn(motionPolicy.enter(MotionInline, delayMillis = MotionMillis)),
                            exit = slideOutVertically(motionPolicy.exit(MotionQuick)) { it } + fadeOut(motionPolicy.exit(MotionExit)), label = "Conversation footer") {
                        Box(Modifier.widthIn(max = 920.dp).fillMaxWidth().onSizeChanged { if (footer.content != null) footer.height = with(density) { it.height.toDp() } }.padding(horizontal = 12.dp).windowInsetsPadding(WindowInsets.ime.union(WindowInsets.navigationBars)).padding(bottom = 8.dp).zIndex(3f)) {
                            if (footer.content != null) FloatingChrome(backdrop, Modifier.testTag("conversation-footer").onGloballyPositioned { materialOcclusion.footer = it.boundsInWindow() }, RoundedCornerShape(24.dp)) { footer.content?.invoke() }
                        }
                        }
                        MaterialOverlayViewport(materialOverlayHost, Modifier.matchParentSize().zIndex(3.5f))
                        PresentationViewport(presentationHost, Modifier.matchParentSize().zIndex(5f))
                        var lastIssue by remember { mutableStateOf("") }
                        SideEffect { state.issue?.let { lastIssue = it } }
                        Column(Modifier.align(Alignment.TopCenter).padding(top = headerHeight + headerTop + 12.dp, start = 16.dp, end = 16.dp).widthIn(max = 620.dp).zIndex(8f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                            androidx.compose.animation.AnimatedVisibility(state.issue != null,
                                enter = slideInVertically(motionPolicy.enter(MotionInline)) { -it } + fadeIn(motionPolicy.enter(MotionInline)),
                                exit = fadeOut(motionPolicy.exit(MotionExit)), label = "Sync notice") {
                                DisposableEffect(materialOcclusion) { onDispose { materialOcclusion.notice = Rect.Zero } }
                                Box(Modifier.onGloballyPositioned { materialOcclusion.notice = it.boundsInWindow() }) { SyncNotice(lastIssue) { command("dismiss", emptyMap()) } }
                            }
                            androidx.compose.animation.AnimatedVisibility(accessNotice,
                                enter = fadeIn(motionPolicy.enter(MotionQuick)), exit = fadeOut(motionPolicy.exit(MotionExit)), label = "Account access notice") {
                                GlobalNotice("manage_accounts", "Your server’s sign-in is changing · Review") { command("close", emptyMap()); conversationPage = ""; navigate("profile") }
                            }
                            androidx.compose.animation.AnimatedVisibility(state.call != null && callMinimized,
                                enter = fadeIn(motionPolicy.enter(MotionQuick)), exit = fadeOut(motionPolicy.exit(MotionExit)), label = "Minimized call notice") {
                                GlobalNotice("call", "Return to call") { callMinimized = false }
                            }
                        }
                    }
                    }
                }
            }
            if (newCall) NewCallDialog(state, dispatch) { newCall = false }
            if (collectionSheet) CollectionSheet(state, selected, command, { collectionSheet = false; selected = emptySet() })
            if (actionSheet.isNotEmpty()) ConversationActionSheet(actionSheet, actionFields, state, command) { actionSheet = ""; selected = emptySet() }
        }
        }
        }
        }
        overlay()
      }
    }
}

@Composable
private fun GlobalNotice(icon: String, label: String, action: () -> Unit) {
    Surface(Modifier.fillMaxWidth().clip(RoundedCornerShape(20.dp)).clickable(role = Role.Button, onClick = action),
        shape = RoundedCornerShape(20.dp), color = MaterialTheme.colorScheme.surfaceContainerHigh, shadowElevation = 3.dp) {
        Row(Modifier.fillMaxWidth().heightIn(min = 56.dp).padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Glyph(icon, 20)
            Text(label, Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
            CompositionLocalProvider(LocalContentColor provides MaterialTheme.colorScheme.onSurfaceVariant) { Glyph("chevron_right", 20) }
        }
    }
}
