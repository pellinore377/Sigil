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

@Composable
internal fun ComposerPanel(draft: TextFieldState, analyze: (String) -> String, enabled: Boolean, notes: Boolean, command: Command, peer: String, voice: VoiceState, sent: Long, sentText: String?, requestContact: (() -> Unit)? = null, attachments: List<Transfer> = emptyList(), editingCaption: Boolean = false, attachmentTarget: Map<String, Any?> = mapOf("peer" to peer), send: (String, Boolean, String?) -> Unit) {
    val motionPolicy = LocalMotion.current
    var panel by remember(peer) { mutableStateOf("") }
    var showSource by remember(peer) { mutableStateOf(false) }
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
    val panelHeight by animateDpAsState(expandedHeight, if (measured > 0.dp || keyboardPending) snap() else motionPolicy.tween(MotionMillis), label = "Composer height")
    val building = panel == "Place" || panel == "Create" || createItems.any { it.first == panel }
    val formInset by animateDpAsState(if (building) measured else 0.dp, if (building) snap() else motionPolicy.tween(MotionMillis), label = "Form keyboard")
    LaunchedEffect(keyboardPending) { if (keyboardPending) { kotlinx.coroutines.delay(1500); keyboardPending = false } }
    LaunchedEffect(measured, keyboardPending) { if (keyboardPending && measured >= keyboardHeight - 2.dp) keyboardPending = false }
    fun change(value: String) { if (panel.isEmpty() && measured > 120.dp) keyboardHeight = measured; keyboardPending = false; focus.clearFocus(); panel = value; keyboard?.hide() }
    fun showKeyboard() { if (panel == "Voice") command("record_stop", emptyMap()); keyboardPending = panel.isNotEmpty(); panel = ""; editor.requestFocus(); keyboard?.show() }
    BackAction(panel.isNotEmpty()) {
        when (panel) {
            "Create", "Format", "Camera", "Place" -> change("Attachments")
            "Help" -> change(if(draft.text.toString().trim().startsWith("help::"))"" else "Create")
            in createItems.map { it.first } -> change("Create")
            else -> { if (panel == "Voice") command("record_stop", emptyMap()); change("") }
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
    val helpSource=draft.text.toString().trim().takeIf {!hasAttachment && !editingCaption && !notes && it.startsWith("help::")}
    val helpQuery=helpSource?.takeIf {';' !in it && '\n' !in it}?.removePrefix("help::")
    LaunchedEffect(helpQuery) {if(helpQuery!=null) {keyboardPending=false;panel="Help"}}
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
                        VoiceDraft(voice, Modifier.weight(1f), { command("record_preview", emptyMap()) }) { command("record_seek", mapOf("position" to it)) }
                        Symbol("delete", "Discard voice message") { command("record_cancel", emptyMap()) }
                    }
                    Composer(draft, analyze, Modifier.fillMaxWidth(), showTools = false, focusRequester = editor, namedFormatting = !hasAttachment && !editingCaption, showSource = showSource, onFocus = { if (panel == "Voice") command("record_stop", emptyMap()); if (panel.isNotEmpty()) keyboardPending = true; panel = "" })
                }

                FilledIconButton({ if (hasAttachment) {
                        val caption = draft.text.toString(); pendingCaption = caption
                        attachmentDrafts.forEach { command("file_send", mapOf("request" to it.request, "caption" to caption)) }
                        if (voiceReady) command("record_send", mapOf("peer" to peer, "caption" to caption))
                    } else if(helpQuery!=null)change("Help") else if (hasText && requestContact != null) requestContact() else if (hasText) send(if (notes && !editingCaption) "note::${escapeField(draft.text.toString())};" else helpSource ?: draft.text.toString(), notes && !editingCaption || helpSource?.endsWith(';')==true, null) else change("Voice") },
                    Modifier.size(48.dp), enabled = if (hasAttachment) enabled && voice.phase != "Sending" && attachmentDrafts.none { it.phase == "Staging" } else if(helpQuery!=null)true else if (hasText) enabled || requestContact != null else true, shape = RoundedCornerShape(16.dp),
                    colors = IconButtonDefaults.filledIconButtonColors(containerColor = MaterialTheme.colorScheme.primary, contentColor = MaterialTheme.colorScheme.onPrimary)) {
                    Glyph(if(helpQuery!=null)"help" else if (hasAttachment || hasText) "send" else "graphic_eq", 24, if(helpQuery!=null)"Open help" else if (editingCaption) "Save caption" else if (voiceReady) "Send voice message" else if (attachmentDrafts.isNotEmpty()) "Send attachments" else if (hasText) if (requestContact != null) "Send request" else "Send message" else "Voice message")
                }
            }
            Box(Modifier.fillMaxWidth().padding(bottom = formInset).then(if (panel.isEmpty() && !keyboardPending && measured > 0.dp) Modifier.windowInsetsBottomHeight(WindowInsets.ime) else Modifier.height(panelHeight))) {
                AnimatedContent(panel, transitionSpec = {
                    when {
                        initialState.isEmpty() -> (slideInHorizontally(motionPolicy.tween(MotionMillis)) { it } + fadeIn(motionPolicy.tween(MotionMillis))) togetherWith ExitTransition.None
                        targetState.isEmpty() -> EnterTransition.None togetherWith (slideOutHorizontally(motionPolicy.tween(MotionMillis)) { it } + fadeOut(motionPolicy.tween(MotionMillis)))
                        else -> {
                            val back = targetState == "Attachments" || targetState == "Create" && createItems.any { it.first == initialState }
                            (slideInHorizontally(motionPolicy.tween(MotionMillis)) { if (back) -it else it } + fadeIn()) togetherWith
                                (slideOutHorizontally(motionPolicy.tween(MotionMillis)) { if (back) it else -it } + fadeOut())
                        }
                    }
                }, label = "Composer panel") { shown ->
                    when (shown) {
                        "" -> Unit
                        "Create" -> builders.SaveableStateProvider("$peer:Create") {CreatePanel({change("Attachments")},::change)}
                        "Attachments" -> Column {
                            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) { Text("Attachments", Modifier.weight(1f).padding(start = 20.dp), style = MaterialTheme.typography.titleMedium) }
                            val items = listOf("Photos" to "image", "Camera" to "photo_camera", "Files" to "attach_file", "Place" to "location_on", "Create" to "add_notes", "Format" to "text_format")
                            LazyVerticalGrid(GridCells.Fixed(3), contentPadding = PaddingValues(horizontal = 20.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(16.dp), horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                                items(items) { (name, icon) -> Column(Modifier.clickable {
                                    when (name) {
                                        "Photos", "Files" -> command("attachment_pick", attachmentTarget + ("kind" to name))
                                        else -> change(name)
                                    }
                                }.semanticsButton(name), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(6.dp)) {
                                    Surface(Modifier.size(58.dp), shape = RoundedCornerShape(19.dp), color = MaterialTheme.colorScheme.surfaceVariant) { Box(contentAlignment = Alignment.Center) { Glyph(icon, 28) } }
                                    Text(name, style = MaterialTheme.typography.bodyMedium)
                                } }
                            }
                        }
                        "Voice" -> VoicePanel(command, peer, voice) { command("record_cancel", emptyMap()); panel = "" }
                        "Camera" -> LocalCameraPanel.current(attachmentTarget, { change("Attachments") }, { change("") })
                        "Place" -> LocalPlacePanel.current(attachmentTarget, { change("Attachments") }, { change("") })
                        "Help" -> HelpPanel(enabled,{change(if(helpQuery==null)"Create" else "")},helpQuery) {source->pendingBuilder="$peer:$shown" to source;send(source,true,null)}
                        "Randomizer" -> builders.SaveableStateProvider("$peer:$shown") {
                            RandomizerBuilder(enabled,{change("Create")}) {source->pendingBuilder="$peer:$shown" to source;send(source,true,null)}
                        }
                        "Table" -> builders.SaveableStateProvider("$peer:$shown") {
                            TableBuilder(enabled,{change("Create")}) {source->pendingBuilder="$peer:$shown" to source;send(source,true,null)}
                        }
                        in createItems.map { it.first } -> builders.SaveableStateProvider("$peer:$shown") {
                            StructuredBuilder(shown, enabled, { change("Create") }) { source, timezone -> pendingBuilder = "$peer:$shown" to source; send(source, true, timezone) }
                        }
                        "Format" -> Column(Modifier.padding(16.dp)) {
                            CompositionLocalProvider(LocalPageHeader provides false) { Header("Formatting", { change("Attachments") }) }
                            Row { listOf("Bold" to "**", "Italic" to "*", "Strike" to "~~", "Code" to "`").forEach { (name, marker) -> SigilTextButton({ draft.format(marker) }) { Text(name) } } }
                            Toggle("Show formatting syntax", showSource) { showSource = it }
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
internal fun StructuredBuilder(kind: String, enabled: Boolean, back: () -> Unit, send: (String, String?) -> Unit) {
    val motionPolicy = LocalMotion.current
    var title by rememberSaveable(kind) { mutableStateOf("") }
    var entries by rememberSaveable(kind) { mutableStateOf(listOf("")) }
    var advanced by rememberSaveable(kind) { mutableStateOf(false) }
    var multi by rememberSaveable(kind) { mutableStateOf(false) }
    var hidden by rememberSaveable(kind) { mutableStateOf(false) }
    var whenText by rememberSaveable(kind) { mutableStateOf(if (kind == "Timer") "5m" else "tomorrow 9am") }
    val temporal=kind in listOf("Reminder","Timer")
    val resolveTime=LocalTemporalPreview.current
    var preview by remember(kind,whenText) {mutableStateOf<TemporalPreview?>(null)}
    var checkingTime by remember(kind,whenText) {mutableStateOf(temporal)}
    LaunchedEffect(kind,whenText,resolveTime) {
        if(temporal) {
            kotlinx.coroutines.delay(150)
            preview=kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.Default) {resolveTime?.invoke(kind,whenText)}
            checkingTime=false
        }
    }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 24.dp, vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) { Symbol("chevron_left", "Back to create", back); Text(kind, Modifier.weight(1f), style = MaterialTheme.typography.titleLarge) }
        if (kind != "Timer") OutlinedTextField(title, { title = it }, Modifier.fillMaxWidth(), shape = RoundedCornerShape(16.dp), label = { Text(if (kind == "Poll") "Question" else if (kind == "Note") "Your note" else "Title") }, minLines = if (kind == "Note") 3 else 1)
        if (kind in listOf("Poll", "Checklist", "Task")) BuilderEntries(entries,if(kind=="Poll")"Option" else "Item",if(kind=="Poll")"radio_button_unchecked" else "check_box_outline_blank") {entries=it}
        if (temporal) {
            OutlinedTextField(whenText, { whenText = it }, Modifier.fillMaxWidth(), singleLine=true, shape=RoundedCornerShape(16.dp),
                isError=!checkingTime && preview==null && whenText.isNotBlank(), label = { Text(if (kind == "Timer") "Duration, e.g. 5m" else "When") })
            Text(if(checkingTime)"Checking time…" else preview?.label ?: if(resolveTime==null)"Time preview is unavailable." else if(kind=="Timer")"Use a duration such as 5m or 1h 30m." else "Use a date such as tomorrow 9am or 2027-07-05 9:30am.",
                Modifier.fillMaxWidth().animateContentSize(motionPolicy.tween(MotionMillis)).semantics {liveRegion=LiveRegionMode.Polite}, style=MaterialTheme.typography.bodySmall,
                color=if(checkingTime || preview!=null)MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.error)
        }
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
                "Reminder" -> "remind::${escapeField(preview?.source ?: return@SigilButton)}::$heading;"
                else -> "timer::${escapeField(preview?.source ?: return@SigilButton)};"
            }
            send(source,preview?.timezone)
        }, enabled = enabled && (!temporal || preview!=null && !checkingTime) && (if (kind == "Timer") whenText.isNotBlank() else title.isNotBlank()) && (kind !in listOf("Poll", "Checklist", "Task") || entries.count { it.isNotBlank() } >= if (kind == "Poll") 2 else 1), modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(16.dp)) { Text(if (kind == "Poll") "Send poll" else "Send") }
    }
}
@Composable
internal fun BuilderEntries(entries:List<String>,label:String,icon:String,change:(List<String>)->Unit) {
    val motion=LocalMotion.current
    val focus=LocalFocusManager.current
    Column {
        entries.forEachIndexed {index,entry->key(index) {
            val visible=remember {MutableTransitionState(index==0).apply {targetState=true}}
            AnimatedVisibility(visible,enter=expandVertically(motion.tween(MotionMillis),expandFrom=Alignment.Top)+slideInHorizontally(motion.tween(MotionMillis)) {it}+fadeIn(motion.tween(MotionMillis))) {
                OutlinedTextField(entry,{raw->val value=raw.replace('\n',' ').replace('\r',' ');change(entries.toMutableList().also {it[index]=value;if(index==it.lastIndex && value.isNotBlank() && it.size<256)it.add("")})},
                    Modifier.fillMaxWidth().padding(top=if(index==0)0.dp else 16.dp),shape=RoundedCornerShape(16.dp),singleLine=true,label={Text("$label ${index+1}")},
                    leadingIcon={Glyph(icon,20,filled=false)},keyboardOptions=KeyboardOptions(imeAction=ImeAction.Next),keyboardActions=KeyboardActions(onNext={focus.moveFocus(FocusDirection.Next)}))
            }
        }}
    }
}
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun VoicePanel(command: Command, peer: String, voice: VoiceState, close: () -> Unit) {
    val recording = voice.peer == peer && voice.phase == "Recording"
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(24.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(20.dp, Alignment.CenterVertically)) {
        Text(if (recording) if (voice.paused) "Paused" else "Recording" else "Voice message", style = MaterialTheme.typography.titleMedium)
        if (recording) {
            AudioWaveform(voice.levels, Modifier.fillMaxWidth().height(48.dp))
            Text("${voice.seconds / 60}:${(voice.seconds % 60).toString().padStart(2, '0')}", style = MaterialTheme.typography.titleMedium, fontFamily = LocalCodeFont.current)
        } else Text("Listen before you send.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            SigilTextButton(close) { Text(if (recording) "Discard" else "Cancel") }
            if (recording) SigilIconButton({ command("record_pause", emptyMap()) }) { Glyph(if (voice.paused) "mic" else "pause", 24, if (voice.paused) "Resume recording" else "Pause recording") }
            SigilButton({ command(if (recording) "record_stop" else "record_start", mapOf("peer" to peer)) }, shape = RoundedCornerShape(18.dp), contentPadding = PaddingValues(horizontal = 24.dp, vertical = 16.dp),
                colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.primary, contentColor = MaterialTheme.colorScheme.onPrimary)) { Glyph(if (recording) "check" else "mic", 24); Spacer(Modifier.width(8.dp)); Text(if (recording) "Done" else "Record") }
        }
    }
}

@Composable
private fun VoiceDraft(voice: VoiceState, modifier: Modifier, play: () -> Unit, seek: (Long) -> Unit) {
    Surface(modifier, shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.background) {
        AudioPlayback(voice.position, voice.duration.takeIf { it > 0 } ?: voice.seconds * 1000, voice.playing, voice.levels,
            enabled = voice.phase == "Ready", preview = true, modifier = Modifier.padding(end = 8.dp, bottom = 8.dp), play = play, seek = seek)
    }
}

@Composable
internal fun ComposerBar(content: @Composable RowScope.() -> Unit) {
    Row(Modifier.fillMaxWidth().padding(10.dp), verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(8.dp), content = content)
}
