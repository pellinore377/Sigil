package org.sigil.compose

import android.content.Context
import android.media.AudioFormat
import android.media.MediaCodec
import android.media.MediaExtractor
import android.media.MediaFormat
import android.os.SystemClock
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import org.sigil.ChatMessage
import java.nio.ByteOrder
import kotlin.math.abs

internal suspend fun audioWaveform(context: Context, message: ChatMessage, duration: Long): List<Float> {
    if (message.attachment!!.bytes !in 1..8 * 1024 * 1024 || duration !in 1..600_000) return emptyList()
    EncryptedMedia(context, message).use { source ->
        val extractor = MediaExtractor()
        var decoder: MediaCodec? = null
        try {
            extractor.setDataSource(source)
            val track = (0 until extractor.trackCount).firstOrNull { extractor.getTrackFormat(it).getString(MediaFormat.KEY_MIME)?.startsWith("audio/") == true } ?: return emptyList()
            extractor.selectTrack(track)
            val format = extractor.getTrackFormat(track)
            val codec = MediaCodec.createDecoderByType(format.getString(MediaFormat.KEY_MIME)!!)
            decoder = codec
            codec.configure(format, null, null, 0); codec.start()
            val levels = FloatArray(64)
            val info = MediaCodec.BufferInfo()
            var inputDone = false
            val deadline = SystemClock.elapsedRealtime() + 8000
            while (SystemClock.elapsedRealtime() < deadline) {
                currentCoroutineContext().ensureActive()
                if (!inputDone) {
                    val index = codec.dequeueInputBuffer(0)
                    if (index >= 0) {
                        val buffer = codec.getInputBuffer(index) ?: return emptyList()
                        val count = extractor.readSampleData(buffer, 0)
                        inputDone = count < 0
                        codec.queueInputBuffer(index, 0, count.coerceAtLeast(0), if (inputDone) 0 else extractor.sampleTime, if (inputDone) MediaCodec.BUFFER_FLAG_END_OF_STREAM else 0)
                        if (!inputDone) extractor.advance()
                    }
                }
                val index = codec.dequeueOutputBuffer(info, 10_000)
                if (index < 0) continue
                try {
                    val output = codec.getOutputBuffer(index) ?: return emptyList()
                    val pcm = codec.outputFormat
                    val rate = pcm.getInteger(MediaFormat.KEY_SAMPLE_RATE)
                    val channels = pcm.getInteger(MediaFormat.KEY_CHANNEL_COUNT)
                    val encoding = if (pcm.containsKey(MediaFormat.KEY_PCM_ENCODING)) pcm.getInteger(MediaFormat.KEY_PCM_ENCODING) else AudioFormat.ENCODING_PCM_16BIT
                    if (rate !in 8000..96000 || channels !in 1..2 || encoding !in listOf(AudioFormat.ENCODING_PCM_16BIT, AudioFormat.ENCODING_PCM_FLOAT)) return emptyList()
                    if (info.presentationTimeUs !in 0..600_000_000) return emptyList()
                    output.position(info.offset); output.limit(info.offset + info.size); output.order(ByteOrder.LITTLE_ENDIAN)
                    val width = if (encoding == AudioFormat.ENCODING_PCM_FLOAT) 4 else 2
                    var frame = 0
                    while (output.remaining() >= width * channels) {
                        val time = info.presentationTimeUs + frame++ * 1_000_000L / rate
                        val bin = (time * levels.size / (duration * 1000)).coerceIn(0, 63).toInt()
                        repeat(channels) {
                            val value = if (width == 4) abs(output.float) else abs(output.short.toInt()) / 32768f
                            if (value.isFinite()) levels[bin] = maxOf(levels[bin], value.coerceAtMost(1f))
                        }
                    }
                } finally { codec.releaseOutputBuffer(index, false) }
                if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) return levels.toList()
            }
            return emptyList()
        } finally { decoder?.release(); extractor.release() }
    }
}
