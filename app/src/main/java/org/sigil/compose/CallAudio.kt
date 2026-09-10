package org.sigil.compose

import android.media.*
import android.media.audiofx.AcousticEchoCanceler
import android.media.audiofx.NoiseSuppressor
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.abs

internal class OpusEncoder(private val encoded: (Long, ByteArray) -> Unit) : AutoCloseable {
    private val codec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_AUDIO_OPUS)
    private var samples = 0L
    init {
        try {
            val format = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_OPUS, 48000, 1)
            format.setInteger(MediaFormat.KEY_BIT_RATE, 32000)
            format.setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, 3840)
            codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            codec.start()
        } catch (error: Exception) { codec.release(); throw error }
    }
    fun input(pcm: ShortArray, count: Int) {
        require(count in 0..pcm.size)
        var index = codec.dequeueInputBuffer(10000)
        val deadline = android.os.SystemClock.elapsedRealtime() + 1000
        while (index < 0) {
            if (Thread.currentThread().isInterrupted) throw InterruptedException()
            check(android.os.SystemClock.elapsedRealtime() < deadline) { "Audio encoder stalled" }
            drain()
            index = codec.dequeueInputBuffer(10000)
        }
        val input = requireNotNull(codec.getInputBuffer(index)).order(ByteOrder.nativeOrder())
        input.clear(); require(count * 2 <= input.remaining()); input.asShortBuffer().put(pcm, 0, count)
        codec.queueInputBuffer(index, 0, count * 2, samples * 1_000_000 / 48000, 0)
        samples += count
        drain()
    }
    fun advance(count: Int) { require(count >= 0); samples += count }
    fun drain() {
        val info = MediaCodec.BufferInfo()
        while (true) {
            val index = codec.dequeueOutputBuffer(info, 0)
            if (index == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) continue
            if (index < 0) return
            try {
                if (info.size > 0 && info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG == 0) {
                    require(info.size <= 8192)
                    val bytes = ByteArray(info.size)
                    try { requireNotNull(codec.getOutputBuffer(index)).apply { position(info.offset); limit(info.offset + info.size) }.get(bytes); encoded(info.presentationTimeUs.coerceAtLeast(0), bytes) }
                    finally { bytes.fill(0) }
                }
            } finally { codec.releaseOutputBuffer(index, false) }
        }
    }
    override fun close() { try { codec.stop() } finally { codec.release() } }
}
internal class OpusDecoder(private val pcm: (ShortArray, Int) -> Unit) : AutoCloseable {
    private val codec = MediaCodec.createDecoderByType(MediaFormat.MIMETYPE_AUDIO_OPUS)
    init {
        try {
            val format = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_OPUS, 48000, 1)
            val header = ByteBuffer.allocate(19).order(ByteOrder.LITTLE_ENDIAN).put("OpusHead".toByteArray(Charsets.US_ASCII)).put(1).put(1).putShort(0).putInt(48000).putShort(0).put(0)
            header.flip(); format.setByteBuffer("csd-0", header)
            format.setByteBuffer("csd-1", ByteBuffer.allocate(8).order(ByteOrder.nativeOrder()).putLong(0).apply { flip() })
            format.setByteBuffer("csd-2", ByteBuffer.allocate(8).order(ByteOrder.nativeOrder()).putLong(80_000_000).apply { flip() })
            codec.configure(format, null, null, 0); codec.start()
        } catch (error: Exception) { codec.release(); throw error }
    }
    fun input(timestamp: Long, bytes: ByteArray) {
        require(bytes.size in 1..8192)
        var index = codec.dequeueInputBuffer(10000)
        val deadline = android.os.SystemClock.elapsedRealtime() + 1000
        while (index < 0) {
            if (Thread.currentThread().isInterrupted) throw InterruptedException()
            check(android.os.SystemClock.elapsedRealtime() < deadline) { "Audio decoder stalled" }
            drain()
            index = codec.dequeueInputBuffer(10000)
        }
        val input = requireNotNull(codec.getInputBuffer(index)); input.clear(); require(input.remaining() >= bytes.size); input.put(bytes)
        codec.queueInputBuffer(index, 0, bytes.size, timestamp, 0)
        drain()
    }
    fun drain() {
        val info = MediaCodec.BufferInfo()
        while (true) {
            val index = codec.dequeueOutputBuffer(info, 0)
            if (index == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) continue
            if (index < 0) return
            try {
                if (info.size > 0) {
                    require(info.size <= 48000)
                    val output = requireNotNull(codec.getOutputBuffer(index)).order(ByteOrder.nativeOrder())
                    output.position(info.offset); output.limit(info.offset + info.size)
                    val samples = ShortArray(info.size / 2)
                    try { output.asShortBuffer().get(samples); pcm(samples, samples.size) } finally { samples.fill(0) }
                }
            } finally { codec.releaseOutputBuffer(index, false) }
        }
    }
    override fun close() { try { codec.stop() } finally { codec.release() } }
}
internal class CallMicrophone(private val send: (Long, ByteArray) -> Unit, private val level: (Float) -> Unit, private val failed: () -> Unit) : AutoCloseable {
    private val running = AtomicBoolean(true)
    @Volatile var muted = false
    @Volatile private var recorder: AudioRecord? = null
    private val worker = Thread({
        var capture: AudioRecord? = null
        var aec: AcousticEchoCanceler? = null
        var noise: NoiseSuppressor? = null
        val samples = ShortArray(960)
        try {
            val size = maxOf(7680, AudioRecord.getMinBufferSize(48000, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT))
            capture = AudioRecord(MediaRecorder.AudioSource.VOICE_COMMUNICATION, 48000, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, size)
            check(capture.state == AudioRecord.STATE_INITIALIZED)
            recorder = capture
            if (AcousticEchoCanceler.isAvailable()) aec = AcousticEchoCanceler.create(capture.audioSessionId)?.apply { enabled = true }
            if (NoiseSuppressor.isAvailable()) noise = NoiseSuppressor.create(capture.audioSessionId)?.apply { enabled = true }
            OpusEncoder(send).use { encoder ->
                if (running.get()) capture.startRecording()
                var tick = 0
                while (running.get()) {
                    val count = capture.read(samples, 0, samples.size, AudioRecord.READ_BLOCKING)
                    if (!running.get()) break
                    check(count > 0)
                    if (muted) samples.fill(0)
                    if (tick++ % 5 == 0) { var peak = 0; for (i in 0 until count) peak = maxOf(peak, abs(samples[i].toInt())); level(peak / 32768f) }
                    if (!muted) encoder.input(samples, count) else encoder.advance(count)
                    samples.fill(0)
                }
            }
        } catch (_: Exception) { if (running.get()) failed() }
        finally { running.set(false); recorder = null; samples.fill(0); try { capture?.stop() } catch (_: Exception) {}; aec?.release(); noise?.release(); capture?.release() }
    }, "Sigil microphone").apply { start() }
    override fun close() { running.set(false); try { recorder?.stop() } catch (_: Exception) {}; worker.interrupt() }
}
internal class CallSpeaker(private val level: (Float) -> Unit, private val failed: () -> Unit) : AutoCloseable {
    private val running = AtomicBoolean(true)
    private val queue = ArrayBlockingQueue<Pair<Long, ByteArray>>(8)
    @Volatile private var output: AudioTrack? = null
    private val worker = Thread({
        var track: AudioTrack? = null
        try {
            track = AudioTrack.Builder().setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION).setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build())
                .setAudioFormat(AudioFormat.Builder().setSampleRate(48000).setChannelMask(AudioFormat.CHANNEL_OUT_MONO).setEncoding(AudioFormat.ENCODING_PCM_16BIT).build())
                .setBufferSizeInBytes(maxOf(7680, AudioTrack.getMinBufferSize(48000, AudioFormat.CHANNEL_OUT_MONO, AudioFormat.ENCODING_PCM_16BIT))).setTransferMode(AudioTrack.MODE_STREAM).build()
            output = track
            var primed = 0
            var playing = false
            var tick = 0
            OpusDecoder { samples, count ->
                if (tick++ % 5 == 0) level((samples.maxOfOrNull { abs(it.toInt()) } ?: 0) / 32768f)
                var offset = 0
                while (running.get() && offset < count) {
                    val requested = if (playing) count - offset else minOf(count - offset, 2880 - primed)
                    val written = track.write(samples, offset, requested, AudioTrack.WRITE_BLOCKING)
                    check(written > 0); offset += written
                    if (!playing) {
                        primed += written
                        if (primed >= 2880) { track.play(); playing = true }
                    }
                }
            }.use { decoder ->
                while (running.get()) {
                    val packet = queue.poll(10, TimeUnit.MILLISECONDS)
                    if (packet == null) { decoder.drain(); continue }
                    val (timestamp, bytes) = packet
                    try { decoder.input(timestamp, bytes) } finally { bytes.fill(0) }
                }
            }
        } catch (_: InterruptedException) { }
        catch (_: Exception) { if (running.get()) failed() }
        finally { synchronized(queue) { running.set(false); drain() }; output = null; try { track?.stop() } catch (_: Exception) {}; track?.release() }
    }, "Sigil speaker").apply { start() }
    fun offer(timestamp: Long, bytes: ByteArray) = synchronized(queue) {
        if (!running.get()) return
        val copy = bytes.copyOf()
        if (!queue.offer(timestamp to copy)) copy.fill(0)
    }
    private fun drain() { while (true) { val value = queue.poll() ?: break; value.second.fill(0) } }
    override fun close() { synchronized(queue) { running.set(false); drain() }; worker.interrupt(); try { output?.pause(); output?.flush() } catch (_: Exception) {} }
}
