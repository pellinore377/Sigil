@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class, kotlinx.coroutines.FlowPreview::class, kotlinx.coroutines.ExperimentalCoroutinesApi::class)
package org.sigil

import androidx.compose.animation.*
import androidx.compose.animation.core.*
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.grid.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.*
import androidx.compose.ui.*
import androidx.compose.ui.focus.*
import androidx.compose.ui.platform.*
import androidx.compose.ui.unit.dp
import androidx.compose.ui.semantics.*
import kotlinx.coroutines.flow.*

private val createItems = listOf("Note" to "description", "Checklist" to "checklist", "Poll" to "ballot", "Reminder" to "notifications_active", "Task" to "assignment", "Timer" to "timer")
@Composable
internal fun ComposerPanel(draft: TextFieldState, analyze: (String) -> String, enabled: Boolean, notes: Boolean, command: Command, peer: String, voice: VoiceState, sent: Long, sentText: String?, requestContact: (() -> Unit)? = null, send: (String, Boolean) -> Unit) {
    var panel by remember(peer) { mutableStateOf("") }
    val builders = rememberSaveableStateHolder()
    var pendingBuilder by remember(peer) { mutableStateOf<Pair<String, String>?>(null) }
    LaunchedEffect(sent) { pendingBuilder?.takeIf { it.second == sentText }?.let { builders.removeState(it.first); pendingBuilder = null; if ("$peer:$panel" == it.first) panel = "" } }
    val keyboard = LocalSoftwareKeyboardController.current
    val focus = LocalFocusManager.current
    val editor = remember { FocusRequester() }
    val density = LocalDensity.current
    val ime = WindowInsets.ime.getBottom(density)
    val navigation = WindowInsets.navigationBars.getBottom(density)
    val measured = with(density) { (ime - navigation).coerceAtLeast(0).toDp() }
    var keyboardHeight by remember { mutableStateOf(300.dp) }
    var keyboardPending by remember { mutableStateOf(false) }
    if (panel.isEmpty() && !keyboardPending && measured > 120.dp) keyboardHeight = measured
    val expandedHeight = if (panel.isNotEmpty() || keyboardPending) maxOf(keyboardHeight, measured) else measured
    val panelHeight by animateDpAsState(expandedHeight, if (measured > 0.dp || keyboardPending) snap() else tween(MotionMillis), label = "Composer height")
    LaunchedEffect(keyboardPending) { if (keyboardPending) { kotlinx.coroutines.delay(1500); keyboardPending = false } }
    LaunchedEffect(measured, keyboardPending) { if (keyboardPending && measured >= keyboardHeight - 2.dp) keyboardPending = false }
    fun change(value: String) { keyboardPending = false; focus.clearFocus(); panel = value; keyboard?.hide() }
    fun showKeyboard() { keyboardPending = panel.isNotEmpty(); panel = ""; editor.requestFocus(); keyboard?.show() }
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
    Surface(shape = RoundedCornerShape(topStart = 24.dp, topEnd = 24.dp), color = MaterialTheme.colorScheme.background,
        border = BorderStroke(0.5.dp, MaterialTheme.colorScheme.outlineVariant)) {
        Column {
            Row(Modifier.fillMaxWidth().padding(horizontal = 10.dp, vertical = 10.dp), verticalAlignment = Alignment.Bottom) {
                Surface(shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.surfaceVariant) {
                    Symbol(if (panel.isEmpty()) "add" else "keyboard", if (panel.isEmpty()) "Attachments" else "Show keyboard") {
                        if (panel.isEmpty()) change("Attachments") else showKeyboard()
                    }
                }
                Spacer(Modifier.width(8.dp))
                Composer(draft, analyze, Modifier.weight(1f), showTools = false, focusRequester = editor, onFocus = { if (panel.isNotEmpty()) keyboardPending = true; panel = "" })
                Spacer(Modifier.width(8.dp))
                FilledIconButton({ if (draft.text.isNotBlank() && requestContact != null) requestContact() else if (draft.text.isNotBlank()) send(if (notes) "note::${escapeField(draft.text.toString())};" else draft.text.toString(), notes) else change("Voice") },
                    Modifier.size(48.dp), enabled = if (draft.text.isNotBlank()) enabled || requestContact != null else true, shape = RoundedCornerShape(16.dp),
                    colors = IconButtonDefaults.filledIconButtonColors(containerColor = MaterialTheme.colorScheme.inverseSurface, contentColor = MaterialTheme.colorScheme.inverseOnSurface)) {
                    Glyph(if (draft.text.isNotBlank()) "arrow_upward" else "graphic_eq", 25, if (draft.text.isNotBlank()) if (requestContact != null) "Send request" else "Send message" else "Voice message")
                }
            }
            Box(Modifier.fillMaxWidth().height(panelHeight)) {
                AnimatedContent(panel, transitionSpec = { (slideInHorizontally(tween(MotionMillis)) { -it } + fadeIn()) togetherWith (slideOutHorizontally(tween(MotionMillis)) { it } + fadeOut()) }, label = "Composer panel") { shown ->
                    when (shown) {
                        "" -> Unit
                        "Attachments", "Create" -> Column {
                            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) { if (shown == "Create") Symbol("chevron_left", "Back to attachments") { change("Attachments") }; Text(shown, Modifier.weight(1f).padding(start = 20.dp), style = MaterialTheme.typography.titleMedium); Symbol("close", "Close attachment panel") { panel = "" } }
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
                        "Voice" -> VoicePanel(command, peer, voice, enabled) { command("record_cancel", emptyMap()); panel = "" }
                        "Format" -> Column(Modifier.padding(16.dp)) {
                            Header("Formatting", { change("Attachments") })
                            Row { listOf("Bold" to "**", "Italic" to "*", "Strike" to "~~", "Code" to "`").forEach { (name, marker) -> TextButton({ draft.format(marker) }) { Text(name) } } }
                            TextButton({ showKeyboard() }) { Text("Continue writing") }
                        }
                        else -> builders.SaveableStateProvider("$peer:$shown") {
                            StructuredBuilder(shown, enabled, { change("Create") }) { source -> pendingBuilder = "$peer:$shown" to source; send(source, true) }
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
    var entries by rememberSaveable(kind) { mutableStateOf("") }
    var advanced by rememberSaveable(kind) { mutableStateOf(false) }
    var multi by rememberSaveable(kind) { mutableStateOf(false) }
    var hidden by rememberSaveable(kind) { mutableStateOf(false) }
    var whenText by rememberSaveable(kind) { mutableStateOf(if (kind == "Timer") "5m" else "tomorrow 9am") }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) { Symbol("chevron_left", "Back to create", back); Text(kind, Modifier.weight(1f), style = MaterialTheme.typography.titleLarge) }
        if (kind != "Timer") OutlinedTextField(title, { title = it }, Modifier.fillMaxWidth(), label = { Text(if (kind == "Poll") "Question" else if (kind == "Note") "Your note" else "Title") }, minLines = if (kind == "Note") 2 else 1)
        if (kind in listOf("Poll", "Checklist", "Task")) OutlinedTextField(entries, { entries = it }, Modifier.fillMaxWidth(), label = { Text(if (kind == "Poll") "Options · one per line" else "Items · one per line") }, minLines = 2)
        if (kind in listOf("Reminder", "Timer")) OutlinedTextField(whenText, { whenText = it }, Modifier.fillMaxWidth(), label = { Text(if (kind == "Timer") "Duration, e.g. 5m" else "When") })
        if (kind == "Poll") {
            TextButton({ advanced = !advanced }) { Text(if (advanced) "Hide advanced" else "Advanced") }
            if (advanced) { Toggle("Allow multiple choices", multi) { multi = it }; Toggle("Hide results until voting", hidden) { hidden = it } }
        }
        Button({
            val heading = escapeField(title)
            val items = entries.lines().filter { it.isNotBlank() }.joinToString("\n") { "- ${escapeField(it.trim())}" }
            val source = when (kind) {
                "Note" -> "note::$heading;"
                "Poll" -> "poll::${if (multi) "multi::" else ""}${if (hidden) "closed::" else ""}$heading\n$items;"
                "Checklist" -> "checklist::$heading\n$items;"
                "Task" -> "checklist::task::$heading\n$items;"
                "Reminder" -> "remind::${escapeField(whenText)}::$heading;"
                else -> "timer::${escapeField(whenText)};"
            }
            send(source)
        }, enabled = enabled && (if (kind == "Timer") whenText.isNotBlank() else title.isNotBlank()) && (kind !in listOf("Poll", "Checklist", "Task") || entries.lines().count { it.isNotBlank() } >= if (kind == "Poll") 2 else 1), modifier = Modifier.align(Alignment.End)) { Text(if (kind == "Poll") "Send poll" else "Send") }
    }
}
@Composable
private fun VoicePanel(command: Command, peer: String, voice: VoiceState, enabled: Boolean, close: () -> Unit) {
    val recording = voice.peer == peer && voice.phase == "Recording"
    val ready = voice.peer == peer && voice.phase == "Ready"
    val color = MaterialTheme.colorScheme.onBackground
    Column(Modifier.fillMaxSize().padding(20.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.SpaceBetween) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) { Text("Voice message", Modifier.weight(1f), style = MaterialTheme.typography.titleMedium); Symbol("close", "Close voice panel", close) }
        Canvas(Modifier.fillMaxWidth().height(64.dp)) {
            val levels = voice.levels.takeIf { voice.peer == peer }.orEmpty()
            if (levels.isEmpty()) drawLine(color.copy(alpha = .3f), androidx.compose.ui.geometry.Offset(0f, center.y), androidx.compose.ui.geometry.Offset(size.width, center.y), 1.dp.toPx())
            else levels.forEachIndexed { index, level -> val x = size.width * (index + .5f) / levels.size; val amplitude = size.height * level.coerceIn(.02f, 1f) / 2f; drawLine(color, androidx.compose.ui.geometry.Offset(x, center.y - amplitude), androidx.compose.ui.geometry.Offset(x, center.y + amplitude), 3.dp.toPx(), androidx.compose.ui.graphics.StrokeCap.Round) }
        }
        Text(if (recording || ready) "${voice.seconds / 60}:${(voice.seconds % 60).toString().padStart(2, '0')}" else "Ready to record", style = MaterialTheme.typography.titleMedium)
        Row(horizontalArrangement = Arrangement.spacedBy(20.dp)) {
            OutlinedButton({ command("record_cancel", emptyMap()) }) { Text("Discard") }
            Button({ command(if (recording) "record_stop" else "record_start", mapOf("peer" to peer)) }) { Glyph(if (recording) "stop" else "mic", 20); Text(if (recording) "Stop" else if (ready) "Record again" else "Record") }
            OutlinedButton({ command("record_send", mapOf("peer" to peer)) }, enabled = enabled && ready) { Text("Send") }
        }
    }
}
