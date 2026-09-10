package org.sigil.compose

import org.sigil.SigilIconButton

import android.media.MediaPlayer
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import kotlinx.coroutines.*
import org.sigil.ChatMessage
import org.sigil.Glyph
import org.sigil.NativeCore
import org.sigil.AudioPlayback
import org.sigil.AudioWaveform
import org.sigil.SigilTextButton

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
    var expanded by remember(message.id) { mutableStateOf(false) }
    var speed by remember(message.id) { mutableFloatStateOf(1f) }
    var levels by remember(message.id) { mutableStateOf<List<Float>>(emptyList()) }
    var seeking by remember(message.id) { mutableStateOf(false) }
    var inlineHeight by remember { mutableStateOf(80.dp) }
    var playbackIssue by remember { mutableStateOf<String?>(null) }
    val density = LocalDensity.current
    DisposableEffect(player, lifecycle) {
        player.setOnPreparedListener {
            duration = it.duration.toLong().coerceAtLeast(0); ready = true
            if (lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)) { it.start(); playing = true }
        }
        player.setOnCompletionListener { playing = false; position = duration }
        player.setOnSeekCompleteListener { seeking = false; position = it.currentPosition.toLong() }
        player.setOnErrorListener { _, _, _ -> failed = true; playing = false; true }
        try { player.setDataSource(media); player.prepareAsync() } catch (_: Exception) { failed = true }
        val observer = LifecycleEventObserver { _, event -> if (event == Lifecycle.Event.ON_STOP && ready && !failed) { player.pause(); playing = false } }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer); player.release(); media.close() }
    }
    LaunchedEffect(ready, playing, failed) { while (ready && playing && !failed) { if (!seeking) position = player.currentPosition.toLong(); delay(150) } }
    LaunchedEffect(ready, duration) {
        if (ready && message.attachment!!.name == "Voice message.aac") try { levels = withContext(Dispatchers.Default) { audioWaveform(context, message, duration) } }
        catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { levels = emptyList() }
    }
    fun play() {
        if (playing) player.pause() else { if (position >= duration) { player.seekTo(0); position = 0 }; player.start() }
        playing = !playing
    }
    fun seek(value: Long) { position = value; seeking = true; player.seekTo(value, MediaPlayer.SEEK_CLOSEST) }
    if (failed) Text("Couldn't play this audio", style = MaterialTheme.typography.bodySmall)
    else if (!ready) LinearProgressIndicator(Modifier.fillMaxWidth())
    else if (!expanded) AudioPlayback(position, duration, playing, levels, modifier = Modifier.onSizeChanged { inlineHeight = with(density) { it.height.toDp() } }, play = ::play, seek = ::seek, expand = { expanded = true })
    else Spacer(Modifier.fillMaxWidth().height(inlineHeight))
    if (expanded) androidx.compose.ui.window.Dialog({ expanded = false }) {
        Surface(shape = androidx.compose.foundation.shape.RoundedCornerShape(24.dp)) {
            Column(Modifier.fillMaxWidth().padding(20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(if (message.attachment!!.name == "Voice message.aac") "Voice message" else message.attachment!!.name, style = MaterialTheme.typography.titleMedium)
                if (levels.isNotEmpty()) AudioWaveform(levels, Modifier.fillMaxWidth().height(96.dp), position.toFloat() / duration.coerceAtLeast(1))
                AudioPlayback(position, duration, playing, levels, enabled = ready && !failed, play = ::play, seek = ::seek)
                @OptIn(ExperimentalLayoutApi::class)
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    listOf(1f, 1.5f, 2f).forEach { rate -> FilterChip(speed == rate, {
                        try { player.playbackParams = player.playbackParams.setSpeed(rate); if (!playing) player.pause(); speed = rate; playbackIssue = null }
                        catch (_: Exception) { playbackIssue = "Playback speed is unavailable for this file." }
                    }, label = { Text("${rate}×") }, shape = androidx.compose.foundation.shape.RoundedCornerShape(16.dp)) }
                }
                playbackIssue?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
                if (message.attachment!!.caption.isNotBlank()) org.sigil.MessageText(message.attachment!!.caption, NativeCore::analyze)
                SigilTextButton({ expanded = false }, Modifier.align(Alignment.End)) { Text("Close") }
            }
        }
    }
}
