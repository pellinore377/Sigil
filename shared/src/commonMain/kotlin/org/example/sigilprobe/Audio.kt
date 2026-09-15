package org.sigil

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.focusable
import androidx.compose.ui.input.key.*
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
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
                AudioWaveform(levels, Modifier.fillMaxWidth().height(40.dp)
                    .testTag(if (preview) "voice-preview-seek" else "audio-seek")
                    .semantics {
                        contentDescription = "Playback position"
                        progressBarRangeInfo = ProgressBarRangeInfo(current.toFloat(), 0f..duration.coerceAtLeast(1).toFloat())
                        if(enabled && duration>0)setProgress {value->seek(value.toLong().coerceIn(0,duration));true}
                    }
                    .onKeyEvent {event->
                        if(!enabled || duration<=0 || event.type!=KeyEventType.KeyDown)false
                        else when(event.key){
                            Key.DirectionLeft->{seek((position-5000).coerceAtLeast(0));true}
                            Key.DirectionRight->{seek((position+5000).coerceAtMost(duration));true}
                            Key.MoveHome->{seek(0);true}
                            Key.MoveEnd->{seek(duration);true}
                            else->false
                        }
                    }.focusable(enabled && duration>0)
                    .pointerInput(enabled,duration) {
                        if(enabled && duration>0)detectTapGestures {point->seek((point.x/size.width*duration).toLong().coerceIn(0,duration))}
                    }
                    .pointerInput(enabled,duration) {
                        if(enabled && duration>0)detectDragGestures(
                            onDragStart={point->seeking=(point.x/size.width*duration).toLong().coerceIn(0,duration)},
                            onDragEnd={seeking?.let(seek);seeking=null},onDragCancel={seeking=null}
                        ) {change,_->change.consume();seeking=(change.position.x/size.width*duration).toLong().coerceIn(0,duration)}
                    },current.toFloat()/duration.coerceAtLeast(1))

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
        val count=(size.width/5.dp.toPx()).toInt().coerceAtLeast(1)
        repeat(count) { i ->
            val from=i*levels.size/count
            val to=((i+1)*levels.size/count).coerceAtLeast(from+1).coerceAtMost(levels.size)
            val level=if(from<levels.size)levels.subList(from,to).maxOrNull() ?: .06f else .06f
            val x = size.width * (i + .5f) / count
            val height = size.height * level.coerceIn(.035f, 1f) / 2f
            drawLine(if (i.toFloat() / count <= progress) ink else muted, Offset(x, center.y - height), Offset(x, center.y + height), 2.dp.toPx(), StrokeCap.Round)
        }
    }
}
