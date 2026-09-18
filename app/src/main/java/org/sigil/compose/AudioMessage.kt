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
import org.sigil.NativeCore
import org.sigil.AudioPlayback
import org.sigil.AudioWaveform
import org.sigil.PlaySquircle

@Composable
internal fun AudioMessage(message: ChatMessage) {
    val context = LocalContext.current
    // A memo loads on sight so the bubble carries its real waveform and length; play stays explicit.
    var requested by remember(message.id) { mutableStateOf(true) }
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
    // One fixed frame for every state, so loading and playing never resize the bubble.
    Column(Modifier.widthIn(max = 300.dp).fillMaxWidth().padding(start = 6.dp, end = 14.dp, top = 6.dp, bottom = 6.dp).testTag("audio-message")) {
        if (ready) InlineAudio(message)
        // Unloaded, the bubble already has the player's anatomy; play decrypts and prepares it.
        else Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            if (failed) SigilIconButton({ requested = true }) { Glyph("refresh", 24, "Retry audio") }
            else PlaySquircle({ requested = true }, false, !requested, false)
            AudioWaveform(emptyList(), Modifier.weight(1f).height(32.dp))
            if (failed) Text("Couldn't load", style = MaterialTheme.typography.labelMedium)
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
    var levels by remember(message.id) { mutableStateOf<List<Float>>(emptyList()) }
    var seeking by remember(message.id) { mutableStateOf(false) }
    DisposableEffect(player, lifecycle) {
        player.setOnPreparedListener {
            duration = it.duration.toLong().coerceAtLeast(0); ready = true
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
    else if (!ready) Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) { PlaySquircle({}, false, false, false); AudioWaveform(emptyList(), Modifier.weight(1f).height(32.dp)) }
    else AudioPlayback(position, duration, playing, levels, play = ::play, seek = ::seek)
}
