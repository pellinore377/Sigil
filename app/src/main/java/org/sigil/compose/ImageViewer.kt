package org.sigil.compose

import android.graphics.Bitmap
import android.graphics.BitmapRegionDecoder
import android.graphics.Rect
import android.graphics.RectF
import android.graphics.Matrix
import android.media.ExifInterface
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.*
import kotlinx.coroutines.*
import org.sigil.ChatMessage
import org.sigil.Glyph
import org.sigil.SigilIconButton
import kotlin.math.roundToInt

internal class RegionSource(private val bytes: ByteArray) : AutoCloseable {
    @Suppress("DEPRECATION")
    private val decoder = BitmapRegionDecoder.newInstance(bytes, 0, bytes.size, false) ?: error("Image preview unavailable")
    private val transform = Matrix().apply {
        val orientation = runCatching { ExifInterface(bytes.inputStream()).getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL) }.getOrDefault(ExifInterface.ORIENTATION_NORMAL)
        when (orientation) {
            ExifInterface.ORIENTATION_FLIP_HORIZONTAL -> setScale(-1f, 1f)
            ExifInterface.ORIENTATION_ROTATE_180 -> setRotate(180f)
            ExifInterface.ORIENTATION_FLIP_VERTICAL -> setScale(1f, -1f)
            ExifInterface.ORIENTATION_TRANSPOSE -> setValues(floatArrayOf(0f, 1f, 0f, 1f, 0f, 0f, 0f, 0f, 1f))
            ExifInterface.ORIENTATION_ROTATE_90 -> setRotate(90f)
            ExifInterface.ORIENTATION_TRANSVERSE -> setValues(floatArrayOf(0f, -1f, 0f, -1f, 0f, 0f, 0f, 0f, 1f))
            ExifInterface.ORIENTATION_ROTATE_270 -> setRotate(270f)
        }
        val bounds = RectF(0f, 0f, decoder.width.toFloat(), decoder.height.toFloat()); mapRect(bounds)
        postTranslate(-bounds.left, -bounds.top)
    }
    private val bounds = RectF(0f, 0f, decoder.width.toFloat(), decoder.height.toFloat()).also { transform.mapRect(it) }
    private val inverse = Matrix().also { check(transform.invert(it)) }
    val width = bounds.width().roundToInt()
    val height = bounds.height().roundToInt()
    private var closed = false
    @Synchronized fun region(rect: Rect, sample: Int): ImageTile {
        check(!closed)
        val raw = RectF(rect); inverse.mapRect(raw)
        val crop = Rect(raw.left.roundToInt().coerceIn(0, decoder.width - 1), raw.top.roundToInt().coerceIn(0, decoder.height - 1), raw.right.roundToInt().coerceIn(1, decoder.width), raw.bottom.roundToInt().coerceIn(1, decoder.height))
        val bitmap = decoder.decodeRegion(crop, android.graphics.BitmapFactory.Options().apply { inSampleSize = sample }) ?: error("Image region unavailable")
        val oriented = Bitmap.createBitmap(bitmap, 0, 0, bitmap.width, bitmap.height, transform, true)
        if (oriented !== bitmap) bitmap.recycle()
        val placed = RectF(crop).also { transform.mapRect(it) }
        return ImageTile(Rect(placed.left.roundToInt(), placed.top.roundToInt(), placed.right.roundToInt(), placed.bottom.roundToInt()), oriented)
    }
    @Synchronized override fun close() { if (!closed) { closed = true; decoder.recycle(); bytes.fill(0) } }
}
internal data class ImageTile(val rect: Rect, val bitmap: Bitmap)

@Composable
internal fun ImageViewer(message: ChatMessage, preview: Bitmap, modifier: Modifier) {
    val context = LocalContext.current
    var source by remember(message.id) { mutableStateOf<RegionSource?>(null) }
    var tile by remember(message.id) { mutableStateOf<ImageTile?>(null) }
    var issue by remember(message.id) { mutableStateOf<String?>(null) }
    var size by remember { mutableStateOf(IntSize.Zero) }
    var zoom by remember { mutableFloatStateOf(1f) }
    var pan by remember { mutableStateOf(Offset.Zero) }
    val width = source?.width ?: preview.width
    val height = source?.height ?: preview.height
    val fit = minOf(size.width.toFloat() / width, size.height.toFloat() / height, 1f)
    val scale = fit * zoom
    fun bounded(value: Offset, z: Float) = Offset(value.x.coerceIn(-maxOf(0f, (width * fit * z - size.width) / 2), maxOf(0f, (width * fit * z - size.width) / 2)),
        value.y.coerceIn(-maxOf(0f, (height * fit * z - size.height) / 2), maxOf(0f, (height * fit * z - size.height) / 2)))
    LaunchedEffect(message.id) {
        var owned: RegionSource? = null
        var bytes: ByteArray? = null
        try {
            withContext(Dispatchers.IO) {
                bytes = mediaBytes(context, message, 16 * 1024 * 1024)
                owned = RegionSource(bytes!!)
            }
            source = owned
            awaitCancellation()
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { issue = "Full-resolution preview is unavailable for this image." }
        finally { withContext(NonCancellable + Dispatchers.IO) { owned?.close(); bytes?.fill(0) } }
    }
    LaunchedEffect(source, size, zoom, pan) {
        val current = source ?: return@LaunchedEffect
        if (scale <= 0) return@LaunchedEffect
        delay(32)
        val left = (size.width - width * scale) / 2 + pan.x
        val top = (size.height - height * scale) / 2 + pan.y
        val rect = Rect((-left / scale).toInt().coerceIn(0, width - 1), (-top / scale).toInt().coerceIn(0, height - 1),
            ((size.width - left) / scale).roundToInt().coerceIn(1, width), ((size.height - top) / scale).roundToInt().coerceIn(1, height))
        var sample = 1
        while (sample < (1 shl 24) && (sample * 2 * scale <= 1 || rect.width().toLong() / sample * (rect.height() / sample) > 8_000_000)) sample *= 2
        try { tile = withContext(Dispatchers.IO) { current.region(rect, sample) } }
        catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { issue = "This part of the image could not be loaded." }
    }
    Column(modifier) {
        Canvas(Modifier.weight(1f).fillMaxWidth().clipToBounds().onSizeChanged { size = it }
            .semantics { contentDescription = message.attachment!!.name }
            .pointerInput(width, height, size) { detectTransformGestures { centroid, movement, change, _ ->
                val next = (zoom * change).coerceIn(1f, 32f)
                val focus = centroid - Offset(size.width / 2f, size.height / 2f)
                pan = bounded(focus - (focus - pan) * (next / zoom) + movement, next); zoom = next
            } }
            .pointerInput(width, height, size) { detectTapGestures(onDoubleTap = { zoom = if (zoom > 1f) 1f else 2f; pan = Offset.Zero }) }) {
            val left = (size.width - width * scale) / 2 + pan.x
            val top = (size.height - height * scale) / 2 + pan.y
            drawImage(preview.asImageBitmap(), dstOffset = IntOffset(left.roundToInt(), top.roundToInt()), dstSize = IntSize((width * scale).roundToInt().coerceAtLeast(1), (height * scale).roundToInt().coerceAtLeast(1)))
            tile?.let { current -> drawImage(current.bitmap.asImageBitmap(), dstOffset = IntOffset((left + current.rect.left * scale).roundToInt(), (top + current.rect.top * scale).roundToInt()),
                dstSize = IntSize((current.rect.width() * scale).roundToInt().coerceAtLeast(1), (current.rect.height() * scale).roundToInt().coerceAtLeast(1))) }
        }
        Row(Modifier.align(Alignment.CenterHorizontally), verticalAlignment = Alignment.CenterVertically) {
            SigilIconButton({ zoom = (zoom / 2).coerceAtLeast(1f); pan = bounded(pan, zoom) }, enabled = zoom > 1) { Glyph("zoom_out", 24, "Zoom out") }
            Text("${(scale * 100).roundToInt()}%", style = MaterialTheme.typography.labelSmall)
            SigilIconButton({ zoom = (zoom * 2).coerceAtMost(32f) }, enabled = zoom < 32) { Glyph("zoom_in", 24, "Zoom in") }
            SigilIconButton({ zoom = 1f; pan = Offset.Zero }) { Glyph("fit_screen", 24, "Fit image") }
        }
        issue?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
    }
}
