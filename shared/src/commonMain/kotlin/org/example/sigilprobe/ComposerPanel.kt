@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class, androidx.compose.material3.ExperimentalMaterial3Api::class, kotlinx.coroutines.FlowPreview::class, kotlinx.coroutines.ExperimentalCoroutinesApi::class)
package org.sigil

import androidx.compose.animation.*
import androidx.compose.animation.core.*
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.grid.*
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.foundation.text.input.clearText
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.*
import androidx.compose.ui.*
import androidx.compose.ui.focus.*
import androidx.compose.ui.platform.*
import androidx.compose.ui.unit.dp
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.semantics.*
import kotlinx.coroutines.flow.*

private val createItems = listOf("Note" to "description", "Checklist" to "checklist", "Poll" to "ballot", "Reminder" to "notifications_active", "Task" to "assignment", "Timer" to "timer")
@Composable
internal fun ComposerPanel(draft: TextFieldState, analyze: (String) -> String, enabled: Boolean, notes: Boolean, command: Command, peer: String, voice: VoiceState, sent: Long, sentText: String?, requestContact: (() -> Unit)? = null, attachments: List<Transfer> = emptyList(), editingCaption: Boolean = false, send: (String, Boolean) -> Unit) {
    var panel by remember(peer) { mutableStateOf("") }
    val builders = rememberSaveableStateHolder()
    var pendingBuilder by remember(peer) { mutableStateOf<Pair<String, String>?>(null) }
    var pendingCaption by remember(peer) { mutableStateOf<String?>(null) }
    LaunchedEffect(sent) { pendingCaption?.takeIf { it == sentText }?.let { if (draft.text.toString() == it) draft.clearText(); pendingCaption = null } }
    LaunchedEffect(sent) { pendingBuilder?.takeIf { it.second == sentText }?.let {
        if ("$peer:$panel" == it.first) panel = ""
        builders.removeState(it.first); pendingBuilder = null
    } }
    val keyboard = LocalSoftwareKeyboardController.current
    val focus = LocalFocusManager.current
    val editor = remember { FocusRequester() }
    val density = LocalDensity.current
    val ime = WindowInsets.ime.getBottom(density)
    val navigation = WindowInsets.navigationBars.getBottom(density)
    val measured = with(density) { (ime - navigation).coerceAtLeast(0).toDp() }
    var keyboardHeight by remember { mutableStateOf(300.dp) }
    var keyboardPending by remember { mutableStateOf(false) }
    val keyboardSample by rememberUpdatedState(Triple(measured, panel, keyboardPending))
    LaunchedEffect(Unit) {
        snapshotFlow { keyboardSample }.debounce(180).collect { (height, active, pending) ->
            if (active.isEmpty() && !pending && height > 120.dp) keyboardHeight = height
        }
    }
    val expandedHeight = if (panel.isNotEmpty() || keyboardPending) maxOf(keyboardHeight, measured) else measured
    val panelHeight by animateDpAsState(expandedHeight, if (measured > 0.dp || keyboardPending) snap() else tween(MotionMillis), label = "Composer height")
    val building = createItems.any { it.first == panel }
    val formInset by animateDpAsState(if (building) measured else 0.dp, if (building) snap() else tween(MotionMillis), label = "Form keyboard")
    LaunchedEffect(keyboardPending) { if (keyboardPending) { kotlinx.coroutines.delay(1500); keyboardPending = false } }
    LaunchedEffect(measured, keyboardPending) { if (keyboardPending && measured >= keyboardHeight - 2.dp) keyboardPending = false }
    fun change(value: String) { if (panel.isEmpty() && measured > 120.dp) keyboardHeight = measured; keyboardPending = false; focus.clearFocus(); panel = value; keyboard?.hide() }
    fun showKeyboard() { if (panel == "Voice") command("record_stop", emptyMap()); keyboardPending = panel.isNotEmpty(); panel = ""; editor.requestFocus(); keyboard?.show() }
    BackAction(panel.isNotEmpty()) {
        when (panel) {
            "Create", "Format" -> change("Attachments")
            in createItems.map { it.first } -> change("Create")
            else -> { if (panel == "Voice") command("record_cancel", emptyMap()); change("") }
        }
    }
    LaunchedEffect(draft, peer) {
        snapshotFlow { draft.text.toString() }.drop(1).debounce(900).distinctUntilChanged().collect {
            command("draft", mapOf("peer" to peer, "text" to it))
        }
    }
    val canType by rememberUpdatedState(enabled)
    LaunchedEffect(draft, peer) {
        var lastSent = 0L
        snapshotFlow { draft.text.toString() }.drop(1).collectLatest { text ->
            val now = androidx.compose.runtime.withFrameNanos { it }
            if (canType && text.isNotBlank() && now - lastSent >= 3_000_000_000L) {
                command("typing", mapOf("peer" to peer, "active" to true)); lastSent = now
            }
            if (text.isNotBlank()) kotlinx.coroutines.delay(2500)
            if (canType || lastSent != 0L) command("typing", mapOf("peer" to peer, "active" to false)); lastSent = 0L
        }
    }
    DisposableEffect(peer) { onDispose { if (canType) command("typing", mapOf("peer" to peer, "active" to false)) } }
    val voiceReady = voice.peer == peer && voice.phase in listOf("Ready", "Sending")
    val attachmentDrafts = attachments.filter { it.draft }
    val hasAttachment = voiceReady || attachmentDrafts.isNotEmpty()
    val hasText = draft.text.isNotBlank() || editingCaption
    LaunchedEffect(voice.phase) { if (voiceReady) change("") }
    Surface(shape = RoundedCornerShape(topStart = 24.dp, topEnd = 24.dp), color = if (LocalFooterHost.current == null) MaterialTheme.colorScheme.surface else androidx.compose.ui.graphics.Color.Transparent) {
        Column {
            ComposerBar {
                Surface(shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.surfaceVariant) {
                    Symbol(if (panel.isEmpty()) "add" else "close", if (panel.isEmpty()) "Attachments" else "Close attachment panel") {
                        if (panel.isEmpty()) change("Attachments") else { if (panel == "Voice") command("record_stop", emptyMap()); change("") }
                    }
                }

                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (attachmentDrafts.isNotEmpty()) LazyRow(Modifier.fillMaxWidth().heightIn(max = 192.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        items(attachmentDrafts, key = { it.request }) { file ->
                            Surface(shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.surfaceVariant) {
                                Column(Modifier.width(176.dp).padding(8.dp)) {
                                    if (file.phase != "Staging") LocalAttachmentDraft.current(file, Modifier.fillMaxWidth().heightIn(max = 120.dp))
                                    Row(verticalAlignment = Alignment.CenterVertically) {
                                        Text(if (file.phase == "Staging") "Importing ${file.name}" else file.name, Modifier.weight(1f), maxLines = 2, style = MaterialTheme.typography.bodySmall)
                                        Symbol("close", "Remove ${file.name}") { command("file_cancel", mapOf("request" to file.request)) }
                                    }
                                }
                            }
                        }
                    }
                    if (voiceReady) Row(verticalAlignment = Alignment.CenterVertically) {
                        VoiceDraft(voice, Modifier.weight(1f)) { command("record_preview", emptyMap()) }
                        Symbol("delete", "Discard voice message") { command("record_cancel", emptyMap()) }
                    }
                    Composer(draft, analyze, Modifier.fillMaxWidth(), showTools = false, focusRequester = editor, onFocus = { if (panel == "Voice") command("record_stop", emptyMap()); if (panel.isNotEmpty()) keyboardPending = true; panel = "" })
                }

                FilledIconButton({ if (hasAttachment) {
                        val caption = draft.text.toString(); pendingCaption = caption
                        attachmentDrafts.forEach { command("file_send", mapOf("request" to it.request, "caption" to caption)) }
                        if (voiceReady) command("record_send", mapOf("peer" to peer, "caption" to caption))
                    } else if (hasText && requestContact != null) requestContact() else if (hasText) send(if (notes && !editingCaption) "note::${escapeField(draft.text.toString())};" else draft.text.toString(), notes && !editingCaption) else change("Voice") },
                    Modifier.size(48.dp), enabled = if (hasAttachment) enabled && voice.phase != "Sending" && attachmentDrafts.none { it.phase == "Staging" } else if (hasText) enabled || requestContact != null else true, shape = RoundedCornerShape(16.dp),
                    colors = IconButtonDefaults.filledIconButtonColors(containerColor = MaterialTheme.colorScheme.primary, contentColor = MaterialTheme.colorScheme.onPrimary)) {
                    Glyph(if (hasAttachment || hasText) "send" else "graphic_eq", 24, if (editingCaption) "Save caption" else if (voiceReady) "Send voice message" else if (attachmentDrafts.isNotEmpty()) "Send attachments" else if (hasText) if (requestContact != null) "Send request" else "Send message" else "Voice message")
                }
            }
            Box(Modifier.fillMaxWidth().padding(bottom = formInset).then(if (panel.isEmpty() && !keyboardPending && measured > 0.dp) Modifier.windowInsetsBottomHeight(WindowInsets.ime) else Modifier.height(panelHeight))) {
                AnimatedContent(panel, transitionSpec = {
                    when {
                        initialState.isEmpty() -> (slideInHorizontally(tween(MotionMillis)) { it } + fadeIn(tween(MotionMillis))) togetherWith ExitTransition.None
                        targetState.isEmpty() -> EnterTransition.None togetherWith (slideOutHorizontally(tween(MotionMillis)) { it } + fadeOut(tween(MotionMillis)))
                        else -> {
                            val back = targetState == "Attachments" || targetState == "Create" && createItems.any { it.first == initialState }
                            (slideInHorizontally(tween(MotionMillis)) { if (back) -it else it } + fadeIn()) togetherWith
                                (slideOutHorizontally(tween(MotionMillis)) { if (back) it else -it } + fadeOut())
                        }
                    }
                }, label = "Composer panel") { shown ->
                    when (shown) {
                        "" -> Unit
                        "Attachments", "Create" -> Column {
                            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) { if (shown == "Create") Symbol("chevron_left", "Back to attachments") { change("Attachments") }; Text(shown, Modifier.weight(1f).padding(start = 20.dp), style = MaterialTheme.typography.titleMedium) }
                            val items = if (shown == "Create") createItems else listOf("Photos" to "image", "Camera" to "photo_camera", "Files" to "attach_file", "Place" to "location_on", "Create" to "add_notes", "Format" to "text_format")
                            LazyVerticalGrid(GridCells.Fixed(3), contentPadding = PaddingValues(horizontal = 20.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(16.dp), horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                                items(items) { (name, icon) -> Column(Modifier.clickable {
                                    when (name) {
                                        "Photos", "Camera", "Files", "Place" -> command("attachment_pick", mapOf("peer" to peer, "kind" to name))
                                        else -> change(name)
                                    }
                                }.semanticsButton(name), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(6.dp)) {
                                    Surface(Modifier.size(58.dp), shape = RoundedCornerShape(19.dp), color = MaterialTheme.colorScheme.surfaceVariant) { Box(contentAlignment = Alignment.Center) { Glyph(icon, 28) } }
                                    Text(name, style = MaterialTheme.typography.bodyMedium)
                                } }
                            }
                        }
                        "Voice" -> VoicePanel(command, peer, voice) { command("record_cancel", emptyMap()); panel = "" }
                        in createItems.map { it.first } -> builders.SaveableStateProvider("$peer:$shown") {
                            StructuredBuilder(shown, enabled, { change("Create") }) { source -> pendingBuilder = "$peer:$shown" to source; send(source, true) }
                        }
                        "Format" -> Column(Modifier.padding(16.dp)) {
                            CompositionLocalProvider(LocalPageHeader provides false) { Header("Formatting", { change("Attachments") }) }
                            Row { listOf("Bold" to "**", "Italic" to "*", "Strike" to "~~", "Code" to "`").forEach { (name, marker) -> SigilTextButton({ draft.format(marker) }) { Text(name) } } }
                            SigilTextButton({ showKeyboard() }) { Text("Continue writing") }
                        }
                    }
                }
            }
        }
    }
}
private fun Modifier.semanticsButton(name: String) = this.then(Modifier.semantics { contentDescription = name; role = androidx.compose.ui.semantics.Role.Button })
internal fun escapeField(value: String) = value.replace("\\", "\\\\").replace(";", "\\;")
@Composable
private fun StructuredBuilder(kind: String, enabled: Boolean, back: () -> Unit, send: (String) -> Unit) {
    var title by rememberSaveable(kind) { mutableStateOf("") }
    var entries by rememberSaveable(kind) { mutableStateOf(listOf("")) }
    var advanced by rememberSaveable(kind) { mutableStateOf(false) }
    var multi by rememberSaveable(kind) { mutableStateOf(false) }
    var hidden by rememberSaveable(kind) { mutableStateOf(false) }
    var whenText by rememberSaveable(kind) { mutableStateOf(if (kind == "Timer") "5m" else "tomorrow 9am") }
    val focus = LocalFocusManager.current
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 24.dp, vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) { Symbol("chevron_left", "Back to create", back); Text(kind, Modifier.weight(1f), style = MaterialTheme.typography.titleLarge) }
        if (kind != "Timer") OutlinedTextField(title, { title = it }, Modifier.fillMaxWidth(), shape = RoundedCornerShape(16.dp), label = { Text(if (kind == "Poll") "Question" else if (kind == "Note") "Your note" else "Title") }, minLines = if (kind == "Note") 3 else 1)
        if (kind in listOf("Poll", "Checklist", "Task")) Column {
          entries.forEachIndexed { index, entry -> key(index) {
            val visible = remember { MutableTransitionState(index == 0).apply { targetState = true } }
            AnimatedVisibility(visible, enter = expandVertically(tween(MotionMillis), expandFrom = Alignment.Top) + slideInHorizontally(tween(MotionMillis)) { it } + fadeIn(tween(MotionMillis))) {
              OutlinedTextField(entry, { value -> entries = entries.toMutableList().also { it[index] = value; if (index == it.lastIndex && value.isNotBlank()) it.add("") } }, Modifier.fillMaxWidth().padding(top = if (index == 0) 0.dp else 16.dp),
                shape = RoundedCornerShape(16.dp), singleLine = true, label = { Text("${if (kind == "Poll") "Option" else "Item"} ${index + 1}") },
                leadingIcon = { Glyph(if (kind == "Poll") "radio_button_unchecked" else "check_box_outline_blank", 20, filled = false) },
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Next), keyboardActions = KeyboardActions(onNext = { focus.moveFocus(FocusDirection.Next) }))
            }
          } }
        }
        if (kind in listOf("Reminder", "Timer")) OutlinedTextField(whenText, { whenText = it }, Modifier.fillMaxWidth(), label = { Text(if (kind == "Timer") "Duration, e.g. 5m" else "When") })
        if (kind == "Poll") {
            SigilTextButton({ advanced = !advanced }) { Text(if (advanced) "Hide advanced" else "Advanced") }
            Expandable(advanced) { Toggle("Allow multiple choices", multi) { multi = it }; Toggle("Hide results until voting", hidden) { hidden = it } }
        }
        SigilButton({
            val heading = escapeField(title)
            val items = entries.filter { it.isNotBlank() }.joinToString("\n") { "- ${escapeField(it.trim())}" }
            val source = when (kind) {
                "Note" -> "note::$heading;"
                "Poll" -> "poll::${if (multi) "multi::" else ""}${if (hidden) "closed::" else ""}$heading\n$items;"
                "Checklist" -> "checklist::$heading\n$items;"
                "Task" -> "checklist::task::$heading\n$items;"
                "Reminder" -> "remind::${escapeField(whenText)}::$heading;"
                else -> "timer::${escapeField(whenText)};"
            }
            send(source)
        }, enabled = enabled && (if (kind == "Timer") whenText.isNotBlank() else title.isNotBlank()) && (kind !in listOf("Poll", "Checklist", "Task") || entries.count { it.isNotBlank() } >= if (kind == "Poll") 2 else 1), modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(16.dp)) { Text(if (kind == "Poll") "Send poll" else "Send") }
    }
}
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun VoicePanel(command: Command, peer: String, voice: VoiceState, close: () -> Unit) {
    val recording = voice.peer == peer && voice.phase == "Recording"
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(20.dp, Alignment.CenterVertically)) {
        Text(if (recording) "Recording" else "Voice message", style = MaterialTheme.typography.titleMedium)
        if (recording) {
            VoiceWaveform(voice.levels, Modifier.fillMaxWidth().height(48.dp))
            Text("${voice.seconds / 60}:${(voice.seconds % 60).toString().padStart(2, '0')}", style = MaterialTheme.typography.titleMedium, fontFamily = LocalCodeFont.current)
        } else Text("Listen before you send.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            SigilTextButton(close) { Text("Cancel") }
            SigilButton({ command(if (recording) "record_stop" else "record_start", mapOf("peer" to peer)) }, shape = RoundedCornerShape(18.dp), contentPadding = PaddingValues(horizontal = 24.dp, vertical = 16.dp),
                colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.primary, contentColor = MaterialTheme.colorScheme.onPrimary)) { Glyph(if (recording) "check" else "mic", 24); Spacer(Modifier.width(8.dp)); Text(if (recording) "Done" else "Record") }
        }
    }
}

@Composable
private fun VoiceDraft(voice: VoiceState, modifier: Modifier, play: () -> Unit) {
    Surface(modifier, shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.background) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Symbol(if (voice.playing) "pause" else "play_arrow", if (voice.playing) "Pause voice preview" else "Play voice preview", play)
            VoiceWaveform(voice.levels, Modifier.weight(1f).height(32.dp))
            Text("${voice.seconds / 60}:${(voice.seconds % 60).toString().padStart(2, '0')}", Modifier.padding(horizontal = 8.dp), style = MaterialTheme.typography.labelSmall)
        }
    }
}

@Composable
private fun VoiceWaveform(levels: List<Float>, modifier: Modifier) {
    val ink = LocalContentColor.current
    Canvas(modifier) { levels.forEachIndexed { i, level ->
        val x = size.width * (i + .5f) / levels.size
        val height = size.height * level.coerceIn(.08f, 1f) / 2f
        drawLine(ink, androidx.compose.ui.geometry.Offset(x, center.y - height), androidx.compose.ui.geometry.Offset(x, center.y + height), 2.dp.toPx(), androidx.compose.ui.graphics.StrokeCap.Round)
    } }
}

@Composable
internal fun ComposerBar(content: @Composable RowScope.() -> Unit) {
    Row(Modifier.fillMaxWidth().padding(10.dp), verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(8.dp), content = content)
}
