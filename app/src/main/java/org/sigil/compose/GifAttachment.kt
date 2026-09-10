package org.sigil.compose

import android.graphics.ImageDecoder
import android.graphics.drawable.AnimatedImageDrawable
import android.graphics.drawable.Drawable
import android.widget.ImageView
import androidx.annotation.RequiresApi
import androidx.compose.foundation.clickable
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import kotlinx.coroutines.*
import org.sigil.*
import java.nio.ByteBuffer
import androidx.compose.ui.unit.dp

@RequiresApi(28)
@Composable
internal fun GifAttachment(message: ChatMessage) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val appearance = LocalAppearance.current
    val reduced = LocalMotion.current.reduced
    var visible by remember { mutableStateOf(lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)) }
    var manual by remember(message.id) { mutableStateOf<Boolean?>(null) }
    var drawable by remember(message.id) { mutableStateOf<Drawable?>(null) }
    var failed by remember(message.id) { mutableStateOf(false) }
    var retry by remember { mutableIntStateOf(0) }
    var expanded by remember { mutableStateOf(false) }
    val playing = visible && !reduced && (manual ?: appearance.autoplayGifs)
    DisposableEffect(lifecycle) {
        val observer = LifecycleEventObserver { _, _ -> visible = lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED) }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer) }
    }
    LaunchedEffect(message.id, retry) {
        var bytes: ByteArray? = null
        try {
            failed = false
            while (!withContext(Dispatchers.IO) { prepare(context, message) }) delay(1000)
            drawable = withContext(Dispatchers.IO) {
                bytes = mediaBytes(context, message, 16 * 1024 * 1024)
                ImageDecoder.decodeDrawable(ImageDecoder.createSource(ByteBuffer.wrap(bytes!!))) { decoder, info, _ ->
                    val scale = maxOf(1f, maxOf(info.size.width, info.size.height) / 1600f)
                    decoder.setTargetSize((info.size.width / scale).toInt().coerceAtLeast(1), (info.size.height / scale).toInt().coerceAtLeast(1))
                }
            }
            awaitCancellation()
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { failed = true }
        finally { (drawable as? AnimatedImageDrawable)?.stop(); bytes?.fill(0) }
    }
    @Composable fun picture(modifier: Modifier) {
        val image = drawable
        if (image == null) Box(modifier) { if (failed) SigilTextButton({ retry++ }) { Text("Retry GIF") } else LinearProgressIndicator(Modifier.fillMaxWidth()) }
        else AndroidView({ ImageView(it).apply { scaleType = ImageView.ScaleType.CENTER_INSIDE; contentDescription = message.attachment!!.name } }, modifier.semantics { contentDescription = message.attachment!!.name },
            onReset = null, onRelease = { view -> if (image.callback === view) { (image as? AnimatedImageDrawable)?.stop(); image.callback = null } },
            update = { view ->
                if (view.drawable !== image) view.setImageDrawable(image)
                (image as? AnimatedImageDrawable)?.let { animation -> if (playing) { animation.repeatCount = AnimatedImageDrawable.REPEAT_INFINITE; if (!animation.isRunning) animation.start() } else animation.stop() }
            })
    }
    val ratio = drawable?.let { it.intrinsicWidth.toFloat() / it.intrinsicHeight.coerceAtLeast(1) } ?: 1f
    Column(Modifier.widthIn(min = 160.dp, max = 300.dp)) {
        val preview = Modifier.fillMaxWidth().heightIn(max = 300.dp).aspectRatio(ratio.coerceIn(.2f, 5f))
        if (expanded) Spacer(preview) else picture(preview.clickable { expanded = true })
        Row {
            SigilIconButton({ manual = !playing }, enabled = drawable is AnimatedImageDrawable && !reduced) { Glyph(if (playing) "pause" else "play_arrow", 24, if (playing) "Pause GIF" else "Play GIF") }
            SigilIconButton({ expanded = true }) { Glyph("open_in_full", 24, "Expand GIF") }
        }
    }
    if (expanded) Dialog({ expanded = false }) {
        Surface(shape = RoundedCornerShape(24.dp)) {
            Column(Modifier.heightIn(max = 600.dp).verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(message.attachment!!.name, style = MaterialTheme.typography.titleMedium)
                picture(Modifier.fillMaxWidth().heightIn(max = 480.dp).aspectRatio(ratio.coerceIn(.2f, 5f)))
                if (message.attachment!!.caption.isNotBlank()) MessageText(message.attachment!!.caption, NativeCore::analyze)
                Row {
                    SigilTextButton({ manual = !playing }, enabled = drawable is AnimatedImageDrawable && !reduced) { Text(if (playing) "Pause" else "Play") }
                    SigilTextButton({ expanded = false }) { Text("Close") }
                }
            }
        }
    }
}
