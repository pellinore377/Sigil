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
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.platform.*
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.semantics.*
import kotlinx.coroutines.flow.*

@Composable
internal fun ComposerPanel(draft: TextFieldState, analyze: (String) -> String, enabled: Boolean, notes: Boolean, command: Command, peer: String, voice: VoiceState, sent: Long, sentText: String?, requestContact: (() -> Unit)? = null, attachments: List<Transfer> = emptyList(), editingCaption: Boolean = false, attachmentTarget: Map<String, Any?> = mapOf("peer" to peer), send: (String, Boolean, String?) -> Unit) {
    val motionPolicy = LocalMotion.current
    val launch = LocalPreviewLaunch.current
    var panel by remember(peer) { mutableStateOf("") }
    val confirmation=remember(peer,panel){ComposerConfirmation()}
    val outgoingConfirmation=remember(peer){ComposerConfirmation()}
    var showSource by remember(peer) { mutableStateOf(false) }
    val builders = rememberSaveableStateHolder()
    var pendingBuilder by remember(peer) { mutableStateOf<Triple<String, String, Long>?>(null) }
    var pendingIntent by remember(peer) { mutableStateOf<Pair<PreviewIntent, String>?>(null) }
    var stagedPreview by remember(peer) {mutableStateOf<MessagePart?>(null)}
    var stagedPreviewSource by remember(peer) {mutableStateOf("")}
    var stagedSource by rememberSaveable(peer) { mutableStateOf("") }
    var stagedKind by rememberSaveable(peer) { mutableStateOf("") }
    var stagedTimezone by rememberSaveable(peer) { mutableStateOf<String?>(null) }
    var stagedContact by rememberSaveable(peer){mutableStateOf<String?>(null)}
    var stagedQuery by rememberSaveable(peer){mutableStateOf<String?>(null)}
    var stagedRich by rememberSaveable(peer) { mutableStateOf(true) }
    var pendingDraft by remember(peer) { mutableStateOf<List<String?>>(emptyList()) }
    fun draftIdentity() = listOf(stagedKind, stagedSource, stagedQuery, stagedContact, stagedTimezone, stagedRich.toString())
    var sentCaption by remember(peer) { mutableStateOf("") }
    var pendingCaption by remember(peer) { mutableStateOf<String?>(null) }
    LaunchedEffect(sent) { pendingCaption?.takeIf { it == sentText }?.let { if (draft.text.toString() == it) draft.clearText(); pendingCaption = null } }
    LaunchedEffect(sent, launch?.activeSource) { pendingBuilder?.takeIf { sent > it.third && it.second == sentText && launch?.holding(it.second)!=true }?.let {
        if (pendingDraft == draftIdentity() && "$peer:$panel" != it.first) {
            stagedSource="";stagedKind="";stagedTimezone=null;stagedQuery=null;stagedContact=null
            if(draft.text.toString()==sentCaption)draft.clearText()
            builders.removeState(it.first)
        }
        pendingBuilder = null; pendingDraft = emptyList()
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
    val headerBottom=LocalFooterHost.current?.headerBottom ?: 0f
    var panelBottom by remember {mutableFloatStateOf(0f)}
    val screenHeight=with(density){LocalWindowInfo.current.containerSize.height.toDp()}
    val available=(screenHeight-measured-navigation.toFloat().let {with(density){it.toDp()}}-180.dp).coerceAtLeast(120.dp)
    val preferredHeights=remember {mutableStateMapOf<String,Dp>()}
    val compactHeight=preferredHeights[panel] ?: when(panel){"Attachments"->204.dp;"Create"->380.dp;"Format"->104.dp;"Voice"->56.dp;"Camera"->maxOf(420.dp,available-12.dp);else->maxOf(keyboardHeight,420.dp)}
    val contextual=panel in createItems.map {it.first}.filter {it!="Help"} || panel in listOf("Code block","Camera","One-time location","Real-time location","Drop a pin") || panel=="Help" && confirmation.action!=null
    val cameraLimit=if(panel=="Camera" && panelBottom>0f && headerBottom>0f)
        with(density){(panelBottom-headerBottom).coerceAtLeast(0f).toDp()}.minus(8.dp).coerceAtLeast(0.dp) else available
    val expandedHeight = if (panel.isNotEmpty()) minOf(compactHeight,available,cameraLimit) else 0.dp
    val panelHeight by animateDpAsState(expandedHeight, if (measured > 0.dp || keyboardPending) snap() else motionPolicy.tween(MotionMillis), label = "Composer height")
    LaunchedEffect(keyboardPending) { if (keyboardPending) { kotlinx.coroutines.delay(1500); keyboardPending = false } }
    LaunchedEffect(measured, keyboardPending) { if (keyboardPending && measured >= keyboardHeight - 2.dp) keyboardPending = false }
    fun change(value: String) { if (panel.isEmpty() && measured > 120.dp) keyboardHeight = measured; keyboardPending = false; panel = value; if(value=="Format"){editor.requestFocus();keyboard?.show()}else{focus.clearFocus();keyboard?.hide()} }
    fun stage(kind:String,source:String,rich:Boolean=true,timezone:String?=null) {stagedQuery?.let {command("service",mapOf("action" to "discard","request" to it))};stagedQuery=null;stagedContact=null;stagedPreview=null;stagedKind=kind;stagedSource=source;stagedRich=rich;stagedTimezone=timezone;change("")}
    fun showKeyboard() { if (panel == "Voice") command("record_stop", emptyMap()); keyboardPending = panel.isNotEmpty(); if(panel!="Format")panel = ""; editor.requestFocus(); keyboard?.show() }
    BackAction(panel.isNotEmpty()) {
        when (panel) {
            "Create", "Format", "Camera", "One-time location", "Real-time location", "Drop a pin" -> change("Attachments")
            "Code block" -> change("Format")
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
    LaunchedEffect(attachmentDrafts.map {it.request to it.phase}) {if(attachmentDrafts.isNotEmpty() && (panel=="Attachments" || panel=="Voice" && attachmentDrafts.any {it.phase!="Staging"}))change("")}
    val hasAttachment = voiceReady || attachmentDrafts.isNotEmpty()
    val hasStructured=stagedSource.isNotEmpty() || stagedQuery!=null || stagedContact!=null
    val previewer=LocalStructuredPreview.current
    LaunchedEffect(stagedSource,previewer) {stagedPreviewSource="";if(stagedSource.isNotEmpty()){stagedPreview=kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.Default){previewer?.invoke(stagedSource)};stagedPreviewSource=stagedSource}}
    val hasText = draft.text.isNotBlank() || editingCaption
    val helpSource=draft.text.toString().trim().takeIf {!hasAttachment && !editingCaption && !notes && it.startsWith("help::")}
    val helpQuery=helpSource?.takeIf {';' !in it && '\n' !in it}?.removePrefix("help::")
    LaunchedEffect(helpQuery) {if(helpQuery!=null) {keyboardPending=false;panel="Help"}}
    LaunchedEffect(voice.phase) { if (voiceReady) change("") }
    Surface(shape = RoundedCornerShape(24.dp), color = if (LocalFooterHost.current == null) MaterialTheme.colorScheme.surface else androidx.compose.ui.graphics.Color.Transparent) {
        Column {
            Box(Modifier.fillMaxWidth().height(if(panel=="Camera")minOf(panelHeight,cameraLimit)else panelHeight).onGloballyPositioned {panelBottom=it.boundsInWindow().bottom}.testTag("composer-panel")) {
                CompositionLocalProvider(LocalBuilderAction provides "Attach") {
                AnimatedContent(panel, transitionSpec = {
                    when {
                        initialState.isEmpty() -> (slideInHorizontally(motionPolicy.enter(MotionMillis)) { it } + fadeIn(motionPolicy.enter(MotionMillis))) togetherWith ExitTransition.None
                        targetState.isEmpty() -> EnterTransition.None togetherWith (slideOutHorizontally(motionPolicy.exit(MotionMillis)) { it } + fadeOut(motionPolicy.exit(MotionExit)))
                        else -> {
                            val back = targetState == "Attachments" || targetState == "Create" && createItems.any { it.first == initialState } || targetState == "Format" && initialState == "Code block"
                            (slideInHorizontally(motionPolicy.enter(MotionMillis)) { if (back) -it else it } + fadeIn(motionPolicy.enter(MotionMillis))) togetherWith
                                (slideOutHorizontally(motionPolicy.exit(MotionMillis)) { if (back) it else -it } + fadeOut(motionPolicy.exit(MotionExit)))
                        }
                    }
                }, label = "Composer panel") { shown ->
                    CompositionLocalProvider(LocalComposerConfirmation provides if(shown==panel)confirmation else outgoingConfirmation,LocalComposerPanelHeight provides {height->if(shown==panel)preferredHeights[shown]=height}) {
                    when (shown) {
                        "" -> Unit
                        "Create" -> builders.SaveableStateProvider("$peer:Create") {CreatePanel({change("Attachments")},::change)}
                        "Attachments" -> AttachmentTools(hasAttachment,hasStructured) {name->
                            if(name in listOf("Photos","Files"))command("attachment_pick",attachmentTarget+("kind" to name)) else change(name)
                        }
                        "Voice" -> VoicePanel(command, peer, voice) { command("record_cancel", emptyMap()); panel = "" }
                        "Camera" -> LocalCameraPanel.current(attachmentTarget, { change("Attachments") }, { change("") })
                        "One-time location", "Real-time location", "Drop a pin" -> {
                            val caption=draft.text.toString()
                            LocalPlacePanel.current(attachmentTarget + mapOf("location_mode" to when(shown){"Real-time location"->"live";"Drop a pin"->"pin";else->"once"},"location_caption" to caption), { change("Attachments") }, {
                                if(draft.text.toString()==caption)draft.edit {replace(0,length,"")}
                                change("")
                            })
                        }
                        "Help" -> HelpPanel(enabled,{change(if(helpQuery==null)"Create" else "")},helpQuery) {source->stage(shown,source)}
                        "Dice", "Coin", "Cards", "Random Number", "Randomizer" -> builders.SaveableStateProvider("$peer:$shown") {
                            RandomizerBuilder(enabled,{change("Create")},initialMode=when(shown){"Cards"->"Choice";"Random Number"->"Number";"Randomizer"->null;else->shown}) {source->stage(shown,source)}
                        }
                        "Table" -> builders.SaveableStateProvider("$peer:$shown") {
                            TableBuilder(enabled,{change("Create")}) {source->stage(shown,source)}
                        }
                        "Code block" -> builders.SaveableStateProvider("$peer:$shown") {
                            CodeBuilder(enabled,{change("Format")}) {source->stage(shown,source,false)}
                        }
                        "Translation","Definition","Weather","Contact" -> ServiceBuilder(shown,{pendingIntent=null;change("Create")},{command("service",mapOf("action" to "discard","request" to it))},initial=pendingIntent?.first?.takeIf {it.tool==shown}) {query,preview,contact->
                            pendingIntent?.let {(intent,original)->if(draft.text.toString()==original && intent.start in 0..intent.end && intent.end<=original.length)draft.edit {replace(intent.start,intent.end,"")}};pendingIntent=null
                            stagedQuery?.takeIf {it!=query}?.let {command("service",mapOf("action" to "discard","request" to it))};stagedKind=shown;stagedSource="";stagedQuery=if(contact)null else query;stagedContact=if(contact)query else null;stagedPreview=preview;change("")}
                        in formSpecs.keys -> builders.SaveableStateProvider("$peer:$shown") {
                            FormBuilder(shown,enabled,{change("Create")}) {source,timezone->stage(shown,source,true,timezone)}
                        }
                        in createItems.map { it.first } -> builders.SaveableStateProvider("$peer:$shown") {
                            StructuredBuilder(shown, enabled, { change("Create") }) { source, timezone -> stage(shown,source,true,timezone) }
                        }
                        "Format" -> FormatPanel(draft,analyze,showSource,{showSource=it},{change("Attachments")},{change("Code block")},::showKeyboard)
                    }
                    }
                }
            }
            }
            Column(Modifier.fillMaxWidth().heightIn(max=minOf(if(hasStructured)320.dp else 240.dp,available)).verticalScroll(rememberScrollState()).padding(start=8.dp,end=8.dp,top=if(hasStructured || hasAttachment)8.dp else 0.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                    if(panel.isEmpty() && !hasStructured && !hasAttachment && !editingCaption && !notes) TypedSigilPreview(draft.text.toString(),open={intent,original->pendingIntent=intent to original;change(intent.tool)})
                    if(hasStructured && panel.isEmpty()) {
                        Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {Glyph(createItems.firstOrNull {it.first==stagedKind}?.second ?: "draft",18);Text(stagedKind,Modifier.weight(1f),style=MaterialTheme.typography.labelMedium,maxLines=1,overflow=TextOverflow.Ellipsis);if(stagedQuery==null && stagedContact==null)Symbol("edit","Edit $stagedKind") {change(stagedKind)};Symbol("close","Remove $stagedKind") {stagedSource="";stagedKind="";stagedQuery?.let {command("service",mapOf("action" to "discard","request" to it))};stagedQuery=null;stagedContact=null;stagedPreview=null}}
                        stagedPreview?.let {if(stagedSource.isNotEmpty()){if(stagedPreviewSource==stagedSource)StructuredDraftPreview(it,stagedSource+draft.text.toString().takeIf {it.isNotBlank()}?.let {"\n\n$it"}.orEmpty())}else BuilderPreview(it)}
                    }
                    if (attachmentDrafts.any {!it.mediaType.startsWith("audio/")}) LazyRow(Modifier.fillMaxWidth().heightIn(max = 144.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        items(attachmentDrafts.filter {!it.mediaType.startsWith("audio/")}, key = { it.request }) { file ->
                            Surface(itemMotion(), shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.surfaceVariant) {
                                Box(Modifier.size(128.dp)) {
                                    if (file.phase != "Staging") LocalAttachmentDraft.current(file, Modifier.fillMaxSize())
                                    else CircularProgressIndicator(Modifier.size(24.dp).align(Alignment.Center),strokeWidth=2.dp)
                                    if(!file.mediaType.startsWith("image/"))Text(file.name,Modifier.align(Alignment.BottomStart).padding(8.dp),maxLines=2,overflow=TextOverflow.Ellipsis,style=MaterialTheme.typography.labelSmall)
                                    Surface(Modifier.align(Alignment.TopEnd).padding(4.dp),shape=RoundedCornerShape(12.dp),color=MaterialTheme.colorScheme.surfaceContainerHigh) {
                                        Symbol("close", "Remove ${file.name}") { command("file_cancel", mapOf("request" to file.request)) }
                                    }
                                }
                            }
                        }
                        item(key="add") {
                            Surface(onClick={command("attachment_pick",attachmentTarget+("kind" to "Photos"))},modifier=itemMotion().size(128.dp),shape=RoundedCornerShape(16.dp),color=MaterialTheme.colorScheme.surfaceVariant) {
                                Box(contentAlignment=Alignment.Center) {Glyph("add",28,"Add photos")}
                            }
                        }
                    }
                    attachmentDrafts.filter {it.mediaType.startsWith("audio/")}.forEach {file->
                        Row(verticalAlignment=Alignment.CenterVertically) {
                            if(file.phase=="Staging")LinearProgressIndicator(Modifier.weight(1f)) else LocalAttachmentDraft.current(file,Modifier.weight(1f))
                            Symbol("delete","Discard audio attachment") {command("file_cancel",mapOf("request" to file.request))}
                        }
                    }
                    if (voiceReady) Row(verticalAlignment = Alignment.CenterVertically) {
                        VoiceDraft(voice, Modifier.weight(1f), { command("record_preview", emptyMap()) }) { command("record_seek", mapOf("position" to it)) }
                        Symbol("delete", "Discard voice message") { command("record_cancel", emptyMap()) }
                    }
            }
            ComposerBar {
                Crossfade(if (panel.isEmpty()) "add" else "close", animationSpec = motionPolicy.tween(MotionMillis), label = "Attach toggle") { icon ->
                    Symbol(icon, if (icon == "add") "Attachments" else "Close attachment panel") {
                        if (panel.isEmpty()) change("Attachments") else { if (panel == "Voice") command("record_stop", emptyMap()); change("") }
                    }
                }

                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Composer(draft, analyze, Modifier.fillMaxWidth(), showTools = false, focusRequester = editor, namedFormatting = !hasAttachment && !editingCaption, showSource = showSource, onFocus = { if (panel == "Voice") command("record_stop", emptyMap()); if (panel == "Attachments") {keyboardPending=true;panel=""} })
                }

                val sendIcon=if(panel in listOf("One-time location","Real-time location","Drop a pin"))"send" else if(contextual)"check" else if(voice.peer==peer && voice.phase=="Save failed")"refresh" else if(voice.peer==peer && voice.phase=="Recording")"stop" else if(helpQuery!=null)"help" else if (hasAttachment || hasStructured || hasText) "send" else "graphic_eq"
                val sendLabel=if(contextual)confirmation.action?.label ?: "Complete attachment" else if(voice.peer==peer && voice.phase=="Save failed")"Retry saving recording" else if(voice.peer==peer && voice.phase=="Recording")"Stop recording" else if(helpQuery!=null)"Open help" else if (editingCaption) "Save caption" else if (voiceReady) "Send voice message" else if (attachmentDrafts.isNotEmpty()) "Send attachments" else if(hasStructured)"Send message" else if (hasText) if (requestContact != null) "Send request" else "Send message" else "Voice message"
                SigilFilledIconButton({ if(contextual){confirmation.action?.takeIf {it.enabled}?.invoke?.invoke()} else if (hasAttachment) {
                        val caption = draft.text.toString(); pendingCaption = caption
                        attachmentDrafts.forEach { command("file_send", mapOf("request" to it.request, "caption" to caption)) }
                        if (voiceReady) command("record_send", mapOf("peer" to peer, "caption" to caption))
                    } else if(hasStructured) {
                        pendingDraft=draftIdentity()
                        sentCaption=draft.text.toString()
                        val text=stagedSource+if(sentCaption.isBlank())"" else "\n\n"+sentCaption
                        if(stagedQuery!=null || stagedContact!=null) {pendingBuilder=Triple("$peer:$stagedKind",sentCaption,sent);command("post",attachmentTarget+mapOf("text" to sentCaption,"service_query" to stagedQuery,"shared_contact" to stagedContact,"rich" to true))} else {pendingBuilder=Triple("$peer:$stagedKind",text,sent);send(text,stagedRich,stagedTimezone)}
                    } else if(voice.peer==peer && voice.phase in listOf("Recording","Save failed"))command("record_stop",emptyMap()) else if(panel=="Voice")command("record_start",mapOf("peer" to peer)) else if(helpQuery!=null)change("Help") else if (hasText && requestContact != null) requestContact() else if (hasText) {
                        val source=if(notes && !editingCaption) "note::${escapeField(draft.text.toString())};" else helpSource ?: draft.text.toString()
                        val structured=previewer?.invoke(source)?.previewLeaves()?.any {it.kind!="text" && it.previewIntent==null}==true
                        send(source,notes && !editingCaption || helpSource?.endsWith(';')==true || structured,null)
                    } else {change("Voice");command("record_start",mapOf("peer" to peer))} },
                    Modifier.semantics {contentDescription=sendLabel}, enabled = if(launch?.activeSource!=null)false else if(contextual)confirmation.action?.enabled==true else voice.phase !in listOf("Starting","Saving") && if (hasAttachment) enabled && voice.phase != "Sending" && attachmentDrafts.none { it.phase == "Staging" } else if(hasStructured)enabled else if(helpQuery!=null)true else if (hasText) enabled || requestContact != null else LocalClientFeatures.current.voice) {
                    Crossfade(sendIcon, animationSpec = motionPolicy.tween(MotionMillis), label = "Send action") { icon -> Glyph(icon, 24) }
                }
            }
            if(LocalFooterHost.current==null)Spacer(Modifier.windowInsetsBottomHeight(WindowInsets.ime))
        }
    }
}
internal fun escapeField(value: String) = value.replace("\\", "\\\\").replace(";", "\\;")

// Filled counterpart to SigilIconButton; disabled ink follows the palette, not Material's own alphas.
@Composable
internal fun SigilFilledIconButton(onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true, content: @Composable () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    Surface(onClick, modifier.semantics { role = Role.Button }, enabled, shape = SigilButtonShape,
        color = if (enabled) scheme.primary else scheme.surfaceVariant,
        contentColor = if (enabled) scheme.onPrimary else scheme.onSurfaceVariant.copy(alpha = .38f)) {
        Box(Modifier.size(48.dp), contentAlignment = Alignment.Center) { content() }
    }
}
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
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).wrapContentHeight(unbounded=true).then(naturalPanelHeight()).padding(start=8.dp,end=8.dp,top=8.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
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
        BuilderConfirm(enabled = enabled && (!temporal || preview!=null && !checkingTime) && (if (kind == "Timer") whenText.isNotBlank() else title.isNotBlank()) && (kind !in listOf("Poll", "Checklist", "Task") || entries.count { it.isNotBlank() } >= if (kind == "Poll") 2 else 1)) {
            val heading = escapeField(title)
            val items = entries.filter { it.isNotBlank() }.joinToString("\n") { "- ${escapeField(it.trim())}" }
            val source = when (kind) {
                "Note" -> "note::$heading;"
                "Poll" -> "poll::${if (multi) "multi::" else ""}${if (hidden) "closed::" else ""}$heading\n$items;"
                "Checklist" -> "checklist::$heading\n$items;"
                "Task" -> "checklist::task::$heading\n$items;"
                "Reminder" -> "remind::${escapeField(preview?.source ?: return@BuilderConfirm)}::$heading;"
                else -> "timer::${escapeField(preview?.source ?: return@BuilderConfirm)};"
            }
            send(source,preview?.timezone)
        }
    }
}
@Composable
internal fun BuilderEntries(entries:List<String>,label:String,icon:String,change:(List<String>)->Unit) {
    val motion=LocalMotion.current
    val focus=LocalFocusManager.current
    Column {
        entries.forEachIndexed {index,entry->key(index) {
            val visible=remember {MutableTransitionState(index==0).apply {targetState=true}}
            AnimatedVisibility(visible,enter=expandVertically(motion.enter(MotionMillis),expandFrom=Alignment.Top)+slideInHorizontally(motion.enter(MotionMillis)) {it}+fadeIn(motion.enter(MotionMillis)),
                exit=shrinkVertically(motion.exit(MotionQuick),shrinkTowards=Alignment.Top)+slideOutHorizontally(motion.exit(MotionQuick)) {it}+fadeOut(motion.exit(MotionExit)),label="Builder entry") {
                OutlinedTextField(entry,{raw->val value=raw.replace('\n',' ').replace('\r',' ');change(entries.toMutableList().also {it[index]=value;if(index==it.lastIndex && value.isNotBlank() && it.size<256)it.add("")})},
                    Modifier.fillMaxWidth().padding(top=if(index==0)0.dp else 12.dp),shape=RoundedCornerShape(16.dp),singleLine=true,label={Text("$label ${index+1}")},
                    leadingIcon={Glyph(icon,20,filled=false)},keyboardOptions=KeyboardOptions(imeAction=ImeAction.Next),keyboardActions=KeyboardActions(onNext={focus.moveFocus(FocusDirection.Next)}))
            }
        }}
    }
}
@Composable
private fun VoicePanel(command: Command, peer: String, voice: VoiceState, close: () -> Unit) {
    val recording=voice.peer==peer && voice.phase=="Recording"
    Row(Modifier.fillMaxWidth().padding(start=8.dp,end=8.dp,top=8.dp).semantics {liveRegion=LiveRegionMode.Polite;stateDescription=if(!recording)"Stopped" else if(voice.paused)"Paused" else "Recording"},verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {
        Symbol("delete","Discard recording",close)
        Box(Modifier.size(8.dp).background(if(recording && !voice.paused)MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,androidx.compose.foundation.shape.CircleShape))
        AudioWaveform(if(recording)voice.levels else emptyList(),Modifier.weight(1f).height(48.dp))
        Text(audioTime(voice.seconds*1000),style=MaterialTheme.typography.labelLarge)
        if(recording)Symbol(if(voice.paused)"play_arrow" else "pause",if(voice.paused)"Resume recording" else "Pause recording") {command("record_pause",emptyMap())}
    }
}

@Composable
private fun VoiceDraft(voice: VoiceState, modifier: Modifier, play: () -> Unit, seek: (Long) -> Unit) {
    Surface(modifier, shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.surfaceContainerHigh) {
        AudioPlayback(voice.position, voice.duration.takeIf { it > 0 } ?: voice.seconds * 1000, voice.playing, voice.levels,
            enabled = voice.phase == "Ready", preview = true, modifier = Modifier.padding(end = 4.dp), play = play, seek = seek)
    }
}

@Composable
internal fun ComposerBar(content: @Composable RowScope.() -> Unit) {
    val occlusion = LocalMaterialOcclusion.current
    Row(Modifier.fillMaxWidth().onGloballyPositioned { occlusion?.input = it.boundsInWindow() }.padding(8.dp), verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(8.dp), content = content)
}
