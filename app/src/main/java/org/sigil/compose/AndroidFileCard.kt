package org.sigil.compose

import android.media.MediaMetadataRetriever
import android.media.MediaPlayer
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.ui.Modifier
import androidx.compose.runtime.*
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import org.json.JSONObject
import org.sigil.*

// Glimpses already read stay with the session; a row scrolled back does not open its file again.
// Messages whose file is known to be on hand, so a card composed again starts ready.
private val readyKeys = HashSet<String>()
private val peeks = object : LinkedHashMap<String, FilePeek>(16, .75f, true) {
    override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, FilePeek>?) = size > 96
}
private const val AutoFetchBytes = 8L * 1024 * 1024

// Documents, sheets, tracks and plain files: the card for anything that is not a picture or a clip.
@Composable internal fun AndroidFileCard(message: ChatMessage) {
    val file = message.attachment ?: return
    val kind = attachmentKind(file.name, file.mediaType)
    val context = LocalContext.current
    val key = "${message.peer}/${message.author}/${message.id}"
    var peek by remember(message.id) { mutableStateOf(peeks[key]) }
    var requested by remember(message.id) { mutableStateOf(file.bytes <= AutoFetchBytes && kind != AttachmentKind.File) }
    var ready by remember(message.id) { mutableStateOf(key in readyKeys) }
    var glimpsing by remember(message.id) { mutableStateOf(false) }
    var failed by remember(message.id) { mutableStateOf(false) }
    var opened by remember(message.id) { mutableStateOf(false) }
    var openWhenReady by remember(message.id) { mutableStateOf(false) }
    LaunchedEffect(message.id, requested) {
        if (!requested) return@LaunchedEffect
        failed = false
        try {
            while (!withContext(Dispatchers.IO) { prepare(context, message) }) delay(1000)
            ready = true; readyKeys += key
            if (peek == null && kind != AttachmentKind.File) {
                glimpsing = true
                peek = withContext(Dispatchers.IO) { runCatching { androidPeek(context, message, kind) }.getOrNull() }?.also { peeks[key] = it }
                glimpsing = false
            }
            if (openWhenReady) { opened = true; openWhenReady = false }
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { failed = true; requested = false; glimpsing = false }
    }
    val open = { if (file.draft) Unit else if (ready) opened = true else { openWhenReady = true; requested = true } }
    val shape = androidx.compose.ui.graphics.RectangleShape
    Column(Modifier.widthIn(max = AttachmentCardWidth)) {
    when (kind) {
        AttachmentKind.Audio -> {
            val track = peek as? FilePeek.Track
            val tint = remember(track?.art) { track?.art?.let(::dominantColor) }
            AudioCard(track?.tags?.title ?: file.name.substringBeforeLast('.'), listOfNotNull(track?.durationMs?.let(::trackTime), attachmentSize(file.bytes)).joinToString(" · "), track?.art, tint, shape, open)
        }
        AttachmentKind.File -> FileChip(file.name, file.bytes, requested && !ready, open)
        else -> DocumentCard(file.name, kind, file.bytes, peek, requested && (!ready || glimpsing), shape, open)
    }
    // The caption sits on the bubble ground beneath the card, kept to the card's own width.
    if (kind != AttachmentKind.File && !file.draft && file.caption.isNotBlank()) androidx.compose.foundation.layout.Box(Modifier.fillMaxWidth().background(LocalBubbleGround.current).padding(horizontal = 14.dp, vertical = 10.dp)) { MessageText(file.caption, NativeCore::analyze) }
    }
    if (opened) when {
        kind == AttachmentKind.Pdf && file.bytes <= 128L * 1024 * 1024 -> PdfViewer(message) { opened = false }
        kind == AttachmentKind.Audio -> AndroidTrackPlayer(message, peek as? FilePeek.Track) { opened = false }
        portableFormat(file.name, file.mediaType) != null && file.bytes <= 128L * 1024 * 1024 -> FileViewer(message, portableFormat(file.name, file.mediaType)!!) { opened = false }
        else -> LaunchedEffect(Unit) { NativeFileProvider.open(context, message); opened = false }
    }
}

internal fun attachmentHead(context: android.content.Context, message: ChatMessage, limit: Int): ByteArray = EncryptedMedia(context, message).use { media ->
    val n = minOf(limit.toLong(), media.size).toInt()
    val out = ByteArray(n)
    var at = 0
    while (at < n) { val count = media.readAt(at.toLong(), out, at, n - at); if (count <= 0) break; at += count }
    out.copyOf(at)
}

private suspend fun androidPeek(context: android.content.Context, message: ChatMessage, kind: AttachmentKind): FilePeek? {
    val file = message.attachment!!
    val extension = file.name.substringAfterLast('.', "").lowercase()
    return when (kind) {
        AttachmentKind.Markdown, AttachmentKind.Text -> FilePeek.Text(attachmentHead(context, message, 64 * 1024).decodeToString().take(4000), kind == AttachmentKind.Markdown)
        AttachmentKind.Sheet -> if (extension == "csv" || extension == "tsv") FilePeek.Table(parseDelimited(attachmentHead(context, message, 64 * 1024).decodeToString(), if (extension == "tsv") '\t' else ',', 16, 8))
            else FilePreviewSession(context).use { session ->
                val table = session.render(NativeFileProvider.reader(context, message), "spreadsheet", JSONObject().put("view", "table").put("sheet", 0).put("row", 0).put("column", 0).toString()) as? FilePreview.Table
                table?.let { FilePeek.Table(it.cells.take(16).map { row -> row.take(8) }) }
            }
        AttachmentKind.Pdf, AttachmentKind.Document, AttachmentKind.Slides -> FilePreviewSession(context).use { session ->
            FilePeek.Page(session.render(NativeFileProvider.reader(context, message), 0, 600).bitmap.asImageBitmap())
        }
        AttachmentKind.Audio -> {
            val tags = readTrackTags(attachmentHead(context, message, 2 * 1024 * 1024))
            val duration = runCatching { EncryptedMedia(context, message).use { source ->
                MediaMetadataRetriever().let { retriever -> try { retriever.setDataSource(source); retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)?.toLongOrNull() } finally { retriever.release() } }
            } }.getOrNull() ?: tags?.lengthMs
            FilePeek.Track(tags, tags?.picture?.let(::decodeImage), duration)
        }
        AttachmentKind.File -> null
    }
}

private class AndroidTrackPlayback(private val player: MediaPlayer) : TrackPlayback {
    override var ready by mutableStateOf(false)
    override var failed by mutableStateOf(false)
    override var playing by mutableStateOf(false)
    override var position by mutableLongStateOf(0L)
    override var duration by mutableLongStateOf(0L)
    override fun toggle() {
        if (!ready || failed) return
        if (playing) player.pause() else { if (position >= duration && duration > 0) { player.seekTo(0); position = 0 }; player.start() }
        playing = !playing
    }
    override fun seek(ms: Long) { if (!ready) return; position = ms; player.seekTo(ms, MediaPlayer.SEEK_CLOSEST) }
}

// The full-screen player over an encrypted track; the tags and artwork come from the card's glimpse.
@Composable internal fun AndroidTrackPlayer(message: ChatMessage, peek: FilePeek.Track?, close: () -> Unit) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val file = message.attachment!!
    val media = remember(message.id) { EncryptedMedia(context, message) }
    val player = remember(message.id) { MediaPlayer() }
    val playback = remember(message.id) { AndroidTrackPlayback(player) }
    DisposableEffect(player, lifecycle) {
        player.setOnPreparedListener { playback.duration = it.duration.toLong().coerceAtLeast(0); playback.ready = true; it.start(); playback.playing = true }
        player.setOnCompletionListener { playback.playing = false; playback.position = playback.duration }
        player.setOnSeekCompleteListener { playback.position = it.currentPosition.toLong() }
        player.setOnErrorListener { _, _, _ -> playback.failed = true; playback.playing = false; true }
        try { player.setDataSource(media); player.prepareAsync() } catch (_: Exception) { playback.failed = true }
        val observer = LifecycleEventObserver { _, event -> if (event == Lifecycle.Event.ON_STOP && playback.ready && !playback.failed) { player.pause(); playback.playing = false } }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer); player.release(); media.close() }
    }
    LaunchedEffect(playback.playing) { while (playback.playing) { playback.position = player.currentPosition.toLong(); delay(200) } }
    val saver = rememberAttachmentSaver(message)
    val track = TrackPresentation(peek?.tags?.title ?: file.name.substringBeforeLast('.'), peek?.tags?.artist, peek?.tags?.album, peek?.art, peek?.tags?.lyrics.orEmpty(),
        file.name.substringAfterLast('.', "").uppercase().ifEmpty { "AUDIO" }, file.bytes)
    Presented(close) {
        TrackPlayerScreen(track, playback, close, saver.save, saver.saving, file.caption)
        saver.Notice()
    }
}
