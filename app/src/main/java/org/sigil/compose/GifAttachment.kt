package org.sigil.compose

import android.graphics.ImageDecoder
import android.graphics.drawable.AnimatedImageDrawable
import android.graphics.drawable.Drawable
import android.widget.ImageView
import androidx.annotation.RequiresApi
import androidx.compose.foundation.clickable
import androidx.compose.foundation.Image
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.Alignment
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import kotlinx.coroutines.*
import org.sigil.*
import java.nio.ByteBuffer

@RequiresApi(28)
@Composable
internal fun GifAttachment(message: ChatMessage) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val appearance = LocalAppearance.current
    val reduced = LocalMotion.current.reduced
    var visible by remember { mutableStateOf(lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)) }
    var imageWidth by rememberSaveable(message.peer,message.author,message.id) { mutableIntStateOf(0) }
    var imageHeight by rememberSaveable(message.peer,message.author,message.id) { mutableIntStateOf(0) }
    var drawable by remember(message.id) { mutableStateOf<Drawable?>(null) }
    var failed by remember(message.id) { mutableStateOf(false) }
    var retry by remember { mutableIntStateOf(0) }
    var expanded by remember { mutableStateOf(false) }
    var poster by remember(message.id) { mutableStateOf<android.graphics.Bitmap?>(null) }
    val playing = visible && !reduced && (expanded || appearance.autoplayGifs)
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
                    val scale = maxOf(1f, maxOf(info.size.width, info.size.height) / 1080f)
                    decoder.setTargetSize((info.size.width / scale).toInt().coerceAtLeast(1), (info.size.height / scale).toInt().coerceAtLeast(1))
                }
            }
            imageWidth=drawable!!.intrinsicWidth;imageHeight=drawable!!.intrinsicHeight
            awaitCancellation()
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { failed = true }
        finally { (drawable as? AnimatedImageDrawable)?.stop(); bytes?.fill(0) }
    }
    @Composable fun picture(modifier: Modifier) {
        val image = drawable
        if (image == null) Box(modifier) { if (failed) SigilTextButton({ retry++ }) { Text("Retry GIF") } else LinearProgressIndicator(Modifier.fillMaxWidth()) }
        else AndroidView({ ImageView(it).apply { scaleType = ImageView.ScaleType.FIT_CENTER; contentDescription = message.attachment!!.name } }, modifier.semantics { contentDescription = message.attachment!!.name },
            onReset = null, onRelease = { view -> if (image.callback === view) { (image as? AnimatedImageDrawable)?.stop(); image.callback = null } },
            update = { view ->
                if (view.drawable !== image) view.setImageDrawable(image)
                (image as? AnimatedImageDrawable)?.let { animation -> if (playing) { animation.repeatCount = AnimatedImageDrawable.REPEAT_INFINITE; if (!animation.isRunning) animation.start() } else animation.stop() }
            })
    }
    val captioned = message.attachment!!.caption.isNotBlank()
    ImageMessageFrame(imageWidth,imageHeight,
        if (captioned) androidx.compose.ui.graphics.RectangleShape else androidx.compose.foundation.shape.RoundedCornerShape(20.dp),
        if (!captioned) null else { { Box(Modifier.padding(horizontal = 14.dp, vertical = 10.dp)) { MessageText(message.attachment!!.caption, NativeCore::analyze) } } }) { frame ->
        Box(frame.clickable {
            drawable?.let { image ->
                val snapshot = android.graphics.Bitmap.createBitmap(image.intrinsicWidth.coerceAtLeast(1), image.intrinsicHeight.coerceAtLeast(1), android.graphics.Bitmap.Config.ARGB_8888)
                val bounds = android.graphics.Rect(image.bounds)
                image.setBounds(0, 0, snapshot.width, snapshot.height)
                image.draw(android.graphics.Canvas(snapshot)); image.bounds = bounds
                poster = snapshot
            }
            expanded = true
        }) {
            if (!expanded) picture(Modifier.fillMaxSize())
            else poster?.let { Image(it.asImageBitmap(), message.attachment!!.name, Modifier.fillMaxSize(), contentScale = ContentScale.Fit) }
            GifChip(Modifier.align(Alignment.TopStart))
        }
    }
    if (expanded) MediaDialog(message, { expanded = false }) {
        MediaViewerFrame(drawable?.intrinsicWidth ?: 0, drawable?.intrinsicHeight ?: 0) { picture(it) }
    }
}
