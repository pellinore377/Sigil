package org.sigil.compose

import android.content.Context
import android.graphics.Matrix
import android.hardware.camera2.*
import android.media.*
import android.os.*
import android.util.Size
import android.view.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.viewinterop.AndroidView
import java.nio.ByteBuffer
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

internal class Vp8Encoder(val width: Int, val height: Int, private val rotation: Int, private val send: (Long, Boolean, ByteArray) -> Unit, private val failed: (Exception) -> Unit) : AutoCloseable {
    private val codec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_VP8)
    private val running = AtomicBoolean(true)
    val surface: Surface
    private val worker: Thread
    init {
        try {
            val format = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_VP8, width, height)
            format.setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
            format.setInteger(MediaFormat.KEY_BIT_RATE, 700000)
            format.setInteger(MediaFormat.KEY_FRAME_RATE, 24)
            format.setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 1)
            codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            surface = codec.createInputSurface(); codec.start()
        } catch (error: Exception) { codec.release(); throw error }
        worker = Thread({
            val info = MediaCodec.BufferInfo()
            try {
                while (running.get()) {
                    val index = codec.dequeueOutputBuffer(info, 10000)
                    if (index < 0) continue
                    try {
                        if (info.size > 0 && info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG == 0) {
                            require(info.size <= 1024 * 1024 - 6)
                            val bytes = ByteArray(info.size + 6)
                            try {
                                ByteBuffer.wrap(bytes).putShort(rotation.toShort()).putShort(width.toShort()).putShort(height.toShort())
                                requireNotNull(codec.getOutputBuffer(index)).apply { position(info.offset); limit(info.offset + info.size) }.get(bytes, 6, info.size)
                                send(info.presentationTimeUs.coerceAtLeast(0), info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0, bytes)
                            } finally { bytes.fill(0) }
                        }
                    } finally { codec.releaseOutputBuffer(index, false) }
                }
            } catch (error: Exception) { if (running.get()) failed(error) }
            finally { try { codec.stop() } catch (_: Exception) {}; codec.release(); surface.release() }
        }, "Sigil video encoder").apply { start() }
    }
    override fun close() { running.set(false); worker.interrupt() }
}
internal class CallCamera(context: Context, front: Boolean, send: (Long, Boolean, ByteArray) -> Unit, private val failed: () -> Unit) : AutoCloseable {
    private val running = AtomicBoolean(true)
    private val thread = HandlerThread("Sigil camera").apply { start() }
    private val handler = Handler(thread.looper)
    private var device: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    private var encoder: Vp8Encoder? = null
    init {
        handler.post {
            try {
                val manager = context.getSystemService(CameraManager::class.java)
                val id = manager.cameraIdList.firstOrNull { manager.getCameraCharacteristics(it).get(CameraCharacteristics.LENS_FACING) == if (front) CameraCharacteristics.LENS_FACING_FRONT else CameraCharacteristics.LENS_FACING_BACK } ?: manager.cameraIdList.first()
                val info = manager.getCameraCharacteristics(id)
                val choices = requireNotNull(info.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)).getOutputSizes(MediaCodec::class.java)
                val size = choices.filter { it.width <= 1280 && it.height <= 720 }.minByOrNull { kotlin.math.abs(it.width * it.height - 640 * 480) } ?: error("Unsupported camera size")
                val rotation = info.get(CameraCharacteristics.SENSOR_ORIENTATION) ?: 0
                val video = Vp8Encoder(size.width, size.height, rotation, send) { failed() }
                encoder = video
                manager.openCamera(id, object : CameraDevice.StateCallback() {
                    override fun onOpened(camera: CameraDevice) {
                        if (!running.get()) { camera.close(); return }
                        device = camera
                        camera.createCaptureSession(listOf(video.surface), object : CameraCaptureSession.StateCallback() {
                            override fun onConfigured(value: CameraCaptureSession) {
                                if (!running.get()) { value.close(); return }
                                session = value
                                try {
                                    val request = camera.createCaptureRequest(CameraDevice.TEMPLATE_RECORD).apply {
                                        addTarget(video.surface)
                                        set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO)
                                        info.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES)?.filter { it.upper <= 30 && it.upper >= 24 }?.minByOrNull { it.upper - it.lower }?.let { set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, it) }
                                    }
                                    value.setRepeatingRequest(request.build(), null, handler)
                                } catch (_: Exception) { failed(); close() }
                            }
                            override fun onConfigureFailed(value: CameraCaptureSession) { value.close(); failed(); close() }
                        }, handler)
                    }
                    override fun onDisconnected(camera: CameraDevice) { camera.close(); if (running.get()) failed(); close() }
                    override fun onError(camera: CameraDevice, error: Int) { camera.close(); if (running.get()) failed(); close() }
                }, handler)
            } catch (_: Exception) { if (running.get()) failed(); close() }
        }
    }
    override fun close() {
        if (!running.getAndSet(false)) return
        handler.post { try { session?.stopRepeating() } catch (_: Exception) {}; session?.close(); device?.close(); encoder?.close(); thread.quitSafely() }
    }
}
private data class VideoPacket(val timestamp: Long, val keyframe: Boolean, val bytes: ByteArray)
internal fun callVideoTransform(width: Int, height: Int, rotation: Int, viewWidth: Int, viewHeight: Int): Matrix {
    val sideways = rotation == 90 || rotation == 270
    val aspect = if (sideways) height.toFloat() / width else width.toFloat() / height
    val display = viewWidth.toFloat() / viewHeight
    return Matrix().apply {
        setRotate(rotation.toFloat(), viewWidth / 2f, viewHeight / 2f)
        if (sideways) postScale(viewWidth.toFloat() / viewHeight, viewHeight.toFloat() / viewWidth, viewWidth / 2f, viewHeight / 2f)
        postScale(if (aspect < display) aspect / display else 1f, if (aspect > display) display / aspect else 1f, viewWidth / 2f, viewHeight / 2f)
    }
}
internal class CallVideoDecoder(private val surface: Surface, private val geometry: (Int, Int, Int) -> Unit) : AutoCloseable {
    private val running = AtomicBoolean(true)
    private val queue = ArrayBlockingQueue<VideoPacket>(4)
    private val lostFrame = AtomicBoolean(false)
    private val worker = Thread({
        var codec: MediaCodec? = null
        var dimensions: Size? = null
        var rendered: Triple<Int, Int, Int>? = null
        var lastTimestamp = Long.MIN_VALUE
        fun reset() {
            val previous = codec; codec = null; dimensions = null
            try { previous?.stop() } catch (_: Exception) {}
            try { previous?.release() } catch (_: Exception) {}
        }
        fun drainOutput() {
            val active = codec ?: return
            val info = MediaCodec.BufferInfo()
            while (true) {
                val output = active.dequeueOutputBuffer(info, 0)
                if (output == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) continue
                if (output < 0) break
                active.releaseOutputBuffer(output, running.get())
            }
        }
        try {
            while (running.get()) {
                if (lostFrame.getAndSet(false)) reset()
                try { drainOutput() } catch (_: Exception) { reset() }
                val packet = queue.poll(10, TimeUnit.MILLISECONDS) ?: continue
                try {
                    if (lostFrame.getAndSet(false)) reset()
                    if (packet.timestamp <= lastTimestamp) { reset(); continue }
                    val bytes = packet.bytes
                    require(bytes.size > 6)
                    val buffer = ByteBuffer.wrap(bytes)
                    val rotation = buffer.short.toInt() and 65535; val width = buffer.short.toInt() and 65535; val height = buffer.short.toInt() and 65535
                    require(rotation in listOf(0, 90, 180, 270) && width in 16..1920 && height in 16..1920 && width * height <= 1920 * 1080)
                    val keyframe = bytes[6].toInt() and 1 == 0
                    require(keyframe == packet.keyframe)
                    if (keyframe) {
                        require(bytes.size >= 16 && bytes[9] == 0x9d.toByte() && bytes[10] == 1.toByte() && bytes[11] == 0x2a.toByte())
                        val codedWidth = ((bytes[12].toInt() and 255) or ((bytes[13].toInt() and 255) shl 8)) and 0x3fff
                        val codedHeight = ((bytes[14].toInt() and 255) or ((bytes[15].toInt() and 255) shl 8)) and 0x3fff
                        require(codedWidth == width && codedHeight == height)
                    }
                    val size = Size(width, height)
                    if (codec == null || size != dimensions) {
                        if (!keyframe) continue
                        reset()
                        codec = MediaCodec.createDecoderByType(MediaFormat.MIMETYPE_VIDEO_VP8)
                        codec!!.configure(MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_VP8, width, height), surface, null, 0); codec!!.start(); dimensions = size
                    }
                    val shape = Triple(width, height, rotation)
                    if (rendered != shape) { geometry(width, height, rotation); rendered = shape }
                    val active = requireNotNull(codec)
                    var input = active.dequeueInputBuffer(10000)
                    var attempts = 0
                    while (input < 0 && running.get() && attempts++ < 4) {
                        drainOutput()
                        input = active.dequeueInputBuffer(10000)
                    }
                    if (input >= 0) {
                        val target = requireNotNull(active.getInputBuffer(input)); target.clear(); require(bytes.size - 6 <= target.remaining()); target.put(bytes, 6, bytes.size - 6)
                        active.queueInputBuffer(input, 0, bytes.size - 6, packet.timestamp, 0)
                        lastTimestamp = packet.timestamp
                    } else {
                        reset()
                    }
                    drainOutput()
                } catch (interrupted: InterruptedException) { throw interrupted }
                catch (_: Exception) { reset() }
                finally { packet.bytes.fill(0) }
            }
        } catch (_: InterruptedException) { }
        catch (_: Exception) { }
        finally { synchronized(queue) { running.set(false); drain() }; reset(); surface.release() }
    }, "Sigil video decoder").apply { start() }
    fun offer(timestamp: Long, keyframe: Boolean, bytes: ByteArray) = synchronized(queue) {
        if (!running.get()) return
        val packet = VideoPacket(timestamp, keyframe, bytes.copyOf())
        if (!queue.offer(packet)) {
            drain()
            lostFrame.set(true)
            if (!keyframe || !queue.offer(packet)) packet.bytes.fill(0)
        }
    }
    private fun drain() { while (true) { val value = queue.poll() ?: break; value.bytes.fill(0) } }
    override fun close() { synchronized(queue) { running.set(false); drain() }; worker.interrupt() }
}
@Composable
internal fun CallVideoView(calls: NativeCalls, member: String, screen: Boolean, modifier: Modifier) {
    var decoder by remember(member, screen) { mutableStateOf<CallVideoDecoder?>(null) }
    DisposableEffect(member, screen) { onDispose { calls.videoOutput(member, screen, null); decoder?.close(); decoder = null } }
    AndroidView(factory = { context -> TextureView(context).apply {
        surfaceTextureListener = object : TextureView.SurfaceTextureListener {
            private var geometry: Triple<Int, Int, Int>? = null
            private fun resize() {
                val (w, h, rotation) = geometry ?: return
                val target = this@apply
                if (target.width <= 0 || target.height <= 0) return
                target.setTransform(callVideoTransform(w, h, rotation, target.width, target.height))
            }
            override fun onSurfaceTextureAvailable(texture: android.graphics.SurfaceTexture, width: Int, height: Int) {
                val target = this@apply
                decoder = CallVideoDecoder(Surface(texture)) { w, h, rotation -> target.post {
                    if (target.surfaceTexture === texture) { geometry = Triple(w, h, rotation); resize() }
                } }
                calls.videoOutput(member, screen, decoder)
            }
            override fun onSurfaceTextureSizeChanged(texture: android.graphics.SurfaceTexture, width: Int, height: Int) { resize() }
            override fun onSurfaceTextureDestroyed(texture: android.graphics.SurfaceTexture): Boolean { calls.videoOutput(member, screen, null); decoder?.close(); decoder = null; return true }
            override fun onSurfaceTextureUpdated(texture: android.graphics.SurfaceTexture) {}
        }
    } }, modifier = modifier)
}
