package org.sigil

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
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

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
    if (selected != null) {
        val person = callContact(selected, state.chats)
        val related = state.calls.filter { call -> call.id == selected.id || person != null && callContact(call, state.chats)?.id == person.id }.sortedByDescending { it.created }
        LazyColumn(Modifier.fillMaxSize(), contentPadding = LocalHomeContentPadding.current) {
            item {
                Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 24.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Avatar(person?.name ?: callName(selected), 80, person?.avatar.orEmpty())
                    Text(person?.name ?: callName(selected), style = MaterialTheme.typography.headlineSmall)
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        CallDetailAction("call", when (selected.phase) { "active", "joining" -> "Return to call"; "ringing" -> "Answer"; else -> "Audio call" }, enabled(selected)) { activate(selected) }
                        if (features.videoCalls && selected.phase !in listOf("active", "joining", "ringing")) CallDetailAction("videocam", "Video call", enabled(selected)) { activate(selected, true) }
                        if (person != null) CallDetailAction("chat_bubble", "Message", !state.busy) { select(null); command("open", mapOf("peer" to person.id)) }
                    }
                }
            }
            callHistoryItems(related, state.call)
        }
    } else {
        val calls = state.calls.filter { !missedOnly || it.missed }.sortedByDescending { it.created }
        LazyColumn(Modifier.fillMaxSize(), contentPadding = LocalHomeContentPadding.current) {
            item {
                Row(Modifier.padding(horizontal = 20.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    FilterChip(!missedOnly, { missedOnly = false }, { Text("All") })
                    FilterChip(missedOnly, { missedOnly = true }, { Text("Missed") })
                }
            }
            if (calls.isEmpty()) item { Box(Modifier.fillMaxWidth().padding(32.dp), contentAlignment = Alignment.Center) { Text(if (missedOnly) "No missed calls." else "Your calls will appear here.", color = MaterialTheme.colorScheme.onSurfaceVariant) } }
            var previousDay: String? = null
            calls.forEach { call ->
                if (call.day.isNotBlank() && call.day != previousDay) { item(key = "day:${call.id}") { CallDay(call.day) }; previousDay = call.day }
                item(key = call.id) {
                    val person = callContact(call, state.chats)
                    Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 2.dp), verticalAlignment = Alignment.CenterVertically) {
                        Row(Modifier.weight(1f).clip(RoundedCornerShape(20.dp)).clickable { select(call.id) }.padding(12.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            Avatar(person?.name ?: callName(call), 48, person?.avatar.orEmpty())
                            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(5.dp)) {
                                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                    Text(person?.name ?: callName(call), Modifier.weight(1f), style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                    if (call.time.isNotBlank()) Text(call.time, style = MaterialTheme.typography.labelSmall, maxLines = 1, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                }
                                CallMeta(call, state.call)
                            }
                        }
                        SigilIconButton({ activate(call, call.video == true && features.videoCalls) }, enabled = enabled(call)) { Glyph(if (call.video == true) "videocam" else "call", 24, when (call.phase) { "active", "joining" -> "Return to call with ${callName(call)}"; "ringing" -> "Answer ${callName(call)}"; else -> "Call ${callName(call)}" }) }
                    }
                }
            }
        }
    }
}

@Composable private fun CallDay(day: String) { Text(day, Modifier.padding(start = 24.dp, end = 24.dp, top = 20.dp, bottom = 8.dp), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant) }
@Composable private fun CallMeta(call: CallSummary, active: ActiveCall?) {
    val ink = if (call.missed) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant
    CompositionLocalProvider(LocalContentColor provides ink) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(5.dp)) {
            Glyph(callDirection(call), 17)
            Text(callDescription(call, active), style = MaterialTheme.typography.bodySmall)
        }
    }
}
@Composable private fun RowScope.CallDetailAction(icon: String, label: String, enabled: Boolean, click: () -> Unit) {
    Column(Modifier.weight(1f), horizontalAlignment = Alignment.CenterHorizontally) {
        FilledTonalIconButton(click, enabled = enabled, modifier = Modifier.size(48.dp)) { Glyph(icon, 24, label) }
        Text(label, Modifier.padding(top = 6.dp), style = MaterialTheme.typography.labelSmall, textAlign = androidx.compose.ui.text.style.TextAlign.Center, maxLines = 2)
    }
}
private fun LazyListScope.callHistoryItems(calls: List<CallSummary>, active: ActiveCall?) {
    var previousDay: String? = null
    calls.forEach { call ->
        if (call.day.isNotBlank() && call.day != previousDay) { item(key = "day:${call.id}") { CallDay(call.day) }; previousDay = call.day }
        item(key = call.id) {
            Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 14.dp), verticalArrangement = Arrangement.spacedBy(7.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    Glyph(if (call.video == true) "videocam" else "call", 24)
                    Text(when (call.video) { true -> "Video call"; false -> "Audio call"; null -> "Call" }, style = MaterialTheme.typography.titleMedium)
                    Spacer(Modifier.weight(1f))
                    if (call.time.isNotBlank()) Text(call.time, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                CallMeta(call, active)
            }
        }
    }
}
