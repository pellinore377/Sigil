@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
package org.sigil.compose

import org.sigil.SigilTextButton
import org.sigil.SigilIconButton

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.ImageDecoder
import android.media.MediaDataSource
import android.media.MediaPlayer
import android.os.Build
import android.view.Surface
import android.view.TextureView
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.ui.Alignment
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import kotlinx.coroutines.*
import org.json.JSONObject
import org.sigil.ChatMessage
import org.sigil.Glyph
import org.sigil.storage.NativeStorage
import org.sigil.storage.StorageKeyProvider
import java.nio.ByteBuffer

internal class EncryptedMedia(private val context: android.content.Context, private val peer: String, private val author: String, private val message: String, private val length: Long, private val draft: Boolean = false) : MediaDataSource() {
    constructor(context: android.content.Context, message: ChatMessage) : this(context, message.peer, message.author, message.id, message.attachment!!.bytes, message.attachment!!.draft)
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
            bytes = StorageKeyProvider(context).withKey { directory, key -> if (draft) NativeStorage.readDraftChunk(directory.path, key, message, index) else NativeStorage.readFileChunk(directory.path, key, peer, author, message, index) } ?: throw java.io.IOException("Attachment unavailable")
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
internal data class PreparedMedia(val ready:Boolean,val identity:String?=null)
internal fun prepare(context: android.content.Context, message: ChatMessage)=prepareMedia(context,message).ready
internal fun prepareMedia(context: android.content.Context, message: ChatMessage):PreparedMedia {
    val draft = message.attachment?.draft == true
    val command = if (draft) JSONObject().put("command", "files") else JSONObject().put("command", "file_get").put("peer", message.peer).put("author", message.author).put("message", message.id)
    val result = StorageKeyProvider(context).withKey { directory, key -> JSONObject(NativeStorage.execute(directory.path, key, command.toString())) }
    check(result.getBoolean("ok"))
    if (draft) {
        val uploads = result.getJSONObject("value").getJSONArray("uploads")
        val upload = (0 until uploads.length()).map { uploads.getJSONObject(it) }.singleOrNull { it.getString("request") == message.id && it.optBoolean("draft") } ?: error("Draft unavailable")
        return PreparedMedia(upload.getString("phase") in listOf("Ready", "Starting", "Uploading", "Checking", "Published"))
    }
    val value=result.getJSONObject("value")
    return PreparedMedia(value.getString("phase") in listOf("Complete", "Published", "Restored"),value.optString("cache_id").takeIf {it.length==64})
}
@Composable
internal fun AndroidAttachmentDraft(file: org.sigil.Transfer, modifier: Modifier) {
    val message = remember(file) { ChatMessage(file.request, "", "", true, "", "", false, emptyList(), emptyList(), null, true,
        peer = file.peer, attachment = org.sigil.AttachmentDetails(file.name, file.mediaType, file.bytes, draft = true)) }
    // In the tray a picture of any shape fills its tile.
    androidx.compose.runtime.CompositionLocalProvider(LocalFillsTile provides true) { Box(modifier) { AndroidAttachment(message) } }
}
/// True inside a fixed tile, where a picture fills and crops rather than keeping its shape.
internal val LocalFillsTile = androidx.compose.runtime.staticCompositionLocalOf { false }
@Composable
internal fun MediaFrame(width: Int, height: Int, shape: androidx.compose.ui.graphics.Shape, caption: (@Composable () -> Unit)?, content: @Composable (Modifier) -> Unit) {
    if (LocalFillsTile.current) Box(Modifier.fillMaxSize().clip(shape)) { content(Modifier.fillMaxSize()) }
    else org.sigil.ImageMessageFrame(width, height, shape, caption, content)
}
internal fun mediaBytes(context: android.content.Context, message: ChatMessage, limit: Int): ByteArray {
    val length = message.attachment!!.bytes
    check(length in 1..limit)
    val bytes = ByteArray(length.toInt())
    try {
        EncryptedMedia(context, message).use { media ->
            var at = 0
            while (at < bytes.size) { val count = media.readAt(at.toLong(), bytes, at, bytes.size - at); check(count > 0); at += count }
        }
        return bytes
    } catch (error: Throwable) { bytes.fill(0); throw error }
}
private fun bitmap(context: android.content.Context, message: ChatMessage): Bitmap {
    val bytes = mediaBytes(context, message, 16 * 1024 * 1024)
    try { return decodeThumbnail(bytes) } finally { bytes.fill(0) }
}
internal fun decodeThumbnail(bytes:ByteArray):Bitmap {
        return if (Build.VERSION.SDK_INT >= 28) ImageDecoder.decodeBitmap(ImageDecoder.createSource(ByteBuffer.wrap(bytes))) { decoder, info, _ ->
            val scale = maxOf(1f, maxOf(info.size.width, info.size.height) / 1080f)
            decoder.setTargetSize((info.size.width / scale).toInt().coerceAtLeast(1), (info.size.height / scale).toInt().coerceAtLeast(1))
            decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
        } else {
            val options = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)
            check(options.outWidth > 0 && options.outHeight > 0)
            options.inSampleSize = 1
            while (maxOf(options.outWidth, options.outHeight) / options.inSampleSize > 1080) options.inSampleSize *= 2
            options.inJustDecodeBounds = false
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options) ?: error("Unsupported image")
        }
}
/// A picture's dimensions read from its header, so its frame is the right size before the picture is decoded.
private fun pictureShape(context: android.content.Context, message: ChatMessage): Pair<Int,Int>? {
    // Most headers sit in the first few kilobytes; only a picture that hides its dimensions behind a large
    // thumbnail or metadata block costs a deeper read, and none costs more than a quarter of a megabyte.
    for (step in longArrayOf(16L * 1024, 64L * 1024, 256L * 1024)) {
        val size = minOf(message.attachment!!.bytes, step).toInt()
        if (size <= 0) return null
        val head = ByteArray(size)
        try {
            EncryptedMedia(context, message).use { media ->
                var at = 0
                while (at < head.size) { val count = media.readAt(at.toLong(), head, at, head.size - at); if (count <= 0) break; at += count }
            }
            val options = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeByteArray(head, 0, head.size, options)
            (options.outWidth to options.outHeight).takeIf { it.first > 0 && it.second > 0 }?.let { return it }
        } finally { head.fill(0) }
        if (message.attachment!!.bytes <= step) return null
    }
    return null
}
/// A video's own dimensions from its metadata; no frame is decoded to learn them.
private fun videoShape(context: android.content.Context, message: ChatMessage): Pair<Int,Int>? =
    EncryptedMedia(context, message).use { source ->
        val retriever = android.media.MediaMetadataRetriever()
        try {
            retriever.setDataSource(source)
            fun key(value: Int) = retriever.extractMetadata(value)?.toIntOrNull()
            val width = key(android.media.MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH) ?: return null
            val height = key(android.media.MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT) ?: return null
            val turned = (key(android.media.MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION) ?: 0) % 180 != 0
            (if (turned) height to width else width to height).takeIf { it.first > 0 && it.second > 0 }
        } finally { retriever.release() }
    }
/// An attachment's shape before its content arrives, so the frame it fades into is already the right size.
internal suspend fun attachmentShape(context: android.content.Context, message: ChatMessage, cache: ImageCache?): Pair<Int,Int>? = withContext(Dispatchers.IO) {
    runCatching {
        val prepared = prepareMedia(context, message)
        if (!prepared.ready) return@withContext null
        val identity = prepared.identity
        identity?.let { cache?.shape(it) }?.let { return@withContext it }
        identity?.let { cache?.retained(it) }?.let { return@withContext it.width to it.height }
        val shape = if (message.attachment!!.mediaType.startsWith("video/")) videoShape(context, message) else pictureShape(context, message)
        shape?.also { (w, h) -> identity?.let { cache?.rememberShape(it, w, h) } }
    }.getOrNull()
}
internal suspend fun authorizedThumbnail(cache:ImageCache?,prepare:suspend()->PreparedMedia,decode:suspend()->Bitmap):Bitmap {
    while(true) {
        val prepared=prepare()
        if(prepared.ready) {
            val identity=prepared.identity
            return if(cache==null || identity==null) decode()
            else cache.load(identity) {
                val image=decode()
                try {
                    check(prepare().let {it.ready && it.identity==identity}) {"Attachment changed while loading"}
                    image
                } catch(error:Throwable) {image.recycle();throw error}
            }
        }
        delay(1000)
    }
}
// Pictures the timeline has already decoded, by message, so a card composed again shows them without a reload.
private val recentPictures = object : LinkedHashMap<String, Bitmap>(16, .75f, true) { override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, Bitmap>?) = size > 32 }
internal suspend fun historyBitmap(context:android.content.Context,message:ChatMessage,cache:ImageCache?) = authorizedThumbnail(
    cache.takeUnless {message.attachment?.draft==true},
    prepare={withContext(Dispatchers.IO){prepareMedia(context,message)}},
    decode={withContext(Dispatchers.IO){bitmap(context,message)}})
@Composable
internal fun AndroidAttachment(message: ChatMessage) {
    val file = message.attachment ?: return
    if (file.mediaType == "image/gif" && file.bytes <= 16 * 1024 * 1024 && Build.VERSION.SDK_INT >= 28) { GifAttachment(message); return }
    if (file.mediaType.startsWith("audio/") && file.name == "Voice message.aac") { AudioMessage(message); return }
    if (!(file.mediaType.startsWith("image/") && file.bytes <= 16 * 1024 * 1024) && !file.mediaType.startsWith("video/")) { AndroidFileCard(message); return }
    val context = LocalContext.current
    val imageCache=LocalImageCache.current
    val image = file.mediaType.startsWith("image/") && file.bytes <= 16 * 1024 * 1024
    val playable = file.mediaType.startsWith("video/")
    var imageWidth by rememberSaveable(message.peer,message.author,message.id) { mutableIntStateOf(0) }
    var imageHeight by rememberSaveable(message.peer,message.author,message.id) { mutableIntStateOf(0) }
    var requested by remember(message.id) { mutableStateOf(image) }
    var ready by remember(message.id) { mutableStateOf(recentPictures.containsKey("${message.peer}/${message.author}/${message.id}")) }
    var failed by remember(message.id) { mutableStateOf(false) }
    // A picture already shown lands at once in a fresh composition, such as the menu's copy, instead of fading in again.
    val pictureKey = "${message.peer}/${message.author}/${message.id}"
    var bitmap by remember(message.id) { mutableStateOf(recentPictures[pictureKey]) }
    LaunchedEffect(bitmap) { bitmap?.let { recentPictures[pictureKey] = it } }
    var opened by remember(message.id) { mutableStateOf(false) }
    var openWhenReady by remember(message.id) { mutableStateOf(false) }
    // The video frame takes the file's own shape before anything is decoded, so its poster lands without a resize.
    LaunchedEffect(message.id, playable, imageCache) {
        if (!playable || imageWidth > 0 || imageHeight > 0) return@LaunchedEffect
        attachmentShape(context, message, imageCache)?.let { (w, h) -> imageWidth = w; imageHeight = h }
    }
    LaunchedEffect(message.id, requested, imageCache) {
        if (!requested) return@LaunchedEffect
        failed = false
        try {
            if(image) {
                // The frame takes its shape from the file header first, so the picture fades into a space already its size.
                if(imageWidth<=0 || imageHeight<=0) attachmentShape(context,message,imageCache)?.let {(w,h)->imageWidth=w;imageHeight=h}
                bitmap=historyBitmap(context,message,imageCache)
                imageWidth=bitmap!!.width;imageHeight=bitmap!!.height
            }
            else while (!withContext(Dispatchers.IO) { prepare(context, message) }) delay(1000)
            // Read once the file is there: the shape must land before the poster, or the frame resizes under it.
            if (playable && (imageWidth <= 0 || imageHeight <= 0)) attachmentShape(context, message, imageCache)?.let { (w, h) -> imageWidth = w; imageHeight = h }
            if (playable) bitmap = withContext(Dispatchers.IO) {
                runCatching { EncryptedMedia(context, message).use { source ->
                    android.media.MediaMetadataRetriever().let { retriever ->
                        try { retriever.setDataSource(source); if (Build.VERSION.SDK_INT >= 27) retriever.getScaledFrameAtTime(0, android.media.MediaMetadataRetriever.OPTION_CLOSEST_SYNC, 600, 600) else null }
                        finally { retriever.release() }
                    }
                } }.getOrNull()
            }
            ready = true
            if (openWhenReady) { opened = true; openWhenReady = false }
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { failed = true; requested = false }
    }
    val captioned = file.caption.isNotBlank() && (image || playable)
    val frameShape = if (captioned) androidx.compose.ui.graphics.RectangleShape else androidx.compose.foundation.shape.RoundedCornerShape(20.dp)
    val captionBlock: (@Composable () -> Unit)? = if (!captioned) null else { { Box(Modifier.fillMaxWidth().background(org.sigil.LocalBubbleGround.current).padding(horizontal = 14.dp, vertical = 10.dp)) { org.sigil.MessageText(file.caption, org.sigil.NativeCore::analyze) } } }
    Column(Modifier.widthIn(max = 300.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        val picture = bitmap
        if (playable) MediaFrame(if (imageWidth > 0) imageWidth else picture?.width ?: 16, if (imageHeight > 0) imageHeight else picture?.height ?: 9, frameShape, captionBlock) { frame ->
            val motion = org.sigil.LocalMotion.current
            val poster by animateFloatAsState(if (picture != null) 1f else 0f, motion.enter(org.sigil.MotionMillis), label = "Poster")
            Box(frame.clip(frameShape).background(MaterialTheme.colorScheme.surfaceContainerHigh).combinedClickable(enabled = !requested || ready, onLongClick = org.sigil.LocalMaterialPress.current) { if (ready) opened = true else { openWhenReady = true; requested = true } }, contentAlignment = Alignment.Center) {
            picture?.let { Image(it.asImageBitmap(), file.name, Modifier.fillMaxSize().graphicsLayer { alpha = poster }, contentScale = if (LocalFillsTile.current) ContentScale.Crop else ContentScale.Fit) }
            if (requested && !ready) CircularProgressIndicator(Modifier.size(32.dp))
            else Surface(shape = androidx.compose.foundation.shape.CircleShape, color = androidx.compose.ui.graphics.Color.Black.copy(alpha = .6f), contentColor = androidx.compose.ui.graphics.Color.White) { Box(Modifier.size(48.dp), contentAlignment = Alignment.Center) { Glyph(if (failed) "refresh" else "play_arrow", 28, if (failed) "Retry video" else "Play video") } }
        } }
        else if (image) MediaFrame(picture?.width ?: imageWidth, picture?.height ?: imageHeight, frameShape, captionBlock) { frame ->
            val motion = org.sigil.LocalMotion.current
            val arrival by animateFloatAsState(if (picture != null) 1f else 0f, motion.enter(org.sigil.MotionMillis), label = "Picture")
            Box(frame.background(MaterialTheme.colorScheme.surfaceContainerHigh).combinedClickable(enabled = picture != null, onLongClick = org.sigil.LocalMaterialPress.current) { opened = true },contentAlignment=Alignment.Center) {
                picture?.let { Image(it.asImageBitmap(), file.name, Modifier.fillMaxSize().graphicsLayer { alpha = arrival }, contentScale = if (LocalFillsTile.current) ContentScale.Crop else ContentScale.Fit) }
                if (picture != null) { if (file.mediaType == "image/gif") org.sigil.GifChip(Modifier.align(Alignment.TopStart)) }
                else if(failed) SigilIconButton({requested=true}) {Glyph("refresh",28,"Retry image")}
                else CircularProgressIndicator(Modifier.size(28.dp).graphicsLayer { alpha = 1f - arrival })
            }
        }
        else {
            Text(file.name, maxLines = 2)
            Text(if (file.bytes >= 1024 * 1024) "${file.bytes / (1024 * 1024)} MB" else "${file.bytes / 1024} KB", style = MaterialTheme.typography.labelSmall)
            if (requested && !ready) LinearProgressIndicator(Modifier.fillMaxWidth())
            SigilTextButton({ if (!ready) requested = true else opened = true }, enabled = !requested || ready) {
                Glyph(if (ready && playable) "play_arrow" else if (ready) "open_in_new" else "download", 22)
                Text(if (failed) "Retry" else if (ready && playable) "Play" else if (ready) "Open" else "Download")
            }
        }
    }
    if (opened) {
        if (image && bitmap != null) MediaCarousel(message) { opened = false }
        else if (playable) VideoDialog(message) { opened = false }
        else if ((file.mediaType == "application/pdf" || file.name.endsWith(".pdf",ignoreCase=true)) && file.bytes <= 128L*1024*1024) PdfViewer(message) { opened = false }
        else if (portableFormat(file.name,file.mediaType)!=null && file.bytes<=128L*1024*1024) FileViewer(message,portableFormat(file.name,file.mediaType)!!) { opened=false }
        else Dialog({ opened = false }) {
            Surface(shape = androidx.compose.foundation.shape.RoundedCornerShape(24.dp)) {
                Column(Modifier.fillMaxWidth().heightIn(max = 600.dp).verticalScroll(rememberScrollState()).padding(20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Glyph("file_present", 32)
                    Text(file.name, style = MaterialTheme.typography.titleMedium)
                    Text("${file.bytes} bytes · ${file.mediaType}", style = MaterialTheme.typography.bodySmall)
                    Text("An in-app preview is unavailable for this file.", style = MaterialTheme.typography.bodyMedium)
                    if (file.caption.isNotBlank()) org.sigil.MessageText(file.caption, org.sigil.NativeCore::analyze)
                    SigilTextButton({ NativeFileProvider.open(context, message) }) { Text("Open externally") }
                    SigilTextButton({ opened = false }) { Text("Close") }
                }
            }
        }
    }
}
@Composable
internal fun VideoPlayer(message: ChatMessage, active: Boolean, modifier: Modifier) {
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
        try { player.setDataSource(media) } catch (_: Exception) { failed = true }
        player.setOnPreparedListener { duration = it.duration.toLong().coerceAtLeast(0); ready = true; if (lifecycle.lifecycle.currentState.isAtLeast(androidx.lifecycle.Lifecycle.State.STARTED)) { it.start(); playing = true } }
        player.setOnVideoSizeChangedListener { _, width, height -> if (width > 0 && height > 0) ratio = width.toFloat() / height }
        player.setOnCompletionListener { playing = false }
        player.setOnErrorListener { _, _, _ -> failed = true; playing = false; true }
        onDispose { released = true; player.release(); media.close() }
    }
    // Only the page in view plays; a page swiped away falls silent.
    LaunchedEffect(active) { if (!active && ready && !failed && playing) { player.pause(); playing = false } }
    Box(modifier) {
        Column(Modifier.widthIn(max = 1000.dp).fillMaxSize(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            org.sigil.MediaViewerFrame((ratio * 1000).toInt(), 1000, Modifier.weight(1f)) { frame ->
                AndroidView(factory = { context -> TextureView(context).apply {
                    surfaceTextureListener = object : TextureView.SurfaceTextureListener {
                        private var surface: Surface? = null
                        override fun onSurfaceTextureAvailable(texture: android.graphics.SurfaceTexture, width: Int, height: Int) { surface = Surface(texture); player.setSurface(surface); if (!prepared && !failed) { prepared = true; try { player.prepareAsync() } catch (_: Exception) { failed = true } } }
                        override fun onSurfaceTextureSizeChanged(texture: android.graphics.SurfaceTexture, width: Int, height: Int) {}
                        override fun onSurfaceTextureUpdated(texture: android.graphics.SurfaceTexture) {}
                        override fun onSurfaceTextureDestroyed(texture: android.graphics.SurfaceTexture): Boolean { if (!released) player.setSurface(null); surface?.release(); surface = null; return true }
                    }
                } }, modifier = frame)
            }
            if (failed) Text("This device could not play the attachment.")
            Surface(shape = androidx.compose.foundation.shape.RoundedCornerShape(24.dp), color = MaterialTheme.colorScheme.surfaceContainerHigh, contentColor = MaterialTheme.colorScheme.onSurface) {
                Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                    org.sigil.SigilIconButton({ if (playing) player.pause() else player.start(); playing = !playing }, enabled = ready && !failed) { Glyph(if (playing) "pause" else "play_arrow", 24, if (playing) "Pause" else "Play") }
                    if (ready && !failed && duration > 0) {
                        Slider(position.toFloat().coerceIn(0f, duration.toFloat()), { seeking = true; position = it.toLong() }, Modifier.weight(1f).testTag("media-seek"), valueRange = 0f..duration.toFloat(), onValueChangeFinished = { player.seekTo(position, MediaPlayer.SEEK_CLOSEST); seeking = false })
                        Text("${mediaTime(position)} / ${mediaTime(duration)}", Modifier.padding(start = 8.dp), style = MaterialTheme.typography.labelSmall, maxLines = 1)
                    } else if (!failed) CircularProgressIndicator(Modifier.padding(12.dp).size(20.dp), strokeWidth = 2.dp)
                }
            }
        }
    }
}
@Composable
internal fun VideoDialog(message: ChatMessage, close: () -> Unit) = MediaCarousel(message, close)
private fun mediaTime(milliseconds: Long): String { val seconds = milliseconds / 1000; return "${seconds / 60}:${(seconds % 60).toString().padStart(2, '0')}" }

// A cropped glimpse for reply previews: the picture, or a video's first frame.
@Composable
internal fun AndroidThumbnail(message: ChatMessage, modifier: Modifier): Boolean {
    val file = message.attachment ?: return false
    if (!file.mediaType.startsWith("image/") && !file.mediaType.startsWith("video/")) return false
    val context = LocalContext.current
    val cache = LocalImageCache.current
    var bitmap by remember(message.id) { mutableStateOf<Bitmap?>(null) }
    LaunchedEffect(message.id, cache) {
        bitmap = runCatching {
            if (file.mediaType.startsWith("image/")) historyBitmap(context, message, cache)
            else withContext(Dispatchers.IO) { EncryptedMedia(context, message).use { source ->
                android.media.MediaMetadataRetriever().let { retriever ->
                    try { retriever.setDataSource(source); if (Build.VERSION.SDK_INT >= 27) retriever.getScaledFrameAtTime(0, android.media.MediaMetadataRetriever.OPTION_CLOSEST_SYNC, 200, 200) else null }
                    finally { retriever.release() }
                }
            } }
        }.getOrNull()
    }
    bitmap?.let { androidx.compose.foundation.Image(it.asImageBitmap(), null, modifier, contentScale = androidx.compose.ui.layout.ContentScale.Crop) }
    return true
}
