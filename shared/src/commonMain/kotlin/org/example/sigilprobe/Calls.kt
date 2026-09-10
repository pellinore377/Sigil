package org.sigil

import androidx.compose.animation.core.*
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.grid.*
import androidx.compose.foundation.shape.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlin.math.*

val LocalCallVideo = staticCompositionLocalOf<@Composable (String, Boolean, Modifier) -> Unit> { { _, _, modifier -> Box(modifier, contentAlignment = Alignment.Center) { Text("Waiting for video…") } } }
@Composable
internal fun CallPage(active: ActiveCall, contacts: List<ChatSummary>, command: Command, ownPhoto: String = "", panel: String, setPanel: (String) -> Unit) {
    val call = active.call
    val incoming = call.phase == "ringing"
    val others = call.participants.filter { !it.own }
    fun photo(person: CallParticipant?): String = if (person?.own == true) ownPhoto else contacts.firstOrNull { it.id == person?.peer }?.avatar.orEmpty()
    val directPhoto = if (call.direct) photo(others.firstOrNull()) else ""
    val video = active.camera || active.screen || others.any { it.camera || it.screen }
    var speaker by remember(call.id) { mutableStateOf<String?>(null) }
    val speaking = call.participants.filter { it.audio }.maxByOrNull { active.levels[if (it.own) "self" else it.id] ?: 0f }
        ?.takeIf { (active.levels[if (it.own) "self" else it.id] ?: 0f) > .05f }?.id
    LaunchedEffect(speaking) { if (speaking != null) { kotlinx.coroutines.delay(600); speaker = speaking } }
    if (panel == "security") AlertDialog({ setPanel("") }, title = { Text("Call security") }, text = {
        Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Text("Audio, video, and screen sharing are end-to-end encrypted. Device verification helps confirm who is on the call.")
            call.participants.forEach { person ->
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(if (person.own) "You" else person.name, style = MaterialTheme.typography.titleMedium)
                    Text(if (person.own) "This device" else if (person.verified) "Known contact device" else "Authenticated by the call host", style = MaterialTheme.typography.bodySmall)
                    androidx.compose.foundation.text.selection.SelectionContainer { Text(person.fingerprint.chunked(4).joinToString(" "), fontFamily = LocalCodeFont.current, style = MaterialTheme.typography.bodySmall) }
                }
            }
            if (others.any { !it.verified }) Text("You can optionally compare fingerprints from that person's conversation settings.")
        }
    }, confirmButton = { SigilTextButton({ setPanel("") }) { Text("Done") } })
    if (panel == "invite") {
        var query by remember { mutableStateOf("") }
        val choices = contacts.filter { contact -> contact.id != "self" && !contact.group && contact.verified && contact.devices.none { it.blocked } && call.participants.none { it.peer == contact.id } && (contact.name.contains(query, true) || contact.address.contains(query, true)) }
        AlertDialog({ setPanel("") }, title = { Text("Invite to call") }, text = {
            Column {
                OutlinedTextField(query, { query = it }, label = { Text("Search contacts") }, singleLine = true)
                if (choices.isEmpty()) Text("No other verified contacts.", Modifier.padding(vertical = 16.dp))
                androidx.compose.foundation.lazy.LazyColumn(Modifier.heightIn(max = 320.dp)) {
                    items(choices.size) { index -> val person = choices[index]; SettingRow("person", person.name, person.address) { command("call_invite", mapOf("call" to call.id, "peer" to person.id)); setPanel("") } }
                }
            }
        }, confirmButton = { SigilTextButton({ setPanel("") }) { Text("Done") } })
    }
    Column(Modifier.fillMaxSize()) {
        if (others.any { !it.verified }) SigilTextButton({ setPanel("security") }, Modifier.padding(horizontal = 20.dp)) { Glyph("info", 18); Spacer(Modifier.width(8.dp)); Text("Some participants are new to you") }
        if (active.screen) Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp), verticalAlignment = Alignment.CenterVertically) {
            Glyph("present_to_all", 20); Spacer(Modifier.width(8.dp)); Text("Sharing your screen", Modifier.weight(1f))
            SigilTextButton({ command("call_screen", emptyMap()) }) { Text("Stop") }
        }
        Box(Modifier.weight(1f).fillMaxWidth().padding(20.dp), contentAlignment = Alignment.Center) {
            if (call.direct && !video) Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(28.dp)) {
                Box(Modifier.sizeIn(maxWidth = 270.dp, maxHeight = 270.dp).fillMaxWidth(.8f).aspectRatio(1f).border(5.dp, MaterialTheme.colorScheme.surfaceVariant, CircleShape), contentAlignment = Alignment.Center) { Avatar(active.name, 236, directPhoto) }
                CallWave(others.maxOfOrNull { active.levels[it.id] ?: 0f } ?: 0f, Modifier.fillMaxWidth(.72f).height(56.dp))
                Text(if (incoming) "Incoming call" else "Audio call", style = MaterialTheme.typography.titleMedium)
            } else if (!video) {
                val featured = call.participants.find { it.id == speaker } ?: others.firstOrNull() ?: call.participants.firstOrNull()
                Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(24.dp)) {
                    featured?.let { person ->
                        Avatar(person.name, 196, photo(person))
                        Text(if (person.own) "You" else person.name, style = MaterialTheme.typography.headlineSmall)
                        CallWave(active.levels[if (person.own) "self" else person.id] ?: 0f, Modifier.fillMaxWidth(.72f).height(48.dp))
                    }
                    LazyRow(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(20.dp, Alignment.CenterHorizontally), contentPadding = PaddingValues(vertical = 12.dp)) {
                        items(call.participants.filter { it.id != featured?.id }, key = { it.id }) { person ->
                            Column(Modifier.width(84.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(8.dp)) {
                                Box { Avatar(person.name, 64, photo(person)); if (!person.audio) Box(Modifier.align(Alignment.BottomEnd).background(MaterialTheme.colorScheme.background, CircleShape).padding(2.dp)) { Glyph("mic_off", 16, "Microphone off") } }
                                Text(if (person.own) "You" else person.name, style = MaterialTheme.typography.bodyMedium, textAlign = androidx.compose.ui.text.style.TextAlign.Center, maxLines = 2, overflow = TextOverflow.Ellipsis)
                            }
                        }
                    }
                }
            } else if (call.direct) {
                val remote = others.firstOrNull()
                if (remote != null && (remote.camera || remote.screen)) LocalCallVideo.current(remote.id, remote.screen, Modifier.fillMaxSize().clip(RoundedCornerShape(24.dp)))
                else Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Avatar(active.name, 120, directPhoto) }
                if (active.camera || active.screen) Box(Modifier.align(Alignment.BottomEnd).width(112.dp).height(168.dp).clip(RoundedCornerShape(20.dp))) { LocalCallVideo.current("self", active.screen, Modifier.fillMaxSize()) }
            } else LazyVerticalGrid(GridCells.Fixed(if (call.participants.size <= 2) 1 else 2), horizontalArrangement = Arrangement.spacedBy(10.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                items(call.participants, key = { it.id }) { person ->
                    val level = active.levels[if (person.own) "self" else person.id] ?: 0f
                    Surface(shape = RoundedCornerShape(22.dp), color = MaterialTheme.colorScheme.surfaceVariant, border = if (person.audio && level > .05f) BorderStroke(2.dp, MaterialTheme.colorScheme.primary) else null) {
                        Box(Modifier.fillMaxWidth().aspectRatio(.85f)) {
                            if (person.camera || person.screen) LocalCallVideo.current(if (person.own) "self" else person.id, person.screen, Modifier.fillMaxSize())
                            else Column(Modifier.align(Alignment.Center), horizontalAlignment = Alignment.CenterHorizontally) { Avatar(person.name, 74, photo(person)); Spacer(Modifier.height(16.dp)); CallWave(active.levels[if (person.own) "self" else person.id] ?: 0f, Modifier.width(90.dp).height(32.dp)) }
                            Row(Modifier.align(Alignment.BottomStart).fillMaxWidth().background(MaterialTheme.colorScheme.background.copy(alpha = .8f)).padding(10.dp), verticalAlignment = Alignment.CenterVertically) { Text(if (person.own) "You" else person.name, Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium); if (!person.audio) Glyph("mic_off", 18, "Microphone off") else CallWave(level, Modifier.width(24.dp).height(18.dp)) }
                        }
                    }
                }
            }
        }
        Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 24.dp), horizontalArrangement = Arrangement.SpaceEvenly) {
            if (incoming) {
                CallControl("call_end", "Decline", true) { command("call_decline", mapOf("call" to call.id)) }
                CallControl("call", "Answer") { command("call_answer", mapOf("call" to call.id)) }
            } else {
                CallControl(if (active.muted) "mic_off" else "mic", if (active.muted) "Unmute" else "Mute") { command("call_mute", emptyMap()) }
                if (video) CallControl(if (active.camera) "videocam" else "videocam_off", "Camera", state = if (active.camera) "On" else "Off") { command("call_camera", emptyMap()) }
                CallControl(if (active.speaker) "volume_up" else "hearing", if (active.speaker) "Earpiece" else "Speaker") { command("call_speaker", emptyMap()) }
                if (video) CallControl(if (active.screen) "stop_screen_share" else "present_to_all", if (active.screen) "Stop share" else "Share") { command("call_screen", emptyMap()) }
                else if (call.canInvite) CallControl("person_add", "Add person") { setPanel("invite") }
                CallControl("call_end", if (call.direct) "End" else "Leave", true) { command("call_end", mapOf("call" to call.id)) }
            }
        }
    }
}
@Composable
private fun RowScope.CallControl(icon: String, label: String, destructive: Boolean = false, state: String? = null, action: () -> Unit) {
    Column(Modifier.weight(1f).padding(horizontal = 4.dp).clip(RoundedCornerShape(22.dp)).clickable(role = Role.Button, onClickLabel = label, onClick = action).semantics { state?.let { stateDescription = it } }, horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Surface(Modifier.size(64.dp), shape = RoundedCornerShape(22.dp),
            color = if (destructive) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.surfaceVariant, contentColor = if (destructive) MaterialTheme.colorScheme.onError else MaterialTheme.colorScheme.onSurface) { Box(contentAlignment = Alignment.Center) { Glyph(icon, 30) } }
        Text(label, style = MaterialTheme.typography.bodyMedium, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
    }
}
@Composable
private fun CallWave(level: Float, modifier: Modifier) {
    val motionPolicy = LocalMotion.current
    val amplitude by animateFloatAsState(level.coerceIn(0f, 1f), motionPolicy.tween(100), label = "Voice level")
    val color = MaterialTheme.colorScheme.onSurfaceVariant
    Canvas(modifier.semantics { contentDescription = "Voice activity" }) {
        repeat(19) { i ->
            val envelope = 1f - abs(i - 9) / 10f
            val height = max(2.dp.toPx(), size.height * amplitude * envelope * (.55f + .45f * abs(sin(i * 1.7f))))
            val x = size.width * (i + .5f) / 19
            drawLine(color, Offset(x, center.y - height / 2), Offset(x, center.y + height / 2), 3.dp.toPx(), StrokeCap.Round)
        }
    }
}

@Composable
internal fun CallHeader(active: ActiveCall, contacts: List<ChatSummary>, ownPhoto: String, command: Command, minimize: () -> Unit, panel: (String) -> Unit) {
    val call = active.call
    val incoming = call.phase == "ringing"
    val others = call.participants.filter { !it.own }
    val person = others.firstOrNull()
    val directPhoto = if (call.direct) { if (person?.own == true) ownPhoto else contacts.firstOrNull { it.id == person?.peer }?.avatar.orEmpty() } else ""
    val video = active.camera || active.screen || others.any { it.camera || it.screen }
    var more by remember { mutableStateOf(false) }
        Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Symbol("chevron_left", "Minimize call", minimize)
            Avatar(active.name, 42, directPhoto)
            Column(Modifier.weight(1f).padding(start = 10.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) { Text(active.name, Modifier.weight(1f, false), maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.titleLarge); Spacer(Modifier.width(6.dp)); Glyph("lock", 16, "End-to-end encrypted call") }
                Text(if (incoming) "Incoming call" else if (call.phase == "joining") "Joining…" else if (call.direct && others.isEmpty()) "Calling…" else if (active.connection != "connected") active.connection.replaceFirstChar { it.uppercase() } + "…" else (if (call.direct) "" else "${call.participants.size} in call · ") + "${active.seconds / 60}:${(active.seconds % 60).toString().padStart(2, '0')}", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            if (!video) Symbol("videocam", "Turn camera on") { command("call_camera", emptyMap()) }
            else if (call.canInvite) Symbol("person_add", "Add person") { panel("invite") }
            Box {
                Symbol("more_vert", "Call options") { more = true }
                DropdownMenu(more, { more = false }) {
                    DropdownMenuItem({ Text("Call security") }, { more = false; panel("security") }, leadingIcon = { Glyph("lock") })
                    DropdownMenuItem({ Text(if (active.screen) "Stop sharing screen" else "Share screen") }, { more = false; command("call_screen", emptyMap()) }, leadingIcon = { Glyph("present_to_all") })
                    if (active.camera) DropdownMenuItem({ Text("Switch camera") }, { more = false; command("call_flip", emptyMap()) }, leadingIcon = { Glyph("cameraswitch") })
                }
            }
        }
}
