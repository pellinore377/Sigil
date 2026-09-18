@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
package org.sigil

import androidx.compose.animation.*
import androidx.compose.animation.core.*
import androidx.compose.foundation.*
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.awaitFirstDown
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
import androidx.compose.ui.graphics.drawscope.clipRect
import androidx.compose.ui.input.pointer.*
import androidx.compose.ui.layout.*
import androidx.compose.ui.platform.*
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.*
import androidx.compose.ui.window.*
import org.jetbrains.compose.resources.Font
import sigil.shared.generated.resources.*
import kotlinx.coroutines.launch
import kotlin.math.*

@Composable
fun Glyph(name: String, size: Int = 24, label: String? = null, filled: Boolean = false) {
    val glyphSize = with(LocalDensity.current) { size.dp.toSp() }
    Text(name, fontFamily = if (name.any { it.code > 127 }) null else FontFamily(Font(if (filled) Res.font.material_symbols else Res.font.material_symbols_outline)), fontSize = glyphSize, lineHeight = glyphSize, maxLines = 1,
        modifier = Modifier.clearAndSetSemantics { if (label != null) contentDescription = label })
}
internal fun showsReceipt(index: Int, messages: List<ChatMessage>) = index == 0 && messages.firstOrNull()?.mine == true
internal fun showSeparator(message: ChatMessage, older: ChatMessage?) = older == null || message.timestamp - older.timestamp >= 900
internal fun swipeAction(mine: Boolean, horizontal: Float) = if ((horizontal > 0) != mine) "reply" else "thread"
// A one-line bubble's height, and the gap the chip keeps from the bubble.
internal val SwipeChipSize = 46.dp
internal val SwipeChipGap = 8.dp
// True once a pull is clearly sideways, false once it is clearly a scroll, null while undecided.
internal fun swipeStarts(dx: Float, dy: Float, slop: Float): Boolean? = when {
    abs(dy) > slop && abs(dy) >= abs(dx) -> false
    abs(dx) > slop * 1.5f && abs(dx) > abs(dy) * 2f -> true
    else -> null
}

@Composable
internal fun ConversationHeader(chat: ChatSummary, page: String, threaded: Boolean, command: Command, back: () -> Unit, navigate: (String) -> Unit) {
    var menu by remember(chat.id) { mutableStateOf(false) }
    Row(Modifier.fillMaxSize().padding(horizontal = 8.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        Symbol("chevron_left", "Back", back)
        PresenceAvatar(chat, 42)
        Column(Modifier.weight(1f).padding(start = 10.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(if (threaded) "Thread" else page.ifEmpty { chat.name }, style = MaterialTheme.typography.titleLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (threaded || page.isNotEmpty()) Text(chat.name, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        if(LocalClientFeatures.current.calls && chat.id != "self") {
        Symbol("call", "Start audio call") { command("call_start", mapOf("peer" to chat.id, "video" to false)) }
        if(LocalClientFeatures.current.videoCalls)Symbol("videocam", "Start video call") { command("call_start", mapOf("peer" to chat.id, "video" to true)) }
        }
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
/// The list insets the bubbles, so the fraction is taken against the whole timeline.
private val TimelineGutter = 16.dp
private const val BubbleWidthFraction = .78f

@Composable
internal fun ConversationPage(chat: ChatSummary, state: MessengerState, draft: TextFieldState, analyze: (String) -> String, command: Command,
    page: String, gradient: Boolean, thread: ThreadTarget?, setThread: (ThreadTarget?) -> Unit) {
    val motionPolicy = LocalMotion.current
    var reply by remember(chat.id) { mutableStateOf<ChatMessage?>(null) }
    var editing by remember(chat.id) { mutableStateOf<ChatMessage?>(null) }
    var selected by remember(chat.id) { mutableStateOf<Pair<ChatMessage, Rect>?>(null) }
    var cardDetails by remember(chat.id) { mutableStateOf<ChatMessage?>(null) }
    // Each message keeps its own details open or closed, as the reference does.
    var details by remember(chat.id) { mutableStateOf(setOf<Pair<String, String>>()) }
    var submitted by remember { mutableStateOf<Triple<String, String, Long>?>(null) }
    var localQuery by remember(page) { mutableStateOf("") }
    val scheme = MaterialTheme.colorScheme
    // The window is asked on every measure, so the core's depth applies as soon as it answers.
    val buffer = rememberUpdatedState(state.timelineBuffer)
    val list = rememberLazyListState(remember { TimelineCacheWindow { buffer.value } })
    val scope = rememberCoroutineScope()
    val clipboard = LocalClipboardManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    val focus = LocalFocusManager.current
    LaunchedEffect(state.editDraft) { state.editDraft?.let { edit ->
        if (edit.peer == chat.id) state.messages.firstOrNull { it.id == edit.message && it.author == edit.author }?.let {
            editing = it; reply = null; setThread(null); draft.edit { replace(0, length, edit.source) }
        }
        command("edit_source_used", emptyMap())
    } }
    val atLatest by remember { derivedStateOf { list.firstVisibleItemIndex == 0 && list.firstVisibleItemScrollOffset < 80 } }
    LaunchedEffect(state.typing, state.messages.firstOrNull()?.id) { if (atLatest) { if (motionPolicy.reduced) list.scrollToItem(0) else list.animateScrollToItem(0) } }
    LaunchedEffect(chat.id, page, localQuery, thread?.id, thread?.author) {
        if (page == "Search") kotlinx.coroutines.delay(180)
        command("timeline_filter", mapOf("peer" to chat.id, "category" to page.takeIf { it in listOf("Notes", "Pins", "Threads", "Search") }.orEmpty().ifEmpty { "Timeline" },
            "query" to localQuery.takeIf { page == "Search" }, "thread_author" to thread?.author, "thread_message" to thread?.id))
    }
    val threadsOverview = page == "Threads" && thread == null
    val messages = if (threadsOverview) state.messages.filter { it.threadAuthor != null && it.threadMessage != null }.distinctBy { it.threadAuthor to it.threadMessage } else state.messages
    val keys=remember(messages) {messages.map {it.author+it.id}}
    val arrivals=remember(chat.id,page,thread,state.historical) {TimelineArrivals()}
    arrivals.update(keys,state.timelineLoaded,!state.historical && page.isEmpty())
    // Returning to the latest messages drops the place the reader held in history along with the history itself.
    val anchor=remember(chat.id,page,thread,state.historical) {TimelineAnchor()}
    // Watched outside composition: reading the layout while composing would rebuild the page on every scroll step.
    LaunchedEffect(list,chat.id,page) {
        // The key and the offset must come from the same item: content padding puts others in view before it.
        snapshotFlow {list.layoutInfo}.collect {info->
            anchor.record(info.visibleItemsInfo.firstOrNull {it.index==list.firstVisibleItemIndex}?.key,list.firstVisibleItemScrollOffset)
        }
    }
    // The core counts messages; the list's deepest index also counts the items ahead of them. Only a new depth
    // is worth a command, but a list that shrank is different history, so the mark it was taken against goes.
    LaunchedEffect(list,chat.id,page) {
        var reported=-1
        snapshotFlow {((list.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0)-TimelineLead).coerceAtLeast(0) to list.layoutInfo.totalItemsCount-TimelineLead}
            .collect {(end,held)->
                if(held<=reported)reported=-1
                if(end>reported) {reported=end;command("viewport",mapOf("peer" to chat.id,"end" to end))}
            }
    }
    LaunchedEffect(keys) {
        if(list.isScrollInProgress)return@LaunchedEffect
        anchor.settle(keys,TimelineLead)?.let {(index,offset)->list.scrollToItem(index,offset)}
    }
    val textMotion=remember(chat.id,page,thread,state.historical) {MotionLedger()}
    val animated=remember(messages) {messages.associate {it.author+it.id to it.messageMotionDuration()}.filterValues {it>0}}
    textMotion.update(keys,state.timelineLoaded,!state.historical && page.isEmpty(),animated.keys)
    val visibleKeys by remember {derivedStateOf {list.layoutInfo.visibleItemsInfo.map {it.key}.toSet()}}
    val materialTimeline=remember(chat.id,page,thread) {MaterialTimeline()}
    val previewLaunch = remember(chat.id, page, thread) { PreviewLaunch() }
    LaunchedEffect(state.sent, previewLaunch.activeSource) {
        submitted?.takeIf { state.sent > it.third && state.sentText == it.second && !previewLaunch.holding(it.second) }?.let {
            if (draft.text.toString() == it.first) draft.clearText()
            submitted = null; reply = null; editing = null
        }
    }
    SideEffect { state.sentMessage?.let { previewLaunch.bind(state.sentText.orEmpty(), it, state.sent); previewLaunch.bindPanel(state.sentText.orEmpty(), it, state.sent) } }
    LaunchedEffect(previewLaunch.activeMessage) {
        val message = previewLaunch.activeMessage ?: return@LaunchedEffect
        kotlinx.coroutines.delay(5000)
        if (previewLaunch.activeMessage == message) previewLaunch.cancel()
    }
    LaunchedEffect(state.issue) { if (state.issue != null) previewLaunch.cancel() }
    val materialOverlay=LocalMaterialOverlay.current
    val canFly = !motionPolicy.reduced && LocalMotionVisible.current && !state.historical && page.isEmpty()
    val canLaunch = materialOverlay != null && canFly
    MaterialRootOverlay(materialTimeline, materialOverlay, selected == null)
    val composerInset = (LocalFooterHost.current?.height ?: 0.dp) + 16.dp
    val materialHeader=with(LocalDensity.current){LocalHeaderInset.current.toPx()}
    BackAction(thread != null) { setThread(null); if (state.historical) command("latest", emptyMap()) }
    fun respond(message: ChatMessage, threaded: Boolean) {
        if (threaded) { setThread(ThreadTarget(message.threadAuthor ?: message.author, message.threadMessage ?: message.id)); reply = null } else reply = message
        selected = null
    }
    val pageMotion = LocalPageMotion.current
    val timelineMotion = if (pageMotion == null) Modifier else with(pageMotion) { Modifier.animateEnterExit(
        enter = slideInVertically(motionPolicy.enter(MotionMillis)) { it },
        exit = slideOutVertically(motionPolicy.exit(MotionMillis, if(LocalNavigationBack.current)MotionQuick else 0)) { it }) }
    // The menu blurs the page behind it where the platform can; elsewhere the scrim alone dims it.
    // The menu's progress: the page's other items blur by it, the scrim dims by it, the pill and actions scale by it.
    val menu = remember { Animatable(0f) }
    // The held bubble itself, moved out of its list item into the menu's slot while the menu is open, and back when it closes.
    val heldContent = remember { mutableStateOf<(@Composable () -> Unit)?>(null) }
    var pageBounds by remember { mutableStateOf(Rect.Zero) }
    val footerHost = LocalFooterHost.current
    val navigationInset = WindowInsets.navigationBars.getBottom(LocalDensity.current)
    Box(Modifier.fillMaxSize().then(timelineMotion).background(scheme.background)
        .onGloballyPositioned { pageBounds = it.boundsInWindow() }.testTag("conversation-page")) {
        LocalWallpaper.current(chat.id, Modifier.matchParentSize())
        Column(Modifier.align(Alignment.TopCenter).widthIn(max = 920.dp).fillMaxSize().then(if (gradient) Modifier.background(Brush.verticalGradient(listOf(scheme.background.copy(alpha = .7f), scheme.primaryContainer.copy(alpha = .7f)))) else Modifier)) {
            val headerInset = LocalHeaderInset.current
            val banner = !chat.group && (!chat.verified || chat.request == "incoming")
            val controls = page == "Search" || state.historical
            Column(Modifier.weight(1f).fillMaxWidth().testTag("timeline-body")) {
            if (controls) Spacer(Modifier.height(headerInset))
                if (page == "Search") OutlinedTextField(localQuery, { localQuery = it }, Modifier.fillMaxWidth().padding(12.dp), placeholder = { Text("Search this conversation") }, singleLine = true)
            if (state.historical) SigilTextButton({ command("latest", emptyMap()) }, Modifier.align(Alignment.CenterHorizontally)) { Text("Return to latest messages") }
            Box(Modifier.weight(1f).fillMaxWidth().clipToBounds().onGloballyPositioned {materialTimeline.viewport=it.boundsInWindow();val r=materialTimeline.viewport;if(materialHeader>0)materialTimeline.bubbles["header"]=Rect(r.left,r.top,r.right,r.top+materialHeader)}) {
            val timelineMedia = remember(messages) { messages.filter { m -> m.attachment?.let { it.mediaType.startsWith("image/") || it.mediaType.startsWith("video/") } == true }.asReversed() }
            CompositionLocalProvider(LocalMaterialTimeline provides materialTimeline.takeIf {materialOverlay!=null}, LocalPreviewLaunch provides previewLaunch, LocalTimelineMedia provides timelineMedia) {
            LazyColumn(Modifier.fillMaxSize().blur(18.dp * menu.value).testTag("timeline"), state = list, reverseLayout = true, userScrollEnabled = selected == null, contentPadding = PaddingValues(start = TimelineGutter, end = TimelineGutter, top = (if (controls) 0.dp else headerInset) + 12.dp, bottom = composerInset)) {
                item("typing") { androidx.compose.animation.AnimatedVisibility(!threadsOverview && state.typing.isNotEmpty(), enter = expandVertically(motionPolicy.enter(MotionMillis)) + fadeIn(motionPolicy.enter(MotionMillis)), exit = shrinkVertically(motionPolicy.exit(MotionQuick)) + fadeOut(motionPolicy.exit(MotionExit)), label = "Typing indicator") { TypingRow(state.typing.map { state.people[it] ?: if (chat.group) "Member" else chat.name }, chat.name, state.typing) } }
                itemsIndexed(messages, key = { _, it -> it.author + it.id }) { index, message ->
                    if (page == "Pins") {
                        var pinBounds by remember(message.id) { mutableStateOf(Rect.Zero) }
                        Surface(itemMotion().fillMaxWidth().padding(vertical = 6.dp).onGloballyPositioned { pinBounds = it.boundsInWindow() }
                            .clip(RoundedCornerShape(20.dp)).combinedClickable(onClick = { val key = message.author to message.id; details = if (key in details) details - key else details + key }, onLongClick = { selected = message to pinBounds }),
                            shape = RoundedCornerShape(20.dp), color = scheme.surfaceContainerHigh) {
                            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                                Row(verticalAlignment = Alignment.CenterVertically) {
                                    Avatar(state.people[message.author] ?: if (message.mine) "You" else chat.name, 28, message.author)
                                    Column(Modifier.weight(1f).padding(start = 8.dp), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                                        Text(state.people[message.author] ?: if (message.mine) "You" else chat.name, style = MaterialTheme.typography.labelLarge)
                                        Text(message.time, style = MaterialTheme.typography.labelSmall, color = scheme.onSurfaceVariant)
                                    }
                                    Symbol("push_pin", "Unpin message") { command("pin", mapOf("peer" to chat.id, "author" to message.author, "message" to message.id, "active" to false)) }
                                }
                                MessageBubble(message.copy(pinned = false), false, false, analyze, if (chat.verified && !state.busy) command else null)
                            }
                        }
                        return@itemsIndexed
                    } else if (threadsOverview) {
                        Surface(itemMotion().fillMaxWidth().padding(vertical = 6.dp).clip(RoundedCornerShape(20.dp)).clickable { setThread(ThreadTarget(message.threadAuthor!!, message.threadMessage!!)) }, shape = RoundedCornerShape(20.dp), color = scheme.surfaceContainerHigh) {
                            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                                Text(message.threadPreview ?: "Earlier message", maxLines = 3, overflow = TextOverflow.Ellipsis)
                                Row(verticalAlignment = Alignment.CenterVertically) { Glyph("forum", 18); Spacer(Modifier.width(8.dp)); Text(message.text, Modifier.weight(1f), maxLines = 2, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodySmall); Glyph("chevron_right", 20) }
                            }
                        }
                        return@itemsIndexed
                    }
                    val older = messages.getOrNull(index + 1)
                    val newer = messages.getOrNull(index - 1)
                    val grouped = older?.author == message.author && !showSeparator(message, older)
                    // The same test in both directions, or a separator below still tightens the corner above it.
                    val followed = newer != null && newer.author == message.author && !showSeparator(newer, message)
                    var bounds by remember { mutableStateOf(Rect.Zero) }
                    // Window bounds clip at the screen edge; the chip anchors on the bubble's own measured width.
                    var bubblePx by remember { mutableIntStateOf(0) }
                    val materialKey=message.author+message.id
                    // Derived per item, so a scroll invalidates only the rows whose own visibility changed,
                    // not every row the buffer holds composed.
                    val onScreen by remember(materialKey) {derivedStateOf {materialKey in visibleKeys}}
                    DisposableEffect(materialKey) {onDispose {materialTimeline.bubbles.remove(materialKey)}}
                    var drag by remember { mutableFloatStateOf(0f) }
                    var dragging by remember { mutableStateOf(false) }
                    // Under the finger the bubble follows exactly; released, it springs home.
                    val settle = remember { Animatable(0f) }
                    val offset = if (dragging) drag else settle.value
                    val density = LocalDensity.current
                    val haptic = LocalHapticFeedback.current
                    val reveal = with(density) { (SwipeChipSize + SwipeChipGap).toPx() }
                    val threshold = reveal
                    val held = selected?.first?.let { it.id == message.id && it.author == message.author } == true
                    val cmd = if (chat.verified && !state.busy) command else null
                    // One instance of the bubble: it renders here, or in the menu's slot, never in both.
                    val bubbleContent = remember(materialKey) { movableContentOf<ChatMessage, Boolean, Boolean, Command?> { m, g, f, c -> MessageBubble(m, g, f, analyze, c) } }
                    var bubbleSize by remember { mutableStateOf(IntSize.Zero) }
                    fun hold() { heldContent.value = { bubbleContent(message, grouped, followed, cmd) }; selected = message to bounds }
                    Column(itemMotion().then(arrivalMotion(arrivals,materialKey)).fillMaxWidth().padding(top = if (grouped) 3.dp else 12.dp)) {
                        if (showSeparator(message, older)) Text(message.separator.ifEmpty { message.time }, Modifier.align(Alignment.CenterHorizontally).padding(top = 6.dp, bottom = 14.dp), style = MaterialTheme.typography.labelMedium, color = scheme.onSurfaceVariant)
                        if (chat.group && !message.mine && !grouped) Row(Modifier.padding(bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) { val name = state.people[message.author] ?: "Former member"; Avatar(name, 20, message.author); Text(name, Modifier.padding(start = 6.dp), style = MaterialTheme.typography.bodySmall, color = scheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                        Row(Modifier.fillMaxWidth().combinedClickable(interactionSource = remember { androidx.compose.foundation.interaction.MutableInteractionSource() }, indication = null, onClick = { val key = message.author to message.id; details = if (key in details) details - key else details + key }, onLongClick = { hold() })
                            .pointerInput(message.id, message.mine) {
                                val slop = viewConfiguration.touchSlop
                                // The bubble never travels further than the chip needs, so it stays on screen.
                                val limit = reveal + with(density) { 16.dp.toPx() }
                                awaitEachGesture {
                                    val down = awaitFirstDown(requireUnconsumed = false)
                                    if (list.isScrollInProgress) return@awaitEachGesture
                                    var dx = 0f; var dy = 0f; var started = false
                                    while (true) {
                                        val change = awaitPointerEvent().changes.firstOrNull { it.id == down.id } ?: break
                                        if (!change.pressed) break
                                        val delta = change.positionChange()
                                        if (!started) {
                                            if (change.isConsumed) break
                                            dx += delta.x; dy += delta.y
                                            when (swipeStarts(dx, dy, slop)) { false -> break; true -> { started = true; dragging = true; drag = 0f }; null -> continue }
                                        }
                                        change.consume()
                                        val before = drag
                                        // Past the chip the bubble resists, so a long pull stays near the chip.
                                        drag = (drag + delta.x * if (abs(drag) >= threshold) .35f else 1f).coerceIn(-limit, limit)
                                        if (abs(before) < threshold && abs(drag) >= threshold) haptic.performHapticFeedback(androidx.compose.ui.hapticfeedback.HapticFeedbackType.LongPress)
                                    }
                                    if (!started) return@awaitEachGesture
                                    val release = drag
                                    val action = if (abs(release) >= threshold) swipeAction(message.mine, release) else null
                                    if (action == "thread") {
                                        // The bubble lands at rest first, so the thread's placement motion starts from a whole bubble, never a clipped one.
                                        scope.launch { settle.snapTo(0f); dragging = false; drag = 0f; respond(message, true) }
                                    } else {
                                        if (action == "reply") respond(message, false)
                                        // Hand over at the release point, never through zero, so the spring starts where the finger left.
                                        scope.launch { settle.snapTo(release); dragging = false; drag = 0f; settle.animateTo(0f, if (motionPolicy.reduced) snap() else spring(dampingRatio = .82f, stiffness = Spring.StiffnessMediumLow)) }
                                    }
                                }
                            },
                            horizontalArrangement = if (message.mine) Arrangement.End else Arrangement.Start) {
                            BoxWithConstraints(Modifier.weight(1f, false)) {
                            val bubbleWidth = ((maxWidth + TimelineGutter * 2) * BubbleWidthFraction).coerceAtMost(maxWidth)
                            Column(Modifier.width(bubbleWidth), horizontalAlignment = if (message.mine) Alignment.End else Alignment.Start) {
                                val launched = previewLaunch.panelOrigin(message.id).takeIf { message.mine }
                                val arrival = remember(message.id) { Animatable(1f) }
                                LaunchedEffect(launched) {
                                    if (launched == null) return@LaunchedEffect
                                    arrival.snapTo(0f)
                                    arrival.animateTo(1f, motionPolicy.tween(MotionMillis))
                                    previewLaunch.landed(message.id)
                                }
                                Box(Modifier.fillMaxWidth(), contentAlignment = if (message.mine) Alignment.CenterEnd else Alignment.CenterStart) {
                                  if (abs(offset) > 1f && !held) {
                                    val action = swipeAction(message.mine, offset)
                                    val armed = abs(offset) >= threshold
                                    // The chip rides beside the bubble's own edge, one gap away, in either direction.
                                    val slack = with(density) { bubbleWidth.toPx() } - bubblePx
                                    val base = if (offset > 0) (if (message.mine) slack else 0f) else (if (message.mine) 0f else -slack)
                                    Surface(Modifier.align(if (offset > 0) Alignment.CenterStart else Alignment.CenterEnd).size(SwipeChipSize)
                                        .graphicsLayer { translationX = base + offset + if (offset > 0) -reveal else reveal; alpha = (abs(offset) / reveal).coerceIn(0f, 1f) },
                                        shape = RoundedCornerShape(16.dp), color = if (armed) scheme.primary else scheme.surfaceContainerHigh, contentColor = if (armed) scheme.onPrimary else scheme.onSurface) {
                                        Box(contentAlignment = Alignment.Center) { Glyph(if (action == "reply") "reply" else "forum", 22, if (action == "reply") "Reply" else "Reply in thread") }
                                    }
                                  }
                                  Box(Modifier.graphicsLayer {
                                    translationX = offset
                                    // The sent card carries on from where the preview panel left it, rather than arriving from nowhere.
                                    val from = launched
                                    if (from != null && bounds != Rect.Zero && bounds.width > 0f) {
                                        val remaining = 1f - arrival.value
                                        transformOrigin = TransformOrigin(0f, 0f)
                                        scaleX = 1f + ((from.width / bounds.width).coerceIn(.5f, 2f) - 1f) * remaining
                                        scaleY = scaleX
                                        translationX += (from.left - bounds.left) * remaining
                                        translationY = (from.top - bounds.top) * remaining
                                    }
                                  }.onSizeChanged { bubblePx = it.width; bubbleSize = it }.onGloballyPositioned { bounds = it.boundsInWindow(); materialTimeline.bubbles[materialKey]=bounds }
                                    .pointerInput(message.id) { awaitPointerEventScope { while (true) { val event = awaitPointerEvent(); if (event.type == PointerEventType.Press && event.buttons.isSecondaryPressed) hold() } } }) {
                                    CompositionLocalProvider(LocalMaterialPress provides { hold() },LocalItemVisible provides onScreen) {
                                    if (held) Spacer(Modifier.size(with(density) { bubbleSize.width.toDp() }, with(density) { bubbleSize.height.toDp() }))
                                    else if(materialKey in animated) MessageMotion(message.id,textMotion.state(materialKey),onScreen && selected==null,animated.getValue(materialKey)) {
                                        bubbleContent(message, grouped, followed, cmd)
                                    } else bubbleContent(message, grouped, followed, cmd)
                                    }
                                  }
                                }
                                MessageDetails(message, (message.author to message.id) in details, !state.historical && page.isEmpty() && thread == null && showsReceipt(index, messages), chat, state.people)
                            }
                        }
                            }
                    }
                    // Buffered items are composed before they are seen; only a message on screen is read.
                    if (!message.mine && !message.readByMe && chat.verified && selected == null && onScreen) LaunchedEffect(message.id) {
                        command("read", mapOf("peer" to chat.id, "author" to message.author, "message" to message.id))
                    }
                }
            }
            }
            if(selected==null)materialOverlay?.invoke(materialTimeline,Modifier.matchParentSize())
            }
            if (banner) Column(Modifier.padding(bottom = composerInset)) { ContactRequestPanel(chat, state.busy, command) }
            }
            FooterContent {
            CompositionLocalProvider(LocalPreviewLaunch provides previewLaunch) {
            Column(Modifier.fillMaxWidth()) {
            val context = editing?.let { "Editing" to it.text } ?: reply?.let { (state.people[it.author] ?: if (it.mine) "You" else chat.name) to it.text }
            context?.let { (title, text) -> ContextChip(title, text, reply?.attachment, reply, reply?.let { cardQuote(it.parts) }) { reply = null; editing = null } }
            val inputCommand: Command = { action, fields ->
                command(action, if (action in listOf("attachment_pick", "record_start")) fields + mapOf("reply_author" to reply?.author, "reply_message" to reply?.id, "thread_author" to thread?.author, "thread_message" to thread?.id) else fields)
            }
            if (!threadsOverview) ComposerPanel(draft, analyze, chat.verified && !state.busy, page == "Notes", inputCommand, chat.id, state.voice, state.sent, state.sentText, requestContact = if (!chat.verified && !chat.group && !state.busy && chat.request in listOf("none", "expired")) ({ command("contact_request", mapOf("peer" to chat.id, "action" to "send")) }) else null, attachments = state.transfers.filter { it.peer == chat.id }, editingCaption = editing?.attachment != null, attachmentTarget = mapOf("peer" to chat.id, "reply_author" to reply?.author, "reply_message" to reply?.id, "thread_author" to thread?.author, "thread_message" to thread?.id)) { text, rich, timezone ->
                submitted = Triple(draft.text.toString(), text, state.sent)
                if (editing != null) command("edit", mapOf("peer" to chat.id, "author" to editing!!.author, "message" to editing!!.id, "text" to text, "formatted" to true))
                else { if (canLaunch) previewLaunch.arm(text, state.sent); if (canFly) previewLaunch.armPanel(text, state.sent); command("post", mapOf("peer" to chat.id, "text" to text, "rich" to rich, "formatted" to true, "timezone" to timezone,
                    "reply_author" to reply?.author, "reply_message" to reply?.id, "thread_author" to thread?.author, "thread_message" to thread?.id)) }
            }
        }
        }
        }
        }
        selected?.let { (message, origin) ->
            val band = Rect(pageBounds.left, footerHost?.headerBottom ?: pageBounds.top, pageBounds.right, pageBounds.bottom - navigationInset - with(LocalDensity.current) { ((footerHost?.height ?: 0.dp) + 16.dp).toPx() })
            MessageMenu(message, origin, menu, pageBounds, band, { CompositionLocalProvider(LocalMaterialTimeline provides materialTimeline.takeIf { materialOverlay != null }) { heldContent.value?.invoke() } },
                { materialTimeline.bubbles[message.author + message.id] = it }, { selected = null; heldContent.value = null }) { action, value ->
                when (action) {
                    "reply" -> respond(message, false)
                    "thread" -> respond(message, true)
                    "copy" -> { clipboard.setText(AnnotatedString(message.text)); selected = null }
                    "edit" -> { command("edit_source", mapOf("peer" to chat.id, "author" to message.author, "message" to message.id)); selected = null }
                    "replay" -> {textMotion.state(message.author+message.id).replay();selected=null}
                    "details" -> { cardDetails = message; selected = null }
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
        cardDetails?.let { CardDetails(it) { cardDetails = null } }
    }
}
private val BubbleCueSpread = 12.dp

@Composable
internal fun MessageBubble(message: ChatMessage, grouped: Boolean, followed: Boolean, analyze: (String) -> String, command: Command? = null) {
    val scheme = MaterialTheme.colorScheme
    val outgoing = LocalOutgoingBubble.current
    val outgoingInk = LocalOutgoingInk.current
    val objectOnly=message.bareRandomizers()
    val bareImage=message.attachment?.mediaType?.let {it.startsWith("image/") || it.startsWith("video/")} == true
    val bareLocation=message.attachment==null && message.parts.any {it.kind=="location"} && message.parts.all {it.kind in listOf("location","text")}
    // A captioned picture is one bubble: the picture flush at the top, the caption on the ground beneath it.
    val captioned=bareImage && message.attachment?.caption?.isNotBlank()==true
    // A fenced block is its own bubble, so the message frame steps aside and the chunks group themselves.
    val panelled=message.attachment==null && message.parts.any {p->p.rich?.let {visibleCodeBlocks(it).isNotEmpty()} == true}
    val emoji = remember(message.text, message.kind, message.reply, message.parts) { if (message.kind == "Text" && message.reply == null && message.parts.all { it.kind == "text" && it.rich?.spans.orEmpty().isEmpty() }) animatedEmoji(message.text) else null }
    val bubbleShape = RoundedCornerShape(topStart = if (!message.mine && grouped) 5.dp else 20.dp, topEnd = if (message.mine && grouped) 5.dp else 20.dp,
        bottomStart = if (!message.mine && followed) 5.dp else 20.dp, bottomEnd = if (message.mine && followed) 5.dp else 20.dp)
    // A card's end cue is drawn here, outside the bubble's clip, tracing the bubble's own outline.
    val cue = remember { mutableFloatStateOf(1f) }
    val cueInk = scheme.onBackground
    Box(Modifier.padding(top = if (message.reactions.isNotEmpty() || message.pinned) 8.dp else 0.dp)) {
        if (emoji != null) EmojiMessage(emoji)
        else
        {
            val bare = objectOnly || (bareImage && !captioned) || bareLocation
            // A quoted bubble: the quote inset 6dp on the incoming tone; an own reply then carries its text as an outgoing band.
            val quoted = message.reply != null && message.attachment == null && !objectOnly && !bareLocation && !panelled
            val ground = if (message.mine && !quoted) outgoing else scheme.surfaceContainer
            // Content that covers the bubble paints nothing underneath; captions and text chunks paint the ground themselves.
            val filled = panelled || captioned || message.attachment?.let { a -> !a.mediaType.startsWith("image/") && !a.mediaType.startsWith("video/") && a.name != "Voice message.aac" && attachmentKind(a.name, a.mediaType) != AttachmentKind.File } == true
            // Bare objects overhang their slot on purpose, so they get no clipping surface at all.
            val frame: @Composable (@Composable () -> Unit) -> Unit = { body -> if (objectOnly) Box { body() } else Surface(Modifier.drawBehind {
                if (cue.floatValue >= 1f) return@drawBehind
                val corners = floatArrayOf(bubbleShape.topStart.toPx(size, this), bubbleShape.topEnd.toPx(size, this), bubbleShape.bottomEnd.toPx(size, this), bubbleShape.bottomStart.toPx(size, this))
                drawEndCue(cue.floatValue, cueInk, BubbleCueSpread.toPx(), BubbleCueSpread.toPx(), corners)
            }, shape = bubbleShape,
            color = if(bare || filled) Color.Transparent else ground, contentColor = if(bare) scheme.onBackground else if (message.mine && !quoted) outgoingInk else scheme.onSurface) { body() } }
            frame {
            CompositionLocalProvider(LocalBubbleCue provides cue,LocalBubbleGround provides ground,LocalMessageKey provides message.author+message.id,LocalMessageBubble provides panelled,LocalMaterialOutgoing provides message.mine,LocalContentColor provides if (bare) scheme.onBackground else if (message.mine) outgoingInk else scheme.onSurface,LocalMessageSurface provides if (bare) scheme.background else if (message.mine && !quoted) outgoing else scheme.surfaceContainer) {
            Column(if (message.attachment == null && !quoted) Modifier.padding(horizontal = if(objectOnly || bareLocation || panelled)0.dp else 14.dp, vertical = if(bareLocation || panelled)0.dp else 10.dp) else Modifier) {
                // The quoted block is the timeline ground set into the bubble.
                val body: @Composable () -> Unit = { if (message.attachment != null) LocalAttachmentContent.current(message) else if (message.parts.isNotEmpty()) MessageCards(message, analyze, command, objectOnly || bareLocation) else MessageText(message.text, analyze) }
                if (quoted) {
                    Box(Modifier.padding(start = 6.dp, end = 6.dp, top = 6.dp, bottom = if (message.mine) 6.dp else 0.dp)) { ReplyQuote(message, if (!message.mine && grouped) 5.dp else 14.dp, if (message.mine && grouped) 5.dp else 14.dp) }
                    if (message.mine) Surface(Modifier.fillMaxWidth(), color = outgoing, contentColor = outgoingInk) {
                        CompositionLocalProvider(LocalMessageSurface provides outgoing) { Box(Modifier.padding(horizontal = 14.dp, vertical = 10.dp)) { body() } }
                    } else Box(Modifier.padding(start = 14.dp, end = 14.dp, top = 6.dp, bottom = 10.dp)) { body() }
                } else if (message.reply != null) { ReplyQuote(message); Spacer(Modifier.height(6.dp)); body() } else body()
                if (!captioned && !filled) message.attachment?.caption?.takeIf { it.isNotEmpty() }?.let { caption ->
                    Box(Modifier.padding(horizontal=14.dp,vertical=10.dp)) { MessageText(caption,analyze) }
                }

            }
            }
            }
        }
        if (message.reactions.isNotEmpty()) Text(message.reactions.distinct().joinToString(""), Modifier.align(if (message.mine) Alignment.TopStart else Alignment.TopEnd).offset(y = (-10).dp), fontSize = 20.sp)
        if (message.pinned) Box(Modifier.align(if (message.mine) Alignment.TopEnd else Alignment.TopStart).offset(y = (-8).dp).background(scheme.background, CircleShape).padding(2.dp)) { Glyph("push_pin", 13, "Pinned message", filled=true) }
    }
}
@Composable
internal fun MessageDetails(message: ChatMessage, expanded: Boolean, receipt: Boolean, chat: ChatSummary, people: Map<String, String>) {
    val motionPolicy = LocalMotion.current
    // The delivery state of an own message, then time and lock; the last own message always carries its state.
    val shown = receipt || expanded
    Row(Modifier.animateContentSize(motionPolicy.tween(MotionMillis)).padding(top = if (shown) 4.dp else 0.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(5.dp)) {
        if (receipt || expanded && message.mine) DeliveryReceipt(message, chat, people)
        // Slides in from the right and back out to the right.
        AnimatedVisibility(expanded, enter = fadeIn(motionPolicy.enter(MotionInline)) + expandHorizontally(motionPolicy.enter(MotionMillis), expandFrom = Alignment.End), exit = fadeOut(motionPolicy.exit(MotionMillis)) + shrinkHorizontally(motionPolicy.exit(MotionMillis), shrinkTowards = Alignment.End), label = "Message details") {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                if (receipt || message.mine) Text("·", style = MaterialTheme.typography.labelSmall)
                Text(message.time, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Glyph("lock", 11, "Encrypted message")
            }
        }
    }
}
@Composable
private fun DeliveryReceipt(message: ChatMessage, chat: ChatSummary, people: Map<String, String>) {
    val motionPolicy = LocalMotion.current
    Crossfade(if (message.delivery in listOf("Queued", "Sending")) "Sending" else message.delivery, animationSpec = motionPolicy.tween(MotionInline), label = "Delivery state") { stage ->
    if (stage == "Read") AvatarStack(message.readers.map { people[it] ?: if (chat.group) "Member" else chat.name }.ifEmpty { listOf(chat.name) }, 17, message.readers.ifEmpty { listOf(chat.avatar) })
    else if (stage == "Sending") {
        val angle = if (motionPolicy.reduced) 0f else {
            val transition = rememberInfiniteTransition(label = "Sending")
            transition.animateFloat(0f, 360f, motionPolicy.loop(), label = "Sending dots").value
        }
        val color = MaterialTheme.colorScheme.onSurfaceVariant
        Canvas(Modifier.size(17.dp).semantics { contentDescription = "Sending" }) {
            repeat(8) { index -> val r = (index * 45f + angle) * PI / 180; drawCircle(color, 1.dp.toPx(), Offset(center.x + cos(r).toFloat() * size.width * .36f, center.y + sin(r).toFloat() * size.height * .36f)) }
        }
    } else Surface(Modifier.size(17.dp).semantics { contentDescription = stage }, shape = CircleShape,
        color = if (stage in listOf("Failed", "Expired", "Cancelled")) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary,
        contentColor = if (stage in listOf("Failed", "Expired", "Cancelled")) MaterialTheme.colorScheme.onError else MaterialTheme.colorScheme.onPrimary) { Box(contentAlignment = Alignment.Center) { Glyph(if (stage in listOf("Failed", "Expired", "Cancelled")) "priority_high" else "check", 12) } }
    }
}
@Composable
internal fun AvatarStack(people: List<String>, size: Int = 22, photos: List<String> = emptyList()) {
    Box(Modifier.width((size + (people.take(5).size - 1).coerceAtLeast(0) * size * .7f).dp).height(size.dp)) {
        people.take(5).forEachIndexed { i, name -> Box(Modifier.offset(x = (i * size * .7f).dp).border(1.dp, MaterialTheme.colorScheme.background, CircleShape)) { Avatar(name, size, photos.getOrElse(i) { "" }) } }
    }
}
@Composable
private fun TypingRow(people: List<String>, name: String, photos: List<String>) {
    val motionPolicy = LocalMotion.current
    val phase = if (motionPolicy.reduced) 0f else {
        val animation = rememberInfiniteTransition(label = "Typing")
        animation.animateFloat(0f, 2f * PI.toFloat(), motionPolicy.loop(), label = "Typing dots").value
    }
    Row(Modifier.padding(top = 8.dp, bottom = 4.dp).semantics { contentDescription = "$name is typing" }, verticalAlignment = Alignment.CenterVertically) {
        AvatarStack(people, 22, photos); Spacer(Modifier.width(10.dp))
        repeat(3) { i -> Box(Modifier.padding(horizontal = 3.dp).offset(y = (-3 * max(0f, sin(phase - i * .8f))).dp).size(5.dp).background(MaterialTheme.colorScheme.onSurfaceVariant, CircleShape)) }
    }
}
@Composable
private fun BoxScope.MessageMenu(message: ChatMessage, origin: Rect, progress: Animatable<Float, AnimationVector1D>, page: Rect, band: Rect, content: @Composable () -> Unit, positioned: (Rect) -> Unit, dismiss: () -> Unit, action: (String, String) -> Unit) {
    val motionPolicy = LocalMotion.current
    val scheme = MaterialTheme.colorScheme
    val scope = rememberCoroutineScope()
    var emojiPicker by remember { mutableStateOf(false) }
    var emoji by remember { mutableStateOf("") }
    var closing by remember { mutableStateOf(false) }
    // The bubble's travel from where it was pressed to its slot in the sandwich, planned once; it opens as it travels and reverses as it returns.
    var plan by remember { mutableStateOf<Float?>(null) }
    LaunchedEffect(plan) { if (plan != null) progress.animateTo(1f, motionPolicy.tween(MotionMillis)) }
    fun finish(after: () -> Unit = {}) {
        if (closing) return
        closing = true
        scope.launch { progress.animateTo(0f, motionPolicy.tween(MotionMillis)); dismiss(); after() }
    }
    fun choose(name: String, value: String) { finish { action(name, value) } }
    BackAction(true) { finish() }
    val local = origin.translate(-page.left, -page.top)
    val enter = Modifier.graphicsLayer {
        alpha = progress.value; val scale = .92f + .08f * progress.value; scaleX = scale; scaleY = scale
        transformOrigin = TransformOrigin(if (message.mine) 1f else 0f, .5f)
    }
    Box(Modifier.matchParentSize().pointerInput(Unit) { detectTapGestures { finish() } }.background(scheme.scrim.copy(alpha = .5f * progress.value))) {
        Layout({
            Surface(enter, shape = RoundedCornerShape(28.dp), color = scheme.surfaceContainerHigh) {
                Row(Modifier.padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                    listOf("👍", "❤️", "😂", "😮", "😢", "😡").forEach { e -> SigilTextButton({ choose("react", e) }, contentPadding = PaddingValues(horizontal = 6.dp)) { Text(e, fontSize = 22.sp) } }
                    Symbol("add_reaction", "Choose reaction") { emojiPicker = true }
                }
            }
            Box(Modifier.onGloballyPositioned { positioned(it.boundsInWindow()) }) { content() }
            // As wide as its longest label, with the same inset either side.
            Surface(enter.width(IntrinsicSize.Max), shape = RoundedCornerShape(24.dp), color = scheme.surfaceContainerHigh) {
                Column(Modifier.padding(horizontal = 8.dp, vertical = 6.dp)) {
                    val entries = listOf("reply" to "Reply", "forward" to "Forward", "copy" to "Copy", "thread" to "Reply in thread", "pin" to if (message.pinned) "Unpin" else "Pin", "note" to if (message.noted) "Remove from notes" else "Add to notes") +
                        (if(message.detailsPart()!=null)listOf("details" to "Details") else emptyList()) +
                        (if(message.hasMessageMotion())listOf("replay" to "Replay animation") else emptyList()) +
                        (if (message.mine && message.editable) listOf("edit" to "Edit") else emptyList()) + (if (message.mine) listOf("delete" to "Delete") else emptyList())
                    entries.forEach { (key, label) -> DropdownMenuItem({ Text(label, color = if (key == "delete") scheme.error else scheme.onSurface) }, { choose(key, "") },
                        leadingIcon = { Glyph(when(key) { "thread" -> "forum"; "pin" -> "push_pin"; "note" -> "description"; "copy" -> "content_copy"; "details" -> "info"; else -> key }, 20) }) }
                }
            }
        }) { measurables, constraints ->
            val gap = 10.dp.roundToPx()
            val margin = 16.dp.roundToPx()
            val loose = constraints.copy(minWidth = 0, minHeight = 0, maxWidth = constraints.maxWidth - margin * 2)
            val pill = measurables[0].measure(loose)
            // The bubble keeps the width it had in the list.
            val slot = measurables[1].measure(Constraints(maxWidth = local.width.roundToInt().coerceAtLeast(1)))
            val actions = measurables[2].measure(loose)
            val left = local.left.roundToInt(); val right = local.right.roundToInt()
            fun x(width: Int) = (if (message.mine) right - width else left).coerceIn(margin, (constraints.maxWidth - margin - width).coerceAtLeast(margin))
            if (plan == null) {
                // The sandwich sits centred in the band; the bubble travels from its spot to its slot.
                val top = (band.top - page.top).roundToInt() + margin
                val bottom = (band.bottom - page.top).roundToInt() - margin
                val group = pill.height + gap + slot.height + gap + actions.height
                val groupTop = (top + (bottom - top - group) / 2).coerceIn(top, (bottom - group).coerceAtLeast(top))
                plan = (groupTop + pill.height + gap - local.top).toFloat()
            }
            val slotTop = (local.top + (plan ?: 0f) * progress.value).roundToInt()
            layout(constraints.maxWidth, constraints.maxHeight) {
                pill.placeRelative(x(pill.width), slotTop - gap - pill.height)
                slot.placeRelative(left, slotTop)
                actions.placeRelative(x(actions.width), slotTop + slot.height + gap)
            }
        }
    }
    if (emojiPicker) AlertDialog({ emojiPicker = false }, title = { Text("React with an emoji") }, text = { OutlinedTextField(emoji, { emoji = it }, label = { Text("Emoji") }, singleLine = true) }, confirmButton = { SigilTextButton({ if (emoji.isNotBlank()) { emojiPicker = false; choose("react", emoji) } }) { Text("React") } })
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
