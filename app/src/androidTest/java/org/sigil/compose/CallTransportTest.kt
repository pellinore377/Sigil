package org.sigil.compose

import android.graphics.ImageFormat
import android.media.ImageReader
import android.os.*
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import org.sigil.storage.NativeStorage
import org.sigil.storage.StorageKeyProvider
import java.io.File
import java.nio.ByteBuffer
import java.util.concurrent.atomic.AtomicIntegerArray
import kotlin.math.*

class CallTransportTest {
    @Test fun audioCameraAndScreenCrossTheEncryptedRelayAndDecodeOnReturn() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        org.junit.Assume.assumeTrue(context.packageName.endsWith(".acceptance"))
        StorageKeyProvider(context).withKey { _, key -> runFixture(context, key) }
    }
    private fun runFixture(context: android.content.Context, key: ByteArray) {
        val directory = File(context.cacheDir, "call-transport-test")
        assertTrue("Synthetic fixture is required", File(directory, "client.db").exists())
        android.system.Os.chmod(directory.path, 448); android.system.Os.chmod(File(directory, "client.db").path, 384)
        var handle = 0L; var audible = 0
        val returned = IntArray(3); val sent = AtomicIntegerArray(3); val rendered = AtomicIntegerArray(2)
        val sendTimes = List(3) { java.util.concurrent.ConcurrentLinkedQueue<Long>() }
        fun send(kind: Int, timestamp: Long, keyframe: Boolean, bytes: ByteArray) {
            val begin = SystemClock.elapsedRealtimeNanos()
            if (NativeStorage.sendCallFrame(handle, kind, timestamp, keyframe, bytes)) {
                sent.incrementAndGet(kind)
                if (sendTimes[kind].size < 1000) sendTimes[kind].add(SystemClock.elapsedRealtimeNanos() - begin)
            }
        }
        val connections = mutableSetOf<Int>()
        val thread = HandlerThread("Call acceptance").apply { start() }
        val readers = List(2) { index -> ImageReader.newInstance(640, 480, ImageFormat.YUV_420_888, 3).apply { setOnImageAvailableListener({ reader -> reader.acquireLatestImage()?.use { rendered.incrementAndGet(index) } }, Handler(thread.looper)) } }
        val videos = readers.map { CallVideoDecoder(it.surface) { _, _, _ -> } }
        fun sync() {
            val response = JSONObject(NativeStorage.execute(directory.path, key, "{\"command\":\"sync\",\"interactive\":true}"))
            assertTrue(response.toString(), response.getBoolean("ok"))
        }
        try {
            assertTrue("Synthetic storage could not reopen", NativeStorage.checkStore(directory.path, key)); sync()
            handle = NativeStorage.openCall(directory.path, key, ByteArray(32) { 97 }, 7)
            assertTrue("Native peer connection failed", handle != 0L)
            val deadline = SystemClock.elapsedRealtime() + 60000
            val pcm = ShortArray(960)
            Vp8Encoder(640, 480, 0, { timestamp, keyframe, bytes -> for (kind in 1..2) send(kind, timestamp, keyframe, bytes) }, { fail("Video encoder failed") }).use { video ->
                DrawSurface(video.surface).use { draw ->
                    OpusDecoder { samples, _ -> audible += samples.count { abs(it.toInt()) > 1000 } }.use { decoder ->
                        OpusEncoder { timestamp, bytes -> send(0, timestamp, false, bytes) }.use { encoder ->
                            var frame = 0
                            while (SystemClock.elapsedRealtime() < deadline && (returned[0] < 80 || audible < 48000 || rendered.get(0) < 10 || rendered.get(1) < 10)) {
                                if (frame % 20 == 0) { sync(); connections += NativeStorage.callState(handle) }
                                pcm.indices.forEach { i -> pcm[i] = (sin((frame * 960 + i) * 2.0 * PI * 330 / 48000) * 12000).toInt().toShort() }
                                encoder.input(pcm, pcm.size)
                                if (frame % 2 == 0) draw.frame(frame / 2)
                                frame++
                                NativeStorage.receiveCallFrames(handle)?.let { bytes ->
                                    try {
                                        val buffer = ByteBuffer.wrap(bytes)
                                        while (buffer.hasRemaining()) {
                                            assertTrue(buffer.remaining() >= 46); buffer.position(buffer.position() + 32)
                                            val kind = buffer.get().toInt(); val keyframe = buffer.get().toInt() != 0
                                            assertTrue(kind in 0..2); if (kind == 0) assertFalse(keyframe)
                                            val timestamp = buffer.long; val count = buffer.int
                                            assertTrue(count in 1..minOf(if (kind == 0) 8192 else 1024 * 1024, buffer.remaining()))
                                            val encoded = ByteArray(count)
                                            try { buffer.get(encoded); if (kind == 0) decoder.input(timestamp, encoded) else videos[kind - 1].offer(timestamp, keyframe, encoded); returned[kind]++ }
                                            finally { encoded.fill(0) }
                                        }
                                    } finally { bytes.fill(0) }
                                }
                                Thread.sleep(20)
                            }
                            pcm.fill(0)
                        }
                    }
                }
            }
            val evidence = "Received ${returned.toList()}; sent $sent; rendered $rendered; states $connections"
            assertTrue(evidence, returned[0] >= 80 && rendered.get(0) >= 10 && rendered.get(1) >= 10)
            assertTrue("Returned audio did not decode", audible >= 48000)
            val times = sendTimes.flatMap { it }.sorted()
            android.util.Log.i("SigilAcceptance", "Native media send: ${times.size} frames, p95 ${times[(times.size - 1) * 95 / 100] / 1_000_000.0} ms")
            sendTimes.forEachIndexed { kind, samples ->
                val sorted = samples.sorted()
                if (sorted.isNotEmpty()) android.util.Log.i("SigilAcceptance", "Media kind=$kind frames=${sorted.size} p95=${sorted[(sorted.size - 1) * 95 / 100] / 1_000_000.0} ms")
            }
            NativeStorage.closeCall(handle)
            assertEquals(-1, NativeStorage.callState(handle)); assertNull(NativeStorage.receiveCallFrames(handle))
            assertFalse(NativeStorage.sendCallFrame(handle, 0, 0, false, byteArrayOf(1)))
        } finally { NativeStorage.closeCall(handle); key.fill(0); videos.forEach { it.close() }; Thread.sleep(100); readers.forEach { it.close() }; thread.quitSafely(); directory.deleteRecursively() }
    }
}
