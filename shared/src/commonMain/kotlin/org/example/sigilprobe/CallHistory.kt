package org.sigil

import androidx.compose.animation.*
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp


internal fun callName(call: CallSummary) = call.name.ifEmpty { call.participants.filter { !it.own }.joinToString(", ") { it.name }.ifEmpty { "Call" } }
internal fun callStatus(call: CallSummary) = when {
    call.missed -> "Missed"
    call.phase == "ringing" -> "Incoming call"
    call.phase == "joining" -> "Connecting"
    call.phase == "active" -> "In progress"
    call.phase == "declined" -> "Declined"
    call.outgoing -> "Outgoing"
    else -> "Incoming"
}
internal fun callContact(call: CallSummary, chats: List<ChatSummary>): ChatSummary? {
    if (!call.direct) return null
    val others = call.participants.filterNot { it.own }
    return chats.filter { chat -> chat.id != "self" && !chat.hidden && !chat.archived && others.any { it.peer == chat.id || it.address.isNotBlank() && it.address == chat.address } }.singleOrNull()
}
private fun callLength(seconds: Long) = if (seconds >= 3600) "${seconds / 3600}:${((seconds / 60) % 60).toString().padStart(2, '0')}:${(seconds % 60).toString().padStart(2, '0')}" else "${seconds / 60}:${(seconds % 60).toString().padStart(2, '0')}"
private fun callDescription(call: CallSummary, active: ActiveCall?) = listOfNotNull(callStatus(call), (if (active?.call?.id == call.id && active.connection == "connected") active.seconds else call.duration)?.let(::callLength)).joinToString(" · ")
private fun callDirection(call: CallSummary) = if (call.missed) "call_missed" else if (call.outgoing) "call_made" else "call_received"

@Composable
internal fun CallHistoryPage(state: MessengerState, command: Command, selectedCallId: String? = null, select: (String?) -> Unit = {}) {
    var missedOnly by rememberSaveable { mutableStateOf(false) }
    val features = LocalClientFeatures.current
    val selected = state.calls.firstOrNull { it.id == selectedCallId }
    fun activate(call: CallSummary, video: Boolean = false) {
        if (!features.calls || state.busy || state.phase != "connected" || video && !features.videoCalls) return
        when (call.phase) {
            "active", "joining" -> command("call_resume", mapOf("call" to call.id))
            "ringing" -> command("call_answer", mapOf("call" to call.id, "video" to video))
            else -> if (state.call == null) command("call_redial", mapOf("call" to call.id, "video" to video, "name" to callName(call)))
        }
    }
    fun enabled(call: CallSummary) = features.calls && !state.busy && state.phase == "connected" && (state.call == null || state.call.call.id == call.id)
    val motionPolicy = LocalMotion.current
    // The details slide in over the register and slide back out.
    AnimatedContent(selected, Modifier.fillMaxSize(), contentKey = { it != null }, transitionSpec = {
        if (targetState != null) (slideInHorizontally(motionPolicy.enter(MotionMillis)) { it } + fadeIn(motionPolicy.enter(MotionMillis))) togetherWith (slideOutHorizontally(motionPolicy.exit(MotionMillis)) { -it / 4 } + fadeOut(motionPolicy.exit(MotionExit)))
        else (slideInHorizontally(motionPolicy.enter(MotionMillis)) { -it / 4 } + fadeIn(motionPolicy.enter(MotionMillis))) togetherWith (slideOutHorizontally(motionPolicy.exit(MotionMillis)) { it } + fadeOut(motionPolicy.exit(MotionExit)))
    }, label = "Call pages") { shown ->
    if (shown != null) {
        val selected = shown
        val person = callContact(selected, state.chats)
        val related = state.calls.filter { call -> call.id == selected.id || person != null && callContact(call, state.chats)?.id == person.id }.sortedByDescending { it.created }
        LazyColumn(Modifier.fillMaxSize(), contentPadding = LocalHomeContentPadding.current) {
            item(key = "detail-header") {
                Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 24.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Avatar(person?.name ?: callName(selected), 80, person?.avatar.orEmpty())
                    Text(person?.name ?: callName(selected), style = MaterialTheme.typography.headlineSmall,
                        maxLines = 2, overflow = TextOverflow.Ellipsis, textAlign = TextAlign.Center)
                    // The latest call, in a line: when it was, and how it went.
                    related.firstOrNull()?.let { last ->
                        Text(listOfNotNull(if (last.day.isNotBlank()) "Last call ${if (last.day in listOf("Today", "Yesterday")) last.day.lowercase() else last.day}" else null, last.time.takeIf { it.isNotBlank() }).joinToString(", "),
                            style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = TextAlign.Center)
                    }
                    Row(Modifier.fillMaxWidth().padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        CallDetailAction("call", when (selected.phase) { "active", "joining" -> "Return to call"; "ringing" -> "Answer"; else -> "Audio call" }, enabled(selected), primary = true) { activate(selected) }
                        if (features.videoCalls && selected.phase !in listOf("active", "joining", "ringing")) CallDetailAction("videocam", "Video call", enabled(selected)) { activate(selected, true) }
                        if (person != null) CallDetailAction("chat_bubble", "Message", !state.busy) { select(null); command("open", mapOf("peer" to person.id)) }
                    }
                }
            }
            callRows(related, state, person, ::enabled, ::activate, select = null)
        }
    } else {
        val calls = state.calls.filter { !missedOnly || it.missed }.sortedByDescending { it.created }
        LazyColumn(Modifier.fillMaxSize(), contentPadding = LocalHomeContentPadding.current) {
            item(key = "filters") {
                Row(Modifier.padding(horizontal = 24.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    FilterChip(!missedOnly, { missedOnly = false }, { Text("All") }, shape = RoundedCornerShape(12.dp))
                    FilterChip(missedOnly, { missedOnly = true }, { Text("Missed") }, shape = RoundedCornerShape(12.dp))
                }
            }
            if (calls.isEmpty()) item("empty") { Box(Modifier.fillMaxWidth().padding(32.dp), contentAlignment = Alignment.Center) { Text(if (missedOnly) "No missed calls." else "Your calls will appear here.", color = MaterialTheme.colorScheme.onSurfaceVariant) } }
            callRows(calls, state, null, ::enabled, ::activate, select)
        }
    }
    }
}

// The register: day labels in small capitals, then a row per call with the person, one quiet line of what happened, the time, and a key to call back.
private fun LazyListScope.callRows(calls: List<CallSummary>, state: MessengerState, person: ChatSummary?, enabled: (CallSummary) -> Boolean, activate: (CallSummary, Boolean) -> Unit, select: ((String) -> Unit)?) {
    var previousDay: String? = null
    calls.forEach { call ->
        if (call.day.isNotBlank() && call.day != previousDay) { item(key = "day:${call.id}") { CallDay(call.day, itemMotion()) }; previousDay = call.day }
        item(key = call.id) {
            val who = person ?: callContact(call, state.chats)
            val features = LocalClientFeatures.current
            Row(itemMotion().fillMaxWidth().padding(horizontal = 12.dp, vertical = 2.dp), verticalAlignment = Alignment.CenterVertically) {
                Row(Modifier.weight(1f).clip(RoundedCornerShape(18.dp)).then(if (select != null) Modifier.clickable(role = Role.Button) { select(call.id) } else Modifier)
                    .heightIn(min = 64.dp).padding(horizontal = 12.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    Avatar(who?.name ?: callName(call), 48, who?.avatar.orEmpty())
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                        Text(who?.name ?: callName(call), style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        CallMeta(call, state.call)
                    }
                    if (call.time.isNotBlank()) Text(call.time, style = MaterialTheme.typography.labelSmall.copy(fontFeatureSettings = "tnum"), maxLines = 1, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                CallKey(if (call.video == true) "videocam" else "call", when (call.phase) { "active", "joining" -> "Return to call with ${callName(call)}"; "ringing" -> "Answer ${callName(call)}"; else -> "Call ${callName(call)}" }, enabled(call)) { activate(call, call.video == true && features.videoCalls) }
            }
        }
    }
}

@Composable private fun CallDay(day: String, modifier: Modifier = Modifier) {
    // Set in small capitals, read aloud as written.
    Text(day.uppercase(), modifier.padding(start = 24.dp, end = 24.dp, top = 20.dp, bottom = 6.dp).clearAndSetSemantics { text = AnnotatedString(day) }, style = MaterialTheme.typography.labelSmall.copy(letterSpacing = 1.4.sp), color = MaterialTheme.colorScheme.onSurfaceVariant)
}
// What happened, in the quiet ink; a missed call reads by its word and glyph, never by a warning colour.
@Composable private fun CallMeta(call: CallSummary, active: ActiveCall?) {
    CompositionLocalProvider(LocalContentColor provides MaterialTheme.colorScheme.onSurfaceVariant) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(5.dp)) {
            Glyph(callDirection(call), 16)
            Text(callDescription(call, active), style = MaterialTheme.typography.bodySmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}
// The call-back key: a squircle tile, like the other tool keys.
@Composable private fun CallKey(icon: String, label: String, enabled: Boolean, click: () -> Unit) {
    val ink = MaterialTheme.colorScheme.onSurfaceVariant.let { if (enabled) it else it.copy(alpha = .38f) }
    Surface(onClick = click, enabled = enabled, modifier = Modifier.size(44.dp).semantics { contentDescription = label }, shape = RoundedCornerShape(14.dp), color = MaterialTheme.colorScheme.surfaceVariant, contentColor = ink) {
        Box(contentAlignment = Alignment.Center) { Glyph(icon, 22) }
    }
}
@Composable private fun RowScope.CallDetailAction(icon: String, label: String, enabled: Boolean, primary: Boolean = false, click: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    val ink = (if (primary) scheme.onPrimary else scheme.onSurface).let { if (enabled) it else it.copy(alpha = .38f) }
    Column(Modifier.weight(1f).padding(horizontal = 4.dp).clip(RoundedCornerShape(22.dp)).clickable(enabled = enabled, role = Role.Button, onClick = click).semantics { contentDescription = label },
        horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Surface(Modifier.widthIn(max = 64.dp).fillMaxWidth().aspectRatio(1f), shape = RoundedCornerShape(20.dp),
            color = if (primary && enabled) scheme.primary else scheme.surfaceVariant, contentColor = ink) { Box(contentAlignment = Alignment.Center) { Glyph(icon, 30) } }
        Text(label, style = MaterialTheme.typography.labelMedium, color = scheme.onSurface.let { if (enabled) it else it.copy(alpha = .38f) }, textAlign = TextAlign.Center, maxLines = 2, overflow = TextOverflow.Ellipsis)
    }
}
