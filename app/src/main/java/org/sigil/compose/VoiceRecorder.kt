package org.sigil.compose

import android.media.MediaRecorder
import android.media.MediaPlayer
import android.media.MediaDataSource
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.sigil.VoiceState
import java.io.ByteArrayOutputStream

internal class VoiceRecorder(private val scope: CoroutineScope, private val stage: suspend (String, ByteArray, Map<String, Any?>) -> Unit,
    private val update: (VoiceState) -> Unit, private val issue: (String) -> Unit) {
    private class Buffer : ByteArrayOutputStream() {
        @Synchronized override fun write(bytes: ByteArray, offset: Int, length: Int) {
            check(count + length <= 8 * 1024 * 1024)
            if (count + length > buf.size) { val prior = buf; buf = prior.copyOf(maxOf(count + length, minOf(8 * 1024 * 1024, prior.size * 2))); prior.fill(0) }
            bytes.copyInto(buf, count, offset, offset + length); count += length
        }
        @Synchronized fun clear() { buf.fill(0); reset() }
    }
    private val mutex = Mutex()
    private val closed = java.util.concurrent.atomic.AtomicBoolean(false)
    private var recorder: MediaRecorder? = null
    private var reader: Deferred<Unit>? = null
    private var pipe: ParcelFileDescriptor? = null
    private val bytes = Buffer()
    private var peer = ""
    private var target = emptyMap<String, Any?>()
    private var started = 0L
    private var pausedAt = 0L
    private var pausedTime = 0L
    private val samples = ArrayList<Float>(6000)
    private var levels = emptyList<Float>()
    private var elapsed = 0L
    private var position = 0L
    private var duration = 0L
    private var preview: MediaPlayer? = null
    private var previewSource: MediaDataSource? = null
    private fun clearPreview() { preview?.release(); preview = null; previewSource?.close(); previewSource = null }
    private fun ready() = VoiceState("Ready", peer, elapsed, levels, preview?.isPlaying == true, position = position, duration = duration)
    private fun recording() = VoiceState("Recording", peer, elapsed, samples.takeLast(48), paused = pausedAt != 0L)
    private fun capturedMillis() = ((if (pausedAt != 0L) pausedAt else SystemClock.elapsedRealtime()) - started - pausedTime).coerceAtLeast(0)
    fun playPreview() { scope.launch(Dispatchers.IO) { mutex.withLock {
        if (closed.get() || recorder != null || bytes.size() == 0) return@withLock
        try {
            val current = preview
            if (current != null) { if (current.isPlaying) current.pause() else { if (position >= duration) { position = 0; current.seekTo(0) }; current.start() } }
            else {
                val audio = bytes.toByteArray()
                val source = object : MediaDataSource() {
                    override fun getSize() = audio.size.toLong()
                    @Synchronized override fun readAt(position: Long, buffer: ByteArray, offset: Int, size: Int): Int {
                        require(position >= 0 && offset >= 0 && size >= 0 && offset <= buffer.size - size)
                        if (size == 0) return 0
                        if (position >= audio.size) return -1
                        val count = minOf(size, audio.size - position.toInt())
                        audio.copyInto(buffer, offset, position.toInt(), position.toInt() + count)
                        return count
                    }
                    @Synchronized override fun close() { audio.fill(0) }
                }
                previewSource = source
                val player = MediaPlayer(); preview = player
                player.setDataSource(source); player.prepare(); duration = player.duration.toLong().coerceAtLeast(0)
                if (position >= duration) position = 0
                if (position > 0) player.seekTo(position, MediaPlayer.SEEK_CLOSEST)
                player.setOnCompletionListener { scope.launch(Dispatchers.IO) { mutex.withLock {
                    if (preview === player) { position = duration; withContext(Dispatchers.Main) { update(ready()) } }
                } } }
                player.start()
            }
            withContext(Dispatchers.Main) { update(ready()) }
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { clearPreview(); withContext(Dispatchers.Main) { issue("Could not play this recording."); update(ready()) } }
    } } }
    fun pausePreview() { scope.launch(Dispatchers.IO) { mutex.withLock {
        preview?.let { if (it.isPlaying) it.pause(); position = it.currentPosition.toLong(); withContext(Dispatchers.Main) { update(ready()) } }
    } } }
    fun seek(milliseconds: Long) { scope.launch(Dispatchers.IO) { mutex.withLock {
        if (closed.get() || recorder != null || bytes.size() == 0) return@withLock
        position = milliseconds.coerceIn(0, duration)
        preview?.seekTo(position, MediaPlayer.SEEK_CLOSEST)
        withContext(Dispatchers.Main) { update(ready()) }
    } } }
    fun pauseRecording() { scope.launch(Dispatchers.IO) { mutex.withLock {
        val capture = recorder ?: return@withLock
        try {
            if (pausedAt == 0L) { capture.pause(); pausedAt = SystemClock.elapsedRealtime() }
            else { capture.resume(); pausedTime += SystemClock.elapsedRealtime() - pausedAt; pausedAt = 0 }
            elapsed = capturedMillis() / 1000
            withContext(Dispatchers.Main) { update(recording()) }
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { withContext(Dispatchers.Main) { issue("Could not pause or resume recording. You can stop and keep it.") } }
    } } }
    fun start(peer: String, target: Map<String, Any?> = emptyMap()) { scope.launch(Dispatchers.IO) { mutex.withLock {
        if (closed.get() || recorder != null) return@withLock
        if (bytes.size() > 0) { withContext(Dispatchers.Main) { issue("Send or discard your recording before starting another.") }; return@withLock }
        clearPreview()
        bytes.clear(); this@VoiceRecorder.peer = peer; this@VoiceRecorder.target = target.toMap(); levels = emptyList(); samples.clear()
        pausedAt = 0; pausedTime = 0; elapsed = 0; position = 0; duration = 0
        val descriptors = ParcelFileDescriptor.createPipe()
        pipe = descriptors[0]
        reader = scope.async(Dispatchers.IO) {
            ParcelFileDescriptor.AutoCloseInputStream(descriptors[0]).use { input ->
                val part = ByteArray(8192)
                try { while (true) { val n = input.read(part); if (n < 0) break; bytes.write(part, 0, n) } }
                finally { part.fill(0) }
            }
        }
        @Suppress("DEPRECATION") val capture = MediaRecorder()
        recorder = capture
        try {
            capture.setAudioSource(MediaRecorder.AudioSource.MIC)
            capture.setOutputFormat(MediaRecorder.OutputFormat.AAC_ADTS)
            capture.setAudioEncoder(MediaRecorder.AudioEncoder.AAC)
            capture.setAudioChannels(1); capture.setAudioSamplingRate(48000); capture.setAudioEncodingBitRate(64000)
            capture.setMaxDuration(600000)
            capture.setOutputFile(descriptors[1].fileDescriptor)
            capture.setOnInfoListener { _, what, _ -> if (what == MediaRecorder.MEDIA_RECORDER_INFO_MAX_DURATION_REACHED) stop() }
            capture.setOnErrorListener { _, _, _ -> discard() }
            capture.prepare(); capture.start(); started = SystemClock.elapsedRealtime()
            withContext(Dispatchers.Main) { update(VoiceState("Recording", peer)) }
        } catch (_: Exception) {
            capture.release(); recorder = null; pipe?.close(); bytes.clear()
            withContext(Dispatchers.Main) { issue("Could not start the microphone."); update(VoiceState()) }
        } finally { descriptors[1].close() }
    } } }
    init { scope.launch(Dispatchers.IO) {
        while (isActive) {
            delay(100)
            mutex.withLock {
                val capture = recorder
                if (capture != null) {
                    elapsed = capturedMillis() / 1000
                    if (pausedAt == 0L && samples.size < 6000) samples += runCatching { capture.maxAmplitude / 32767f }.getOrDefault(0f)
                    withContext(Dispatchers.Main) { update(recording()) }
                } else preview?.let { player ->
                    if (player.isPlaying) { position = player.currentPosition.toLong(); withContext(Dispatchers.Main) { update(ready()) } }
                }
            }
        }
    } }
    private suspend fun finish(): Boolean {
        val capture = recorder ?: return bytes.size() > 0
        duration = capturedMillis(); elapsed = duration / 1000
        levels = if (samples.isEmpty()) emptyList() else List(minOf(64, samples.size)) { index ->
            val count = minOf(64, samples.size)
            (index * samples.size / count until (index + 1) * samples.size / count).maxOf { samples[it] }
        }
        samples.clear()
        recorder = null
        val success = runCatching { capture.stop() }.isSuccess
        capture.release()
        val read = runCatching { reader?.await() }.isSuccess
        withContext(NonCancellable) { reader?.join() }
        reader = null; pipe = null
        if (!success || !read) bytes.clear()
        return bytes.size() > 0
    }
    fun stop() { scope.launch(Dispatchers.IO) { mutex.withLock {
        val ready = finish()
        withContext(Dispatchers.Main) { update(if (ready) ready() else VoiceState()) }
    } } }
    fun discard() { scope.launch(Dispatchers.IO) { mutex.withLock {
        clearPreview()
        finish(); bytes.clear(); withContext(Dispatchers.Main) { update(VoiceState()) }
    } } }
    fun send(caption: String = "") { scope.launch(Dispatchers.IO) { mutex.withLock {
        clearPreview()
        if (!finish()) return@withLock
        withContext(Dispatchers.Main) { update(VoiceState("Sending", peer, elapsed, levels)) }
        val audio = bytes.toByteArray()
        try {
            stage(peer, audio, target + ("caption" to caption))
            bytes.clear()
            withContext(Dispatchers.Main) { update(VoiceState()) }
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { withContext(Dispatchers.Main) { issue("Could not queue this recording. You can retry sending it."); update(ready()) } }
        finally { audio.fill(0) }
    } } }
    fun close() {
        if (!closed.compareAndSet(false, true)) return
        scope.launch(NonCancellable + Dispatchers.IO) { mutex.withLock { clearPreview(); finish(); runCatching { pipe?.close() }; bytes.clear() } }
    }
}
