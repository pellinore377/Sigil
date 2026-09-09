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
    private var levels = emptyList<Float>()
    private var elapsed = 0L
    private var preview: MediaPlayer? = null
    private var previewSource: MediaDataSource? = null
    private fun clearPreview() { preview?.release(); preview = null; previewSource?.close(); previewSource = null }
    fun playPreview() { scope.launch(Dispatchers.IO) { mutex.withLock {
        if (closed.get() || recorder != null || bytes.size() == 0) return@withLock
        try {
            val current = preview
            if (current != null) { if (current.isPlaying) current.pause() else current.start() }
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
                player.setDataSource(source); player.prepare()
                player.setOnCompletionListener { scope.launch(Dispatchers.IO) { mutex.withLock {
                    if (preview === player) { clearPreview(); withContext(Dispatchers.Main) { update(VoiceState("Ready", peer, elapsed, levels)) } }
                } } }
                player.start()
            }
            withContext(Dispatchers.Main) { update(VoiceState("Ready", peer, elapsed, levels, preview?.isPlaying == true)) }
        } catch (_: Exception) { clearPreview(); withContext(Dispatchers.Main) { issue("Could not play this recording."); update(VoiceState("Ready", peer, elapsed, levels)) } }
    } } }
    fun pausePreview() { scope.launch(Dispatchers.IO) { mutex.withLock {
        if (preview != null) { clearPreview(); withContext(Dispatchers.Main) { update(VoiceState("Ready", peer, elapsed, levels)) } }
    } } }
    fun start(peer: String, target: Map<String, Any?> = emptyMap()) { scope.launch(Dispatchers.IO) { mutex.withLock {
        if (closed.get() || recorder != null) return@withLock
        if (bytes.size() > 0) { withContext(Dispatchers.Main) { issue("Send or discard your recording before starting another.") }; return@withLock }
        clearPreview()
        bytes.clear(); this@VoiceRecorder.peer = peer; this@VoiceRecorder.target = target.toMap(); levels = emptyList()
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
                val capture = recorder ?: return@withLock
                elapsed = (SystemClock.elapsedRealtime() - started) / 1000
                val level = runCatching { capture.maxAmplitude / 32767f }.getOrDefault(0f)
                levels = (levels + level).takeLast(48)
                withContext(Dispatchers.Main) { update(VoiceState("Recording", peer, elapsed, levels)) }
            }
        }
    } }
    private suspend fun finish(): Boolean {
        val capture = recorder ?: return bytes.size() > 0
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
        withContext(Dispatchers.Main) { update(if (ready) VoiceState("Ready", peer, elapsed, levels) else VoiceState()) }
    } } }
    fun discard() { scope.launch(Dispatchers.IO) { mutex.withLock {
        clearPreview()
        finish(); bytes.clear(); withContext(Dispatchers.Main) { update(VoiceState()) }
    } } }
    fun send() { scope.launch(Dispatchers.IO) { mutex.withLock {
        clearPreview()
        if (!finish()) return@withLock
        withContext(Dispatchers.Main) { update(VoiceState("Sending", peer, elapsed, levels)) }
        val audio = bytes.toByteArray()
        try {
            stage(peer, audio, target)
            bytes.clear()
            withContext(Dispatchers.Main) { update(VoiceState()) }
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { withContext(Dispatchers.Main) { issue("Could not queue this recording. You can retry sending it."); update(VoiceState("Ready", peer, elapsed, levels)) } }
        finally { audio.fill(0) }
    } } }
    fun close() {
        if (!closed.compareAndSet(false, true)) return
        scope.launch(NonCancellable + Dispatchers.IO) { mutex.withLock { clearPreview(); finish(); runCatching { pipe?.close() }; bytes.clear() } }
    }
}
