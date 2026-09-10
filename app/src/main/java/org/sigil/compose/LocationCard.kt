package org.sigil.compose

import androidx.compose.animation.core.*
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.*
import androidx.lifecycle.*
import androidx.lifecycle.compose.LocalLifecycleOwner
import kotlinx.coroutines.delay
import org.sigil.*

@Composable
internal fun LocationCard(message: ChatMessage, part: MessagePart, name: String, command: ((String, Map<String, Any?>) -> Unit)?) {
    var opened by remember(message.id, part.id) { mutableStateOf(false) }
    var mapFailed by remember(message.id, part.id) { mutableStateOf(false) }
    var recenter by remember(message.id, part.id) { mutableIntStateOf(0) }
    var now by remember { mutableLongStateOf(System.currentTimeMillis() / 1000) }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    LaunchedEffect(part.sampledAt, part.until, part.stopped, lifecycle) {
        if (part.locationMode == "live" && !part.stopped) lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (true) { now = System.currentTimeMillis() / 1000; delay(10_000) }
        }
    }
    val active = part.locationMode == "live" && !part.stopped && part.until?.let { now < it } == true
    val fresh = active && now >= part.sampledAt && now - part.sampledAt <= 60
    val title = when (part.locationMode) { "live" -> if (active) "Live location" else "Location sharing ended"; "once" -> "Shared location"; else -> "Dropped pin" }
    val status = when {
        part.locationMode != "live" -> "Sent ${message.time}"
        !active -> "Last shared ${locationTime(part.sampledAt)}"
        !fresh -> "Last updated ${locationTime(part.sampledAt)} · waiting for an update"
        else -> "Updating · until ${locationTime(part.until!!)}"
    }
    val coordinates = String.format(java.util.Locale.ROOT, "%.5f, %.5f", part.latitude, part.longitude)
    val stop = { command?.invoke("location_stop", mapOf("peer" to message.peer, "author" to message.author, "message" to message.id, "card" to part.id)); Unit }
    Column(Modifier.widthIn(min = 200.dp, max = 280.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Box(Modifier.fillMaxWidth().height(160.dp).clip(RoundedCornerShape(16.dp)).background(MaterialTheme.colorScheme.surfaceVariant)
            .clickable(role = Role.Button) { opened = true }.semantics { contentDescription = "Open $title" }, contentAlignment = Alignment.Center) {
            if (!mapFailed && !opened) ServerMap(Modifier.matchParentSize().clearAndSetSemantics {}, part.latitude, part.longitude, movable = false, marker = if (part.locationMode != "pin") { { LocationAvatar(name, message.author, fresh) } } else null, failure = { mapFailed = true })
            if (part.locationMode != "pin" && (mapFailed || opened)) LocationAvatar(name, message.author, fresh)
            else if (mapFailed) Glyph("location_on", 36)
            // The map's native view must not consume the card's open action.
            Box(Modifier.matchParentSize().clickable { opened = true }.clearAndSetSemantics {})
        }
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) { Glyph(if (part.locationMode == "live") "location_searching" else "location_on", 18); Text(title, style = MaterialTheme.typography.labelMedium) }
        part.rich?.let { RichMessageText(it) } ?: Text(part.text)
        Text(status, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        if (mapFailed) Text(coordinates, style = MaterialTheme.typography.bodySmall)
        if (active && part.canStop && command != null) SigilTextButton(stop) { Text("Stop sharing") }
    }
    if (opened) Dialog({ opened = false }, DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            Column(Modifier.fillMaxSize().systemBarsPadding().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) { SigilIconButton({ opened = false }) { Glyph("close", 24, "Close map") }; Text(title, Modifier.weight(1f), style = MaterialTheme.typography.titleLarge) }
                Box(Modifier.weight(1f).fillMaxWidth().clip(RoundedCornerShape(24.dp)).background(MaterialTheme.colorScheme.surfaceVariant), contentAlignment = Alignment.Center) {
                    if (!mapFailed) ServerMap(Modifier.matchParentSize(), part.latitude, part.longitude, recenter = recenter, marker = if (part.locationMode != "pin") { { LocationAvatar(name, message.author, fresh) } } else null, failure = { mapFailed = true })
                    else Text("Map unavailable\n$coordinates", Modifier.padding(24.dp))
                }
                Column(Modifier.heightIn(max = 220.dp).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        if (part.locationMode != "pin") LocationAvatar(name, message.author, fresh)
                        Column { Text(name, style = MaterialTheme.typography.titleMedium); Text(status, style = MaterialTheme.typography.bodySmall) }
                    }
                    part.rich?.let { RichMessageText(it) } ?: Text(part.text)
                    SelectionContainerText(coordinates)
                    if (!mapFailed) SigilOutlinedButton({ recenter++ }) { Glyph("center_focus_strong", 22); Spacer(Modifier.width(8.dp)); Text("Recenter") }
                    part.accuracyCm?.let { Text("Accuracy about ${maxOf(1, it / 100)} m", style = MaterialTheme.typography.bodySmall) }
                    if (active && part.canStop && command != null) SigilButton(stop) { Text("Stop sharing") }
                }
            }
        }
    }
}
private fun locationTime(seconds: Long) = java.text.DateFormat.getTimeInstance(java.text.DateFormat.SHORT).format(java.util.Date(seconds * 1000))
@Composable
private fun SelectionContainerText(text: String) { androidx.compose.foundation.text.selection.SelectionContainer { Text(text, style = MaterialTheme.typography.bodySmall) } }
@Composable
internal fun LocationAvatar(name: String, photo: String, fresh: Boolean) {
    val motion = LocalMotion.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    var visible by remember { mutableStateOf(lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)) }
    DisposableEffect(lifecycle) {
        val observer = LifecycleEventObserver { _, _ -> visible = lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED) }
        lifecycle.addObserver(observer); onDispose { lifecycle.removeObserver(observer) }
    }
    val pulse = if (fresh && visible && !motion.reduced) {
        val transition = rememberInfiniteTransition(label = "Location signal")
        transition.animateFloat(0f, 1f, infiniteRepeatable(tween(1800, easing = LinearEasing)), label = "Radio ring").value
    } else 0f
    val accent = MaterialTheme.colorScheme.primary
    Box(Modifier.size(64.dp), contentAlignment = Alignment.Center) {
        if (fresh) Canvas(Modifier.matchParentSize()) {
            for (shift in listOf(0f, .5f)) {
                val progress = (pulse + shift) % 1f
                drawCircle(accent.copy(alpha = (1f - progress) * .6f), radius = (20 + progress * 12).dp.toPx(), style = Stroke(1.5.dp.toPx()))
            }
        }
        Surface(Modifier.size(40.dp), shape = CircleShape, color = MaterialTheme.colorScheme.surfaceVariant, border = BorderStroke(2.dp, MaterialTheme.colorScheme.surface)) {
            Box(contentAlignment = Alignment.Center) { Text(name.take(1).uppercase()); LocalProfilePhoto.current(photo, Modifier.matchParentSize()) }
        }
    }
}
