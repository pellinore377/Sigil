package org.sigil.compose

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.ImageDecoder
import android.media.MediaDataSource
import android.media.MediaPlayer
import android.os.Build
import android.view.Surface
import android.view.TextureView
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import kotlinx.coroutines.*
import org.json.JSONObject
import org.sigil.ChatMessage
import org.sigil.Glyph
import org.sigil.storage.NativeStorage
import org.sigil.storage.StorageKeyProvider
import java.nio.ByteBuffer

internal class EncryptedMedia(private val context: android.content.Context, private val peer: String, private val author: String, private val message: String, private val length: Long) : MediaDataSource() {
    constructor(context: android.content.Context, message: ChatMessage) : this(context, message.peer, message.author, message.id, message.attachment!!.bytes)
    private var part = -1
    private var bytes = ByteArray(0)
    private var closed = false
    override fun getSize() = length
    @Synchronized override fun readAt(position: Long, buffer: ByteArray, offset: Int, size: Int): Int {
        check(!closed)
        require(position >= 0 && offset >= 0 && size >= 0 && offset <= buffer.size - size)
        if (size == 0) return 0
        if (position >= getSize()) return -1
        val index = (position / (1024 * 1024)).toInt()
        if (part != index) {
            bytes.fill(0)
            bytes = StorageKeyProvider(context).withKey { directory, key -> NativeStorage.readFileChunk(directory.path, key, peer, author, message, index) } ?: throw java.io.IOException("Attachment unavailable")
            part = index
        }
        val within = (position % (1024 * 1024)).toInt()
        val count = minOf(size.toLong(), (bytes.size - within).toLong(), getSize() - position).toInt()
        if (count <= 0) throw java.io.IOException("Incomplete attachment")
        bytes.copyInto(buffer, offset, within, within + count)
        return count
    }
    @Synchronized override fun close() { closed = true; bytes.fill(0); bytes = ByteArray(0) }
}
internal fun prepare(context: android.content.Context, message: ChatMessage): Boolean {
    val command = JSONObject().put("command", "file_get").put("peer", message.peer).put("author", message.author).put("message", message.id)
    val result = StorageKeyProvider(context).withKey { directory, key -> JSONObject(NativeStorage.execute(directory.path, key, command.toString())) }
    check(result.getBoolean("ok"))
    return result.getJSONObject("value").getString("phase") in listOf("Complete", "Published", "Restored")
}
private fun bitmap(context: android.content.Context, message: ChatMessage): Bitmap {
    val length = message.attachment!!.bytes
    check(length in 1..16 * 1024 * 1024)
    val bytes = ByteArray(length.toInt())
    try {
        EncryptedMedia(context, message).use { media ->
            var at = 0
            while (at < bytes.size) { val count = media.readAt(at.toLong(), bytes, at, bytes.size - at); check(count > 0); at += count }
        }
        return if (Build.VERSION.SDK_INT >= 28) ImageDecoder.decodeBitmap(ImageDecoder.createSource(ByteBuffer.wrap(bytes))) { decoder, info, _ ->
            val scale = maxOf(1f, maxOf(info.size.width, info.size.height) / 1600f)
            decoder.setTargetSize((info.size.width / scale).toInt().coerceAtLeast(1), (info.size.height / scale).toInt().coerceAtLeast(1))
            decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
        } else {
            val options = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)
            check(options.outWidth > 0 && options.outHeight > 0)
            options.inSampleSize = 1
            while (maxOf(options.outWidth, options.outHeight) / options.inSampleSize > 1600) options.inSampleSize *= 2
            options.inJustDecodeBounds = false
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options) ?: error("Unsupported image")
        }
    } finally { bytes.fill(0) }
}
@Composable
internal fun AndroidAttachment(message: ChatMessage) {
    val file = message.attachment ?: return
    if (file.mediaType.startsWith("audio/")) { AudioMessage(message); return }
    val context = LocalContext.current
    val image = file.mediaType.startsWith("image/") && file.bytes <= 16 * 1024 * 1024
    val playable = file.mediaType.startsWith("video/")
    var requested by remember(message.id) { mutableStateOf(image) }
    var ready by remember(message.id) { mutableStateOf(false) }
    var failed by remember(message.id) { mutableStateOf(false) }
    var bitmap by remember(message.id) { mutableStateOf<Bitmap?>(null) }
    var opened by remember(message.id) { mutableStateOf(false) }
    LaunchedEffect(message.id, requested) {
        if (!requested) return@LaunchedEffect
        failed = false
        try {
            while (!withContext(Dispatchers.IO) { prepare(context, message) }) delay(1000)
            if (image) bitmap = withContext(Dispatchers.IO) { bitmap(context, message) }
            ready = true
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { failed = true; requested = false }
    }
    Column(Modifier.widthIn(min = 160.dp, max = 300.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        val picture = bitmap
        if (picture != null) Image(picture.asImageBitmap(), file.name, Modifier.fillMaxWidth().heightIn(max = 300.dp).clickable { opened = true }, contentScale = ContentScale.Fit)
        else {
            Text(file.name, maxLines = 2)
            Text(if (file.bytes >= 1024 * 1024) "${file.bytes / (1024 * 1024)} MB" else "${file.bytes / 1024} KB", style = MaterialTheme.typography.labelSmall)
            if (requested && !ready) LinearProgressIndicator(Modifier.fillMaxWidth())
            TextButton({ if (!ready) requested = true else opened = true }, enabled = !requested || ready) {
                Glyph(if (ready && playable) "play_arrow" else if (ready) "open_in_new" else "download", 22)
                Text(if (failed) "Retry" else if (ready && playable) "Play" else if (ready) "Open" else "Download")
            }
        }
    }
    if (opened) {
        if (image && bitmap != null) Dialog({ opened = false }) { Image(bitmap!!.asImageBitmap(), file.name, Modifier.fillMaxWidth(), contentScale = ContentScale.Fit) }
        else if (playable) VideoDialog(message) { opened = false }
        else LaunchedEffect(message.id) { NativeFileProvider.open(context, message); opened = false }
    }
}
@Composable
internal fun VideoDialog(message: ChatMessage, close: () -> Unit) {
    val context = LocalContext.current
    val media = remember(message.id) { EncryptedMedia(context, message) }
    val player = remember(message.id) { MediaPlayer() }
    var ready by remember { mutableStateOf(false) }
    var playing by remember { mutableStateOf(false) }
    var failed by remember { mutableStateOf(false) }
    var duration by remember { mutableLongStateOf(0) }
    var position by remember { mutableLongStateOf(0) }
    var seeking by remember { mutableStateOf(false) }
    var ratio by remember { mutableFloatStateOf(16f / 9f) }
    var prepared by remember { mutableStateOf(false) }
    var released by remember { mutableStateOf(false) }
    val lifecycle = androidx.lifecycle.compose.LocalLifecycleOwner.current
    DisposableEffect(lifecycle, player) {
        val observer = androidx.lifecycle.LifecycleEventObserver { _, event ->
            if (event == androidx.lifecycle.Lifecycle.Event.ON_STOP && ready && !failed) { player.pause(); playing = false }
        }
        lifecycle.lifecycle.addObserver(observer)
        onDispose { lifecycle.lifecycle.removeObserver(observer) }
    }
    LaunchedEffect(ready, playing, failed) {
        while (ready && !failed) { if (!seeking) position = player.currentPosition.toLong(); delay(250) }
    }
    DisposableEffect(player) {
        player.setDataSource(media)
        player.setOnPreparedListener { duration = it.duration.toLong().coerceAtLeast(0); ready = true; if (lifecycle.lifecycle.currentState.isAtLeast(androidx.lifecycle.Lifecycle.State.STARTED)) { it.start(); playing = true } }
        player.setOnVideoSizeChangedListener { _, width, height -> if (width > 0 && height > 0) ratio = width.toFloat() / height }
        player.setOnCompletionListener { playing = false }
        player.setOnErrorListener { _, _, _ -> failed = true; playing = false; true }
        onDispose { released = true; player.release(); media.close() }
    }
    Dialog(close) {
        Surface(shape = androidx.compose.foundation.shape.RoundedCornerShape(24.dp)) {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(message.attachment!!.name, style = MaterialTheme.typography.titleMedium)
                AndroidView(factory = { context -> TextureView(context).apply {
                    surfaceTextureListener = object : TextureView.SurfaceTextureListener {
                        private var surface: Surface? = null
                        override fun onSurfaceTextureAvailable(texture: android.graphics.SurfaceTexture, width: Int, height: Int) { surface = Surface(texture); player.setSurface(surface); if (!prepared) { prepared = true; player.prepareAsync() } }
                        override fun onSurfaceTextureSizeChanged(texture: android.graphics.SurfaceTexture, width: Int, height: Int) {}
                        override fun onSurfaceTextureUpdated(texture: android.graphics.SurfaceTexture) {}
                        override fun onSurfaceTextureDestroyed(texture: android.graphics.SurfaceTexture): Boolean { if (!released) player.setSurface(null); surface?.release(); surface = null; return true }
                    }
                } }, modifier = Modifier.fillMaxWidth().aspectRatio(ratio.coerceIn(.25f, 4f)))
                if (failed) Text("This device could not play the attachment.")
                if (ready && !failed && duration > 0) {
                    Slider(position.toFloat().coerceIn(0f, duration.toFloat()), { seeking = true; position = it.toLong() }, Modifier.testTag("media-seek"), valueRange = 0f..duration.toFloat(), onValueChangeFinished = { player.seekTo(position, MediaPlayer.SEEK_CLOSEST); seeking = false })
                    Text("${mediaTime(position)} / ${mediaTime(duration)}", style = MaterialTheme.typography.labelSmall)
                }
                Row { TextButton({ if (playing) player.pause() else player.start(); playing = !playing }, enabled = ready && !failed) { Glyph(if (playing) "pause" else "play_arrow", 24); Text(if (playing) "Pause" else "Play") }; TextButton(close) { Text("Close") } }
            }
        }
    }
}
private fun mediaTime(milliseconds: Long): String { val seconds = milliseconds / 1000; return "${seconds / 60}:${(seconds % 60).toString().padStart(2, '0')}" }
