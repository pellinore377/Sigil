package org.sigil

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.animateScrollBy
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.layout.positionInParent
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
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

private const val LyricsGlide = 900

// A thin track with a small round thumb; tap or drag anywhere along it.
@Composable fun SlimScrubber(fraction: Float, enabled: Boolean, ink: Color, onChange: (Float) -> Unit, onFinish: () -> Unit, modifier: Modifier = Modifier) {
    var width by remember { mutableIntStateOf(0) }
    val change by rememberUpdatedState(onChange)
    val finish by rememberUpdatedState(onFinish)
    Box(modifier.fillMaxWidth().height(28.dp).onSizeChanged { width = it.width }
        .pointerInput(enabled) { if (enabled) detectTapGestures { change((it.x / width.coerceAtLeast(1)).coerceIn(0f, 1f)); finish() } }
        .pointerInput(enabled) { if (enabled) detectHorizontalDragGestures(onDragEnd = { finish() }, onDragCancel = { finish() }) { event, _ -> change((event.position.x / width.coerceAtLeast(1)).coerceIn(0f, 1f)) } }) {
        Canvas(Modifier.fillMaxSize()) {
            val y = size.height / 2
            val stroke = 3.dp.toPx()
            drawLine(ink.copy(alpha = .26f), Offset(0f, y), Offset(size.width, y), stroke, StrokeCap.Round)
            drawLine(ink, Offset(0f, y), Offset(size.width * fraction, y), stroke, StrokeCap.Round)
            drawCircle(ink, 6.dp.toPx(), Offset(size.width * fraction, y))
        }
    }
}

// The room: the artwork's tone colours the screen in the current theme, the controls rest at the bottom, the lyrics wait below the fold.
@Composable fun TrackPlayerScreen(track: TrackPresentation, playback: TrackPlayback, close: () -> Unit, download: (() -> Unit)?, downloading: Boolean = false, caption: String? = null) {
    val scheme = MaterialTheme.colorScheme
    val dark = scheme.background.brightness() < .5f
    val tone = remember(track.art) { track.art?.let(::dominantColor) } ?: scheme.primary
    val room = lerp(tone, scheme.background, if (dark) .55f else .72f)
    val floor = lerp(tone, scheme.background, if (dark) .86f else .94f)
    val ink = scheme.onBackground
    val faint = scheme.onSurfaceVariant
    val synced = track.lyrics.isNotEmpty() && track.lyrics.all { it.atMs != null }
    val current = if (synced) track.lyrics.indexOfLast { (it.atMs ?: 0) <= playback.position } else -1
    val list = rememberLazyListState()
    val scope = rememberCoroutineScope()
    val backdrop = rememberChromeBackdrop()
    val density = LocalDensity.current
    var reach by remember { mutableStateOf(112.dp) }
    val navigation = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()
    val lineTops = remember { mutableMapOf<Int, Float>() }
    var controlsTop by remember { mutableFloatStateOf(0f) }
    var lyricsHeight by remember { mutableIntStateOf(0) }
    // A scrolled list is placed, not redrawn; reading its position in draw keeps the glass and its source current.
    val follow = Modifier.drawBehind { list.firstVisibleItemIndex; list.firstVisibleItemScrollOffset }
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val viewport = maxHeight
        val width = maxWidth
        val viewportPx = with(density) { viewport.toPx() }
        // Reading position: the transport row tucked under the pill, lyrics beneath it, the key still in reach.
        val lyricsTop = (controlsTop - with(density) { (reach + 12.dp).toPx() }).coerceAtLeast(0f)
        // Where the list stands, in pixels from the top: the page is exactly one viewport tall, the lyrics follow it.
        val absolute = when (list.firstVisibleItemIndex) { 0 -> list.firstVisibleItemScrollOffset.toFloat(); 1 -> viewportPx + list.firstVisibleItemScrollOffset; else -> viewportPx + lyricsHeight + list.firstVisibleItemScrollOffset }
        val inLyrics = absolute > lyricsTop * .5f
        val glide = tween<Float>(LyricsGlide, easing = FastOutSlowInEasing)
        // The page may only scroll as far as the reading position; nothing empty lies past the last line.
        val tail = with(density) { (lyricsTop - lyricsHeight).coerceAtLeast(0f).toDp() }
        // The live line is followed only once the reader has gone down to the lyrics; the first screen stays put.
        LaunchedEffect(current) { if (current >= 0 && inLyrics) lineTops[current]?.let { list.animateScrollBy((it - viewportPx * .45f).coerceIn(0f, lyricsTop) - absolute, tween(500)) } }
        CompositionLocalProvider(LocalContentColor provides ink) {
            // The capture sits on a plain box around the list, as the timeline's does; the list itself keeps its own layers.
            Box(Modifier.fillMaxSize().background(Brush.verticalGradient(listOf(room, floor))).then(follow).captureBackdrop(backdrop)) { LazyColumn(Modifier.fillMaxSize(), list, horizontalAlignment = Alignment.CenterHorizontally) {
                item { Column(Modifier.fillMaxWidth().height(viewport).padding(top = reach + 12.dp, bottom = navigation + 12.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    val side = minOf(width - 72.dp, viewport - reach - 330.dp, 380.dp).coerceAtLeast(120.dp)
                    Spacer(Modifier.weight(1f))
                    Box(Modifier.size(side).clip(RoundedCornerShape(18.dp)).background(lerp(tone, scheme.background, .3f)), contentAlignment = Alignment.Center) {
                        if (track.art != null) Image(track.art, null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
                        else Box(Modifier.size(96.dp).background(ink.copy(alpha = .14f), SquircleShape), contentAlignment = Alignment.Center) { Glyph("music_note", 48) }
                    }
                    // The words sit with the picture, and the pair floats midway between the header and the controls.
                    Column(Modifier.fillMaxWidth().padding(horizontal = 28.dp, vertical = 18.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(track.title, style = MaterialTheme.typography.headlineMedium, fontWeight = FontWeight.Bold, textAlign = TextAlign.Center, maxLines = 2, overflow = TextOverflow.Ellipsis)
                        track.artist?.let { Text(it, style = MaterialTheme.typography.titleMedium, color = faint, textAlign = TextAlign.Center, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                    }
                    Spacer(Modifier.weight(1f))
                    val duration = playback.duration.coerceAtLeast(1)
                    var dragging by remember { mutableStateOf<Float?>(null) }
                    Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 10.dp)) {
                        SlimScrubber(dragging ?: (playback.position.toFloat() / duration).coerceIn(0f, 1f), playback.ready, ink, { dragging = it }, { dragging?.let { playback.seek((it * duration).toLong()) }; dragging = null })
                        Row(Modifier.fillMaxWidth().padding(horizontal = 2.dp), horizontalArrangement = Arrangement.SpaceBetween) {
                            Text(trackTime(dragging?.let { (it * duration).toLong() } ?: playback.position), style = MaterialTheme.typography.labelSmall, color = faint)
                            Text(trackTime(playback.duration), style = MaterialTheme.typography.labelSmall, color = faint)
                        }
                    }
                    // Transport centred; the lyrics key at the end of the row, in the play key's colour while the lyrics are in view.
                    Box(Modifier.fillMaxWidth().onGloballyPositioned { controlsTop = it.positionInRoot().y + absolute }.padding(start = 24.dp, end = 24.dp, bottom = 8.dp)) {
                        Row(Modifier.align(Alignment.Center), horizontalArrangement = Arrangement.spacedBy(28.dp), verticalAlignment = Alignment.CenterVertically) {
                            SigilIconButton({ playback.seek((playback.position - 10_000).coerceAtLeast(0)) }, enabled = playback.ready) { Glyph("replay_10", 30, "Back ten seconds") }
                            Box(Modifier.size(72.dp).background(scheme.primary, SquircleShape).clip(SquircleShape).clickable(enabled = playback.ready) { playback.toggle() }, contentAlignment = Alignment.Center) {
                                CompositionLocalProvider(LocalContentColor provides scheme.onPrimary) {
                                    if (playback.failed) Glyph("error", 32, "Playback failed")
                                    else if (!playback.ready) CircularProgressIndicator(Modifier.size(28.dp), color = scheme.onPrimary, strokeWidth = 3.dp)
                                    else Glyph(if (playback.playing) "pause" else "play_arrow", 40, if (playback.playing) "Pause" else "Play", filled = true)
                                }
                            }
                            SigilIconButton({ playback.seek((playback.position + 10_000).coerceAtMost(playback.duration)) }, enabled = playback.ready) { Glyph("forward_10", 30, "Forward ten seconds") }
                        }
                        if (track.lyrics.isNotEmpty()) Box(Modifier.align(Alignment.CenterEnd).size(48.dp).then(if (inLyrics) Modifier.background(scheme.primary, SquircleShape) else Modifier).clip(SquircleShape)
                            .clickable { scope.launch { list.animateScrollBy((if (inLyrics) 0f else lyricsTop) - absolute, glide) } }, contentAlignment = Alignment.Center) {
                            CompositionLocalProvider(LocalContentColor provides if (inLyrics) scheme.onPrimary else scheme.onSurface) { Glyph("lyrics", 22, if (inLyrics) "Back to the player" else "Show lyrics") }
                        }
                    }
                } }
                if (track.lyrics.isNotEmpty()) {
                    item { Column(Modifier.fillMaxWidth().onSizeChanged { lyricsHeight = it.height }) {
                        Text("Lyrics", Modifier.fillMaxWidth().padding(horizontal = 28.dp, vertical = 8.dp), style = MaterialTheme.typography.labelLarge, color = faint)
                        track.lyrics.forEachIndexed { index, line ->
                            val live = synced && index == current
                            Text(line.text.ifEmpty { "…" }, Modifier.fillMaxWidth().onGloballyPositioned { lineTops[index] = viewportPx + it.positionInParent().y }
                                .then(if (synced) Modifier.clickable { line.atMs?.let(playback::seek) } else Modifier).padding(horizontal = 28.dp, vertical = 6.dp),
                                style = if (synced) MaterialTheme.typography.titleLarge else MaterialTheme.typography.bodyLarge, fontWeight = if (live) FontWeight.Bold else FontWeight.Medium,
                                color = if (!synced || live) ink else ink.copy(alpha = .42f), fontSize = if (synced) 22.sp else 17.sp, lineHeight = if (synced) 30.sp else 26.sp)
                        }
                        Spacer(Modifier.height(navigation + 96.dp))
                    } }
                    item { Spacer(Modifier.height(tail)) }
                }
            } }
            // The timeline's header, floating over the room like it floats over the conversation.
            ViewerHeader(backdrop, Modifier.align(Alignment.TopCenter).then(follow), { reach = it }) {
                Symbol("chevron_left", "Back", close)
                Column(Modifier.weight(1f).padding(start = 10.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text("Now playing", style = MaterialTheme.typography.titleLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text("${track.kind} · ${attachmentSize(track.bytes)}", style = MaterialTheme.typography.labelMedium, color = scheme.onSurfaceVariant, maxLines = 1)
                }
                download?.let { Symbol("download", if (downloading) "Saving track" else "Save track", it) }
            }
            // The way back, sliding up from the foot once the lyrics are in view and away again when they are left.
            val motion = LocalMotion.current
            AnimatedVisibility(inLyrics, Modifier.align(Alignment.BottomCenter).padding(bottom = navigation + 16.dp),
                enter = slideInVertically(motion.enter(MotionMillis)) { it * 2 } + fadeIn(motion.enter(MotionMillis)),
                exit = slideOutVertically(motion.exit(MotionMillis)) { it * 2 } + fadeOut(motion.exit(MotionExit)), label = "Return to top") {
                FloatingChrome(backdrop, follow.size(52.dp).clip(SquircleShape).clickable { scope.launch { list.animateScrollBy(-absolute, glide) } }, SquircleShape) {
                    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Glyph("keyboard_arrow_up", 26, "Return to top") }
                }
            }
        }
    }
}
