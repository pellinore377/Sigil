package org.sigil

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.launch

// What the screen needs from a platform player; positions are milliseconds.
interface TrackPlayback {
    val ready: Boolean
    val failed: Boolean
    val playing: Boolean
    val position: Long
    val duration: Long
    fun toggle()
    fun seek(ms: Long)
}

data class TrackPresentation(val title: String, val artist: String?, val album: String?, val art: ImageBitmap?, val lyrics: List<LyricLine>, val kind: String, val bytes: Long)

// The room: the artwork's colour fills the screen, the controls rest at the bottom, the lyrics wait below the fold.
@Composable fun TrackPlayerScreen(track: TrackPresentation, playback: TrackPlayback, close: () -> Unit, download: (() -> Unit)?, downloading: Boolean = false, caption: String? = null) {
    val scheme = MaterialTheme.colorScheme
    val tone = remember(track.art) { track.art?.let(::dominantColor) } ?: scheme.primary
    val room = lerp(tone, Color.Black, .55f)
    val floor = lerp(tone, Color.Black, .86f)
    val ink = Color.White
    val faint = ink.copy(alpha = .62f)
    val synced = track.lyrics.isNotEmpty() && track.lyrics.all { it.atMs != null }
    val current = if (synced) track.lyrics.indexOfLast { (it.atMs ?: 0) <= playback.position } else -1
    val list = rememberLazyListState()
    val scope = rememberCoroutineScope()
    val backdrop = rememberChromeBackdrop()
    var reach by remember { mutableStateOf(112.dp) }
    val navigation = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()
    BoxWithConstraints(Modifier.fillMaxSize().background(Brush.verticalGradient(listOf(room, floor)))) {
        val viewport = maxHeight
        val third = with(androidx.compose.ui.platform.LocalDensity.current) { (viewport * .35f).roundToPx() }
        // The live line is followed only once the reader has gone down to the lyrics; the first screen stays put.
        LaunchedEffect(current) { if (current >= 0 && list.firstVisibleItemIndex >= 1) list.animateScrollToItem(1 + current, -third) }
        CompositionLocalProvider(LocalContentColor provides ink) {
            LazyColumn(Modifier.fillMaxSize().captureBackdrop(backdrop), list, horizontalAlignment = Alignment.CenterHorizontally) {
                item {
                    Column(Modifier.fillMaxWidth().height(viewport).padding(top = reach + 12.dp, bottom = navigation + 12.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                        BoxWithConstraints(Modifier.weight(1f).fillMaxWidth().padding(horizontal = 36.dp, vertical = 12.dp), contentAlignment = Alignment.Center) {
                            val side = minOf(maxWidth, maxHeight, 380.dp)
                            Box(Modifier.size(side).clip(RoundedCornerShape(18.dp)).background(lerp(tone, Color.Black, .3f)), contentAlignment = Alignment.Center) {
                                if (track.art != null) Image(track.art, null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
                                else Box(Modifier.size(96.dp).background(ink.copy(alpha = .16f), CircleShape), contentAlignment = Alignment.Center) { Text("♫", fontSize = 44.sp, lineHeight = 44.sp) }
                            }
                        }
                        Column(Modifier.fillMaxWidth().padding(horizontal = 28.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                            Text(track.title, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold, maxLines = 2, overflow = TextOverflow.Ellipsis)
                            track.artist?.let { Text(it, style = MaterialTheme.typography.bodyLarge, color = faint, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                            if (!caption.isNullOrBlank()) Text(caption, Modifier.padding(top = 4.dp), style = MaterialTheme.typography.bodyMedium, color = faint, maxLines = 2, overflow = TextOverflow.Ellipsis)
                        }
                        val duration = playback.duration.coerceAtLeast(1)
                        var dragging by remember { mutableStateOf<Float?>(null) }
                        Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 10.dp)) {
                            Slider(dragging ?: (playback.position.toFloat() / duration).coerceIn(0f, 1f), { dragging = it }, Modifier.fillMaxWidth(), enabled = playback.ready,
                                onValueChangeFinished = { dragging?.let { playback.seek((it * duration).toLong()) }; dragging = null },
                                colors = SliderDefaults.colors(thumbColor = ink, activeTrackColor = ink, inactiveTrackColor = ink.copy(alpha = .28f)))
                            Row(Modifier.fillMaxWidth().padding(horizontal = 4.dp), horizontalArrangement = Arrangement.SpaceBetween) {
                                Text(trackTime(dragging?.let { (it * duration).toLong() } ?: playback.position), style = MaterialTheme.typography.labelSmall, color = faint)
                                Text(trackTime(playback.duration), style = MaterialTheme.typography.labelSmall, color = faint)
                            }
                        }
                        Row(Modifier.fillMaxWidth().padding(bottom = 8.dp), horizontalArrangement = Arrangement.spacedBy(28.dp, Alignment.CenterHorizontally), verticalAlignment = Alignment.CenterVertically) {
                            SigilIconButton({ playback.seek((playback.position - 10_000).coerceAtLeast(0)) }, enabled = playback.ready) { Glyph("replay_10", 30, "Back ten seconds") }
                            Box(Modifier.size(72.dp).background(ink, CircleShape).clickable(enabled = playback.ready) { playback.toggle() }, contentAlignment = Alignment.Center) {
                                CompositionLocalProvider(LocalContentColor provides room) {
                                    if (playback.failed) Glyph("error", 32, "Playback failed")
                                    else if (!playback.ready) CircularProgressIndicator(Modifier.size(28.dp), color = room, strokeWidth = 3.dp)
                                    else Glyph(if (playback.playing) "pause" else "play_arrow", 40, if (playback.playing) "Pause" else "Play", filled = true)
                                }
                            }
                            SigilIconButton({ playback.seek((playback.position + 10_000).coerceAtMost(playback.duration)) }, enabled = playback.ready) { Glyph("forward_10", 30, "Forward ten seconds") }
                        }
                        // The cue that there is more below: tapping it goes there.
                        if (track.lyrics.isNotEmpty()) Row(Modifier.clip(RoundedCornerShape(20.dp)).clickable { scope.launch { list.animateScrollToItem(1) } }.padding(horizontal = 16.dp, vertical = 6.dp),
                            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                            Text("Lyrics", style = MaterialTheme.typography.labelLarge, color = faint)
                            CompositionLocalProvider(LocalContentColor provides faint) { Glyph("keyboard_arrow_down", 20) }
                        }
                        else Spacer(Modifier.height(20.dp))
                    }
                }
                itemsIndexed(track.lyrics) { index, line ->
                    val live = synced && index == current
                    Text(line.text.ifEmpty { "…" }, Modifier.fillMaxWidth().then(if (synced) Modifier.clickable { line.atMs?.let(playback::seek) } else Modifier).padding(horizontal = 28.dp, vertical = 6.dp),
                        style = if (synced) MaterialTheme.typography.titleLarge else MaterialTheme.typography.bodyLarge, fontWeight = if (live) FontWeight.Bold else FontWeight.Medium,
                        color = if (!synced || live) ink else ink.copy(alpha = .42f), fontSize = if (synced) 22.sp else 17.sp, lineHeight = if (synced) 30.sp else 26.sp)
                }
                if (track.lyrics.isNotEmpty()) item { Spacer(Modifier.height(viewport * .4f)) }
            }
            // The timeline's header, floating over the room like it floats over the conversation.
            CompositionLocalProvider(LocalContentColor provides scheme.onSurface) {
                ViewerHeader(backdrop, Modifier.align(Alignment.TopCenter), { reach = it }) {
                    Symbol("chevron_left", "Back", close)
                    Column(Modifier.weight(1f).padding(start = 10.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(track.album ?: "Now playing", style = MaterialTheme.typography.titleLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text("${track.kind} · ${attachmentSize(track.bytes)}", style = MaterialTheme.typography.labelMedium, color = scheme.onSurfaceVariant, maxLines = 1)
                    }
                    download?.let { Symbol("download", if (downloading) "Saving track" else "Save track", it) }
                }
            }
        }
    }
}
