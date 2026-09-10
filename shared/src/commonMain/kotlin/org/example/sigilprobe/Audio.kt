package org.sigil

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp

fun audioTime(milliseconds: Long): String {
    val seconds = milliseconds.coerceAtLeast(0) / 1000
    return "${seconds / 60}:${(seconds % 60).toString().padStart(2, '0')}"
}

@Composable
fun AudioPlayback(position: Long, duration: Long, playing: Boolean, levels: List<Float> = emptyList(), enabled: Boolean = true,
    preview: Boolean = false, modifier: Modifier = Modifier, play: () -> Unit, seek: (Long) -> Unit, expand: (() -> Unit)? = null) {
    var seeking by remember { mutableStateOf<Long?>(null) }
    val current = seeking ?: position
    Row(modifier, verticalAlignment = Alignment.CenterVertically) {
        SigilIconButton(play, enabled = enabled) { Glyph(if (playing) "pause" else "play_arrow", 24,
            if (preview) if (playing) "Pause voice preview" else "Play voice preview" else if (playing) "Pause audio message" else "Play audio message") }
        Column(Modifier.weight(1f)) {
            Box(Modifier.fillMaxWidth().height(48.dp), contentAlignment = Alignment.Center) {
                if (levels.isNotEmpty()) AudioWaveform(levels, Modifier.fillMaxWidth().padding(horizontal = 10.dp).height(32.dp), current.toFloat() / duration.coerceAtLeast(1))
                Slider(current.toFloat().coerceIn(0f, duration.coerceAtLeast(1).toFloat()), { seeking = it.toLong() },
                    Modifier.fillMaxWidth().testTag(if (preview) "voice-preview-seek" else "audio-seek"), enabled = enabled && duration > 0,
                    valueRange = 0f..duration.coerceAtLeast(1).toFloat(), onValueChangeFinished = { seeking?.let(seek); seeking = null },
                    colors = if (levels.isEmpty()) SliderDefaults.colors() else SliderDefaults.colors(activeTrackColor = Color.Transparent, inactiveTrackColor = Color.Transparent,
                        disabledActiveTrackColor = Color.Transparent, disabledInactiveTrackColor = Color.Transparent))
            }
            Text("${audioTime(current)} / ${audioTime(duration)}", style = MaterialTheme.typography.labelSmall)
        }
        if (expand != null) Symbol("open_in_full", "Expand audio", expand)
    }
}

@Composable
fun AudioWaveform(levels: List<Float>, modifier: Modifier, progress: Float = 1f) {
    val ink = LocalContentColor.current
    val muted = ink.copy(alpha = .32f)
    Canvas(modifier) {
        levels.forEachIndexed { i, level ->
            val x = size.width * (i + .5f) / levels.size
            val height = size.height * level.coerceIn(.035f, 1f) / 2f
            drawLine(if (i.toFloat() / levels.size <= progress) ink else muted, Offset(x, center.y - height), Offset(x, center.y + height), 2.dp.toPx(), StrokeCap.Round)
        }
    }
}
