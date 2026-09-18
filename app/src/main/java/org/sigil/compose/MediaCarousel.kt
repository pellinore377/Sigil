package org.sigil.compose

import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.graphics.drawable.AnimatedImageDrawable
import android.graphics.drawable.Drawable
import android.os.Build
import android.widget.ImageView
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import org.sigil.*
import java.nio.ByteBuffer

// The viewer pages through every picture, clip and gif in the conversation, starting from the one that was opened.
@Composable
internal fun MediaCarousel(start: ChatMessage, close: () -> Unit) {
    val all = LocalTimelineMedia.current.ifEmpty { listOf(start) }
    val startIndex = all.indexOfFirst { it.id == start.id && it.author == start.author }.coerceAtLeast(0)
    val pager = rememberPagerState(startIndex) { all.size }
    val zooms = remember { mutableStateMapOf<Int, Float>() }
    val current = all[pager.currentPage.coerceIn(all.indices)]
    // Only picture pages feed the glass: they draw pixels, never views.
    val backdrop = rememberChromeBackdrop()
    MediaDialog(current, close, backdrop) {
        // A zoomed picture holds the carousel still; a swipe then pans the picture instead.
        HorizontalPager(pager, Modifier.fillMaxSize(), pageSpacing = 16.dp, userScrollEnabled = (zooms[pager.currentPage] ?: 1f) <= 1f) { index ->
            MediaPage(all[index], active = index == pager.currentPage, backdrop) { zooms[index] = it }
        }
    }
}

@Composable
private fun MediaPage(message: ChatMessage, active: Boolean, backdrop: ChromeBackdrop, onZoom: (Float) -> Unit) {
    val file = message.attachment ?: return
    when {
        file.mediaType.startsWith("video/") -> VideoPlayer(message, active, Modifier.fillMaxSize())
        file.mediaType == "image/gif" && Build.VERSION.SDK_INT >= 28 -> GifPage(message, active)
        else -> ImagePage(message, active, backdrop, onZoom)
    }
}

@Composable
private fun ImagePage(message: ChatMessage, active: Boolean, backdrop: ChromeBackdrop, onZoom: (Float) -> Unit) {
    val context = LocalContext.current
    val cache = LocalImageCache.current
    var bitmap by remember(message.id) { mutableStateOf<Bitmap?>(null) }
    var failed by remember(message.id) { mutableStateOf(false) }
    LaunchedEffect(message.id) {
        try { while (!withContext(Dispatchers.IO) { prepare(context, message) }) delay(1000); bitmap = historyBitmap(context, message, cache) }
        catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { failed = true }
    }
    val picture = bitmap
    if (picture == null) Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { if (failed) Text("This picture could not be loaded.") else CircularProgressIndicator() }
    else Box(Modifier.fillMaxSize().then(if (active) Modifier.captureBackdrop(backdrop) else Modifier)) { MediaViewerFrame(picture.width, picture.height) { frame -> ImageViewer(message, picture, frame, onZoom) } }
}

// A gif decoded once for the page; it animates only while its page is the one in view.
@androidx.annotation.RequiresApi(28)
@Composable
private fun GifPage(message: ChatMessage, active: Boolean) {
    val context = LocalContext.current
    val reduced = LocalMotion.current.reduced
    var drawable by remember(message.id) { mutableStateOf<Drawable?>(null) }
    var failed by remember(message.id) { mutableStateOf(false) }
    LaunchedEffect(message.id) {
        var bytes: ByteArray? = null
        try {
            while (!withContext(Dispatchers.IO) { prepare(context, message) }) delay(1000)
            drawable = withContext(Dispatchers.IO) {
                bytes = mediaBytes(context, message, 16 * 1024 * 1024)
                ImageDecoder.decodeDrawable(ImageDecoder.createSource(ByteBuffer.wrap(bytes!!))) { decoder, info, _ ->
                    val scale = maxOf(1f, maxOf(info.size.width, info.size.height) / 1080f)
                    decoder.setTargetSize((info.size.width / scale).toInt().coerceAtLeast(1), (info.size.height / scale).toInt().coerceAtLeast(1))
                }
            }
            awaitCancellation()
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { failed = true }
        finally { (drawable as? AnimatedImageDrawable)?.stop(); bytes?.fill(0) }
    }
    val image = drawable
    if (image == null) Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { if (failed) Text("This gif could not be loaded.") else CircularProgressIndicator() }
    else MediaViewerFrame(image.intrinsicWidth, image.intrinsicHeight) { frame ->
        AndroidView({ ImageView(it).apply { scaleType = ImageView.ScaleType.FIT_CENTER; contentDescription = message.attachment!!.name } }, frame.semantics { contentDescription = message.attachment!!.name },
            onReset = null, onRelease = { view -> if (image.callback === view) { (image as? AnimatedImageDrawable)?.stop(); image.callback = null } },
            update = { view ->
                if (view.drawable !== image) view.setImageDrawable(image)
                (image as? AnimatedImageDrawable)?.let { animation -> if (active && !reduced) { animation.repeatCount = AnimatedImageDrawable.REPEAT_INFINITE; if (!animation.isRunning) animation.start() } else animation.stop() }
            })
    }
}
