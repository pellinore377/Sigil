@file:OptIn(androidx.compose.ui.ExperimentalComposeUiApi::class)
package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.WebElementView
import kotlinx.browser.document
import kotlinx.coroutines.*
import org.khronos.webgl.Uint8Array
import org.khronos.webgl.get
import org.w3c.dom.HTMLAudioElement
import org.w3c.dom.HTMLIFrameElement
import kotlin.js.*

// Glimpses already read stay with the session; a row scrolled back does not read its file again.
private val peeks = LinkedHashMap<String, FilePeek>()
private const val AutoFetchBytes = 8L * 1024 * 1024
private fun WebFile.key() = "$peer/$author/$message"

internal fun WebFile.isVoiceNote() = type.startsWith("audio/") && (name == "Voice message.aac" || draft.isNotEmpty() || levels.isNotEmpty())

// The first stored chunk of a delivered file, at most the requested bytes.
private suspend fun webHead(file: WebFile, limit: Int): ByteArray {
    val chunk = browserFileRead(file.peer, file.author, file.message, 0).awaitBrowser<JsAny>() as Uint8Array
    try {
        val n = minOf(limit, chunk.length)
        val out = ByteArray(n)
        for (i in 0 until n) out[i] = chunk[i]
        return out
    } finally { browserReleaseBytes(chunk) }
}

private suspend fun webPeek(file: WebFile, kind: AttachmentKind, url: String): FilePeek? {
    val extension = file.name.substringAfterLast('.', "").lowercase()
    return when (kind) {
        AttachmentKind.Markdown, AttachmentKind.Text -> FilePeek.Text(webHead(file, 64 * 1024).decodeToString().take(4000), kind == AttachmentKind.Markdown)
        AttachmentKind.Sheet -> if (extension == "csv" || extension == "tsv") FilePeek.Table(parseDelimited(webHead(file, 64 * 1024).decodeToString(), if (extension == "tsv") '\t' else ',', 16, 8)) else null
        AttachmentKind.Audio -> {
            val tags = readTrackTags(webHead(file, 2 * 1024 * 1024))
            val audio = (document.createElement("audio") as HTMLAudioElement).apply { src = url; preload = "metadata" }
            val duration = try { resolveWebAudioDuration(audio).takeIf { it > 0 } } finally { audio.removeAttribute("src"); audio.load() }
            FilePeek.Track(tags, tags?.picture?.let(::decodeImage), duration ?: tags?.lengthMs)
        }
        else -> null
    }
}

// Documents, sheets, tracks and plain files in the timeline; pictures and clips keep their own frames.
@Composable internal fun WebFileCard(file: WebFile, load: suspend (WebFile) -> String, open: (WebFile) -> Unit) {
    val kind = attachmentKind(file.name, file.type)
    var peek by remember(file) { mutableStateOf(peeks[file.key()]) }
    var requested by remember(file) { mutableStateOf(file.bytes <= AutoFetchBytes && kind != AttachmentKind.File) }
    var ready by remember(file) { mutableStateOf(false) }
    var glimpsing by remember(file) { mutableStateOf(false) }
    var openWhenReady by remember(file) { mutableStateOf(false) }
    LaunchedEffect(file, requested) {
        if (!requested) return@LaunchedEffect
        try {
            val url = load(file)
            ready = true
            if (peek == null && kind != AttachmentKind.File) {
                glimpsing = true
                peek = runCatching { webPeek(file, kind, url) }.getOrNull()?.also { if (peeks.size >= 96) peeks.remove(peeks.keys.first()); peeks[file.key()] = it }
                glimpsing = false
            }
            if (openWhenReady) { openWhenReady = false; open(file) }
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { requested = false; glimpsing = false }
    }
    val onOpen = { if (ready || file.bytes > 128L * 1024 * 1024) open(file) else { openWhenReady = true; requested = true } }
    val shape = androidx.compose.ui.graphics.RectangleShape
    when (kind) {
        AttachmentKind.Audio -> {
            val track = peek as? FilePeek.Track
            val tint = remember(track?.art) { track?.art?.let(::dominantColor) }
            AudioCard(track?.tags?.title ?: file.name.substringBeforeLast('.'), listOfNotNull(track?.durationMs?.let(::trackTime), attachmentSize(file.bytes)).joinToString(" · "), track?.art, tint, shape, onOpen)
        }
        AttachmentKind.File -> FileChip(file.name, file.bytes, requested && !ready, onOpen)
        else -> DocumentCard(file.name, kind, file.bytes, peek, requested && (!ready || glimpsing), shape, onOpen)
    }
}

private class WebTrackPlayback(private val audio: HTMLAudioElement, private val scope: CoroutineScope) : TrackPlayback {
    override var ready by mutableStateOf(false)
    override var failed by mutableStateOf(false)
    override var playing by mutableStateOf(false)
    override var position by mutableLongStateOf(0L)
    override var duration by mutableLongStateOf(0L)
    override fun toggle() {
        if (!ready) return
        if (!audio.paused) audio.pause() else scope.launch { try { audio.play().awaitBrowser<JsAny?>(); failed = false } catch (_: Exception) { failed = true } }
    }
    override fun seek(ms: Long) { audio.currentTime = ms / 1000.0; position = ms }
}

// The full-screen reader for anything the timeline showed as a card.
@Composable internal fun WebDocumentViewer(file: WebFile, load: suspend (WebFile) -> String, close: () -> Unit) {
    val kind = attachmentKind(file.name, file.type)
    val save = LocalWebFileSave.current
    val scope = rememberCoroutineScope()
    var issue by remember { mutableStateOf<String?>(null) }
    var saving by remember { mutableStateOf(false) }
    var url by remember(file) { mutableStateOf<String?>(null) }
    var text by remember(file) { mutableStateOf<String?>(null) }
    var failed by remember(file) { mutableStateOf(false) }
    val download = {
        val destination = if (browserFileStreamSupported()) browserFileDestination(file.name) else null
        scope.launch {
            saving = true
            try {
                if (destination != null) destination.awaitBrowser<JsAny?>()?.let { save(file, it) }
                else browserSaveFileUrl(load(file), file.name)
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { issue = "Could not save this attachment." }
            finally { saving = false }
        }
        Unit
    }
    LaunchedEffect(file) {
        try {
            url = load(file)
            if (kind == AttachmentKind.Markdown || kind == AttachmentKind.Text || (kind == AttachmentKind.Sheet && file.name.substringAfterLast('.', "").lowercase() in setOf("csv", "tsv")))
                text = webHead(file, 1024 * 1024).decodeToString()
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { failed = true }
    }
    if (kind == AttachmentKind.Audio) {
        val current = url
        if (current == null) Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
        else {
            val audio = remember(current) { (document.createElement("audio") as HTMLAudioElement).apply { src = current; preload = "auto" } }
            val playback = remember(current) { WebTrackPlayback(audio, scope) }
            LaunchedEffect(audio) {
                playback.duration = resolveWebAudioDuration(audio); playback.ready = true
                playback.toggle()
                while (isActive) {
                    playback.position = (audio.currentTime * 1000).toLong().coerceAtLeast(0)
                    audio.duration.takeIf { it.isFinite() && it > 0 }?.let { playback.duration = (it * 1000).toLong() }
                    playback.playing = !audio.paused && !audio.ended
                    delay(200)
                }
            }
            DisposableEffect(audio) { onDispose { audio.pause(); audio.removeAttribute("src"); audio.load() } }
            val peek = peeks[file.key()] as? FilePeek.Track
            val track = TrackPresentation(peek?.tags?.title ?: file.name.substringBeforeLast('.'), peek?.tags?.artist, peek?.tags?.album, peek?.art, peek?.tags?.lyrics.orEmpty(),
                file.name.substringAfterLast('.', "").uppercase().ifEmpty { "AUDIO" }, file.bytes)
            TrackPlayerScreen(track, playback, close, download, saving, file.caption)
        }
    } else DocumentViewerChrome(file.name, kind.chip, file.bytes, close, download, saving, file.caption) {
        val current = url
        when {
            failed -> Text("This file could not be displayed.", Modifier.align(Alignment.Center))
            current == null -> CircularProgressIndicator(Modifier.align(Alignment.Center))
            text != null && kind == AttachmentKind.Sheet -> TableDocumentView(parseDelimited(text!!, if (file.name.endsWith(".tsv", true)) '\t' else ',', 4096, 128))
            text != null -> TextDocumentView(text!!, kind == AttachmentKind.Markdown)
            kind == AttachmentKind.Pdf -> {
                // The browser's own PDF reader, framed on the file's object URL.
                var zoom by remember(current) { mutableStateOf(1f) }
                val frame = remember(current) { (document.createElement("iframe") as HTMLIFrameElement).apply { src = current; setAttribute("title", file.name) } }
                WebElementView(factory = { frame }, modifier = Modifier.fillMaxSize(), update = { it.setAttribute("style", "border:0;background:#fff;transform-origin:0 0;transform:scale($zoom);width:${100 / zoom}%;height:${100 / zoom}%") })
                ZoomControls({ zoom = (zoom * 1.25f).coerceAtMost(4f) }, { zoom = (zoom / 1.25f).coerceAtLeast(1f) }, zoom < 4f, zoom > 1f, Modifier.align(Alignment.BottomEnd))
            }
            kind in setOf(AttachmentKind.Markdown, AttachmentKind.Text, AttachmentKind.Sheet) -> CircularProgressIndicator(Modifier.align(Alignment.Center))
            else -> Column(Modifier.align(Alignment.Center).padding(24.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Glyph(kind.glyph, 40)
                Text("Save this file to open it in another app.", style = MaterialTheme.typography.bodyMedium)
                SigilTextButton(download, enabled = !saving) { Text("Save file") }
            }
        }
    }
    issue?.let { message -> AlertDialog({ issue = null }, text = { Text(message) }, confirmButton = { SigilTextButton({ issue = null }) { Text("OK") } }) }
}
