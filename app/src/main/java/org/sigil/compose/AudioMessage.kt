package org.sigil.compose

import org.sigil.SigilIconButton

import android.media.MediaPlayer
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import kotlinx.coroutines.*
import org.sigil.ChatMessage
import org.sigil.Glyph

@Composable
internal fun AudioMessage(message: ChatMessage) {
    val context = LocalContext.current
    var requested by remember(message.id) { mutableStateOf(false) }
    var ready by remember(message.id) { mutableStateOf(false) }
    var failed by remember(message.id) { mutableStateOf(false) }
    LaunchedEffect(message.id, requested) {
        if (requested) try {
            failed = false
            while (!withContext(Dispatchers.IO) { prepare(context, message) }) delay(1000)
            ready = true
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { failed = true; requested = false }
    }
    Column(Modifier.widthIn(min = 220.dp, max = 300.dp).testTag("audio-message")) {
        if (ready) InlineAudio(message)
        else Row(verticalAlignment = Alignment.CenterVertically) {
            SigilIconButton({ requested = true }, enabled = !requested) { Glyph(if (failed) "refresh" else "play_arrow", 28, if (failed) "Retry audio" else "Play audio message") }
            Column(Modifier.weight(1f)) {
                Text(if (message.attachment!!.name == "Voice message.aac") "Voice message" else message.attachment!!.name, style = MaterialTheme.typography.bodyMedium, maxLines = 2)
                if (requested) LinearProgressIndicator(Modifier.fillMaxWidth().padding(top = 8.dp))
                else Text(if (failed) "Couldn't load audio" else "Tap to play", style = MaterialTheme.typography.labelSmall)
            }
        }
    }
}

@Composable
private fun InlineAudio(message: ChatMessage) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val media = remember(message.id) { EncryptedMedia(context, message) }
    val player = remember(message.id) { MediaPlayer() }
    var ready by remember(message.id) { mutableStateOf(false) }
    var playing by remember(message.id) { mutableStateOf(false) }
    var failed by remember(message.id) { mutableStateOf(false) }
    var duration by remember(message.id) { mutableLongStateOf(0) }
    var position by remember(message.id) { mutableLongStateOf(0) }
    var seeking by remember(message.id) { mutableStateOf(false) }
    DisposableEffect(player, lifecycle) {
        player.setOnPreparedListener {
            duration = it.duration.toLong().coerceAtLeast(0); ready = true
            if (lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)) { it.start(); playing = true }
        }
        player.setOnCompletionListener { playing = false; position = duration }
        player.setOnErrorListener { _, _, _ -> failed = true; playing = false; true }
        try { player.setDataSource(media); player.prepareAsync() } catch (_: Exception) { failed = true }
        val observer = LifecycleEventObserver { _, event -> if (event == Lifecycle.Event.ON_STOP && ready && !failed) { player.pause(); playing = false } }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer); player.release(); media.close() }
    }
    LaunchedEffect(ready, playing, failed) { while (ready && playing && !failed) { if (!seeking) position = player.currentPosition.toLong(); delay(150) } }
    Row(verticalAlignment = Alignment.CenterVertically) {
        SigilIconButton({
            if (playing) player.pause() else { if (position >= duration) { player.seekTo(0); position = 0 }; player.start() }
            playing = !playing
        }, enabled = ready && !failed) { Glyph(if (playing) "pause" else "play_arrow", 28, if (playing) "Pause audio message" else "Play audio message") }
        Column(Modifier.weight(1f)) {
            if (failed) Text("Couldn't play this audio", style = MaterialTheme.typography.bodySmall)
            else if (!ready) LinearProgressIndicator(Modifier.fillMaxWidth())
            else {
                Slider(position.toFloat().coerceIn(0f, duration.toFloat()), { seeking = true; position = it.toLong() }, Modifier.testTag("audio-seek"),
                    valueRange = 0f..duration.coerceAtLeast(1).toFloat(), onValueChangeFinished = { player.seekTo(position, MediaPlayer.SEEK_CLOSEST); seeking = false })
                fun time(ms: Long): String { val seconds = ms / 1000; return "${seconds / 60}:${(seconds % 60).toString().padStart(2, '0')}" }
                Text("${time(position)} / ${time(duration)}", style = MaterialTheme.typography.labelSmall)
            }
        }
    }
}
