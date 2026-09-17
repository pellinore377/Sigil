package org.sigil.compose

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
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
            do { now = System.currentTimeMillis() / 1000; if (now < (part.until ?: now)) delay(1_000) } while (now < (part.until ?: now))
        }
    }
    val active = part.locationMode == "live" && !part.stopped && part.until?.let { now < it } == true
    val fresh = active && now >= part.sampledAt && now - part.sampledAt <= 60
    val title = when (part.locationMode) { "live" -> if (active) "Live location" else "Location sharing ended"; "once" -> "Shared location"; else -> "Dropped pin" }
    val remaining = locationRemaining(part.until, now, part.stopped)
    val stop = { command?.invoke("location_stop", mapOf("peer" to message.peer, "author" to message.author, "message" to message.id, "card" to part.id)); Unit }
    Column(Modifier.widthIn(min = 200.dp, max = 280.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Box(Modifier.fillMaxWidth().height(160.dp).clip(RoundedCornerShape(16.dp)).background(MaterialTheme.colorScheme.surfaceVariant)
            .clickable(role = Role.Button) { opened = true }.semantics { contentDescription = "Open $title" }, contentAlignment = Alignment.Center) {
            if (!mapFailed && !opened) ServerMap(Modifier.matchParentSize().clearAndSetSemantics {}, part.latitude, part.longitude, movable = false, marker = if (part.locationMode != "pin") { { LocationAvatar(name, message.author, fresh) } } else null, failure = { mapFailed = true })
            if (part.locationMode != "pin" && (mapFailed || opened)) LocationAvatar(name, message.author, fresh)
            else if (mapFailed) Glyph("place", 36)
            // The map's native view must not consume the card's open action.
            Box(Modifier.matchParentSize().clickable { opened = true }.clearAndSetSemantics {})
            if (part.locationMode == "live") LocationMapChip(remaining, Modifier.align(Alignment.TopStart).padding(10.dp))
        }
    }
    if (opened) Dialog({ opened = false }, DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            Box(Modifier.fillMaxSize().systemBarsPadding().padding(12.dp)) {
                Box(Modifier.fillMaxSize().clip(RoundedCornerShape(28.dp)).background(MaterialTheme.colorScheme.surfaceVariant), contentAlignment = Alignment.Center) {
                    if (!mapFailed) ServerMap(Modifier.matchParentSize(), part.latitude, part.longitude, recenter = recenter, marker = if (part.locationMode != "pin") { { LocationAvatar(name, message.author, fresh) } } else null, failure = { mapFailed = true })
                    else Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Text("Map unavailable")
                        SigilTextButton({ mapFailed = false }) { Text("Retry") }
                    }
                }
                Surface(Modifier.align(Alignment.TopCenter).padding(10.dp).fillMaxWidth(), shape = RoundedCornerShape(24.dp), color = MaterialTheme.colorScheme.surface.copy(alpha = .94f), contentColor = MaterialTheme.colorScheme.onSurface) {
                    Row(Modifier.padding(4.dp), verticalAlignment = Alignment.CenterVertically) {
                        SigilIconButton({ opened = false }) { Glyph("close", 24, "Close map") }
                        Column(Modifier.weight(1f)) {
                            Text(name, style = MaterialTheme.typography.titleMedium)
                            if (part.locationMode == "live") Text(remaining, style = MaterialTheme.typography.labelMedium)
                        }
                        SigilIconButton({ recenter++ }) { Glyph("my_location", 24, "Recenter map") }
                    }
                }
                if (active && part.canStop && command != null) Surface(Modifier.align(Alignment.BottomCenter).padding(bottom = 36.dp), shape = RoundedCornerShape(24.dp), color = MaterialTheme.colorScheme.surface.copy(alpha = .94f), contentColor = MaterialTheme.colorScheme.onSurface) {
                    SigilTextButton(stop) { Text("Stop sharing") }
                }
            }
        }
    }
}
