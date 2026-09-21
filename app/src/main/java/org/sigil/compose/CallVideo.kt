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

/// Wire codec identifiers carried in every frame header, so a sender encodes what it can and
/// each receiver decodes per sender without any negotiation.
internal const val CODEC_VP9: Byte = 1
internal const val CODEC_AV1: Byte = 2
/// Codec, rotation, width and height, each big endian, ahead of the encoded frame.
internal const val VIDEO_HEADER = 7
internal fun callVideoMime(codec: Byte) = when (codec) {
    CODEC_VP9 -> MediaFormat.MIMETYPE_VIDEO_VP9
    CODEC_AV1 -> MediaFormat.MIMETYPE_VIDEO_AV1
    else -> null
}
/// The decoder to use for a type: a hardware one if the platform offers it.
internal fun callVideoDecoder(mime: String): MediaCodecInfo? {
    val all = runCatching { MediaCodecList(MediaCodecList.ALL_CODECS).codecInfos }.getOrDefault(emptyArray())
        .filter { !it.isEncoder && it.supportedTypes.any { type -> type.equals(mime, true) } }
    return all.firstOrNull { it.isHardwareAccelerated } ?: all.firstOrNull()
}
/// Every codec the platform will admit for a type, in both list modes, with the facts that decide
/// selection. Logged once per process so a device's real capability is never inferred from a spec sheet.
internal fun callVideoCodecs(mime: String) {
    if (!codecsLogged.add(mime)) return
    for (mode in listOf(MediaCodecList.ALL_CODECS to "all", MediaCodecList.REGULAR_CODECS to "regular")) {
        for (info in runCatching { MediaCodecList(mode.first).codecInfos }.getOrDefault(emptyArray())) {
            if (info.supportedTypes.none { it.equals(mime, true) }) continue
            val video = runCatching { info.getCapabilitiesForType(mime).videoCapabilities }.getOrNull()
            val best = runCatching { video?.let { "${it.supportedWidths.upper}x${it.supportedHeights.upper}@${it.getSupportedFrameRatesFor(1920, 1080).upper.toInt()}" } }.getOrNull()
            android.util.Log.i("SigilTiming", "codec ${mode.second} ${info.name} encoder=${info.isEncoder} hardware=${info.isHardwareAccelerated} softwareOnly=${info.isSoftwareOnly} vendor=${info.isVendor} max=$best")
        }
    }
}
private val codecsLogged = java.util.Collections.synchronizedSet(mutableSetOf<String>())
/// The encoder to use for a type: a hardware one if the platform offers it, whatever exists otherwise.
internal fun callVideoEncoder(mime: String): MediaCodecInfo? {
    val all = runCatching { MediaCodecList(MediaCodecList.ALL_CODECS).codecInfos }.getOrDefault(emptyArray())
        .filter { it.isEncoder && it.supportedTypes.any { type -> type.equals(mime, true) } }
    return all.firstOrNull { it.isHardwareAccelerated } ?: all.firstOrNull()
}
/// AV1 camera target; encrypted frames travel on the native RTP video track.
internal const val CALL_VIDEO_WIDTH = 1920
internal const val CALL_VIDEO_HEIGHT = 1080
internal const val CALL_VIDEO_FPS = 60
/// Preserve the per-frame bitrate budget when capture rate changes.
internal fun callVideoBitrate(width: Int, height: Int, fps: Int): Int {
    val pixels = width * height
    val base = when {
        pixels >= 3840 * 2160 -> 14_000_000
        pixels >= 1920 * 1080 -> 3_500_000
        pixels >= 1280 * 720 -> 2_000_000
        pixels >= 960 * 540 -> 1_200_000
        else -> 600_000
    }
    return (base.toLong() * fps.coerceIn(15, 60) / 30).toInt()
}
internal class CallEncoder(val width: Int, val height: Int, private val rotation: Int, private val fps: Int, private val send: (Long, Boolean, ByteArray) -> Unit, private val failed: (Exception) -> Unit) : AutoCloseable {
    private val wire = CODEC_AV1
    private val mime = requireNotNull(callVideoMime(wire))
    private val codec = callVideoEncoder(mime)
        ?.let { android.util.Log.i("SigilTiming", "codec encode ${it.name} hardware=${it.isHardwareAccelerated}"); MediaCodec.createByCodecName(it.name) }
        ?: MediaCodec.createEncoderByType(mime)
    private val running = AtomicBoolean(true)
    private val keyframeRequested = AtomicBoolean(false)
    fun requestKeyframe() { keyframeRequested.set(true) }
    val surface: Surface
    private val worker: Thread
    init {
        try {
            val format = MediaFormat.createVideoFormat(mime, width, height)
            format.setInteger(MediaFormat.KEY_PRIORITY, 0)
            format.setInteger(MediaFormat.KEY_OPERATING_RATE, fps)
            format.setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
            format.setInteger(MediaFormat.KEY_BIT_RATE, callVideoBitrate(width, height, fps))
            format.setInteger(MediaFormat.KEY_FRAME_RATE, fps)
            // One-second keyframes bound how long a lost frame can hold the picture without spending the bitrate on them.
            format.setFloat(MediaFormat.KEY_I_FRAME_INTERVAL, 1f)
            codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            surface = codec.createInputSurface(); codec.start()
        } catch (error: Exception) { codec.release(); throw error }
        worker = Thread({
            val info = MediaCodec.BufferInfo()
            var counted = 0L; var since = android.os.SystemClock.elapsedRealtime()
            try {
                while (running.get()) {
                    if (keyframeRequested.getAndSet(false)) codec.setParameters(Bundle().apply { putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0) })
                    val index = codec.dequeueOutputBuffer(info, 10000)
                    if (index < 0) continue
                    try {
                        if (info.size > 0 && info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG == 0) {
                            require(info.size <= 1024 * 1024 - VIDEO_HEADER)
                            val bytes = ByteArray(info.size + VIDEO_HEADER)
                            try {
                                ByteBuffer.wrap(bytes).put(wire).putShort(rotation.toShort()).putShort(width.toShort()).putShort(height.toShort())
                                requireNotNull(codec.getOutputBuffer(index)).apply { position(info.offset); limit(info.offset + info.size) }.get(bytes, VIDEO_HEADER, info.size)
                                send(info.presentationTimeUs.coerceAtLeast(0), info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0, bytes)
                                counted++
                                val at = android.os.SystemClock.elapsedRealtime()
                                if (at - since >= 5000) { android.util.Log.i("SigilTiming", "video out fps=${counted * 1000 / (at - since)}"); counted = 0; since = at }
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
// Queue only a short interval; after a gap, resume with an independent frame.
internal class CallVideoSender(private val send: (Long, Boolean, ByteArray) -> Unit, private val requestKeyframe: () -> Unit, private val failed: (Exception) -> Unit) : AutoCloseable {
    private data class Pending(val stamp: Long, val key: Boolean, val bytes: ByteArray, val added: Long)
    private val queue = ArrayBlockingQueue<Pending>(3)
    private val running = AtomicBoolean(true)
    private var waitingKey = true
    private fun clear() { while (true) (queue.poll() ?: break).bytes.fill(0) }
    @Synchronized fun offer(timestamp: Long, keyframe: Boolean, bytes: ByteArray) {
        if (!running.get()) return
        if (waitingKey && !keyframe) return
        if (queue.remainingCapacity() == 0) {
            clear(); waitingKey = true; requestKeyframe()
        }
        if (waitingKey && !keyframe) return
        waitingKey = false
        queue.add(Pending(timestamp, keyframe, bytes.copyOf(), System.nanoTime()))
    }
    private val worker = Thread({
        try {
            while (running.get()) {
                val packet = queue.poll(20, TimeUnit.MILLISECONDS) ?: continue
                try {
                    if (!running.get()) continue
                    if (System.nanoTime() - packet.added > 100_000_000L) {
                        synchronized(this) { clear(); waitingKey = true; requestKeyframe() }
                        continue
                    }
                    send(packet.stamp, packet.key, packet.bytes)
                } finally { packet.bytes.fill(0) }
            }
        } catch (error: Exception) { if (running.get()) failed(error) }
        finally { synchronized(this) { clear() } }
    }, "Sigil video sender").apply { start() }
    @Synchronized override fun close() { running.set(false); clear(); worker.interrupt() }
}
internal class CallCamera(context: Context, front: Boolean, send: (Long, Boolean, ByteArray) -> Unit, private val failed: () -> Unit, preview: (Long, Boolean, ByteArray) -> Unit = { _, _, _ -> }) : AutoCloseable {
    private val running = AtomicBoolean(true)
    private val thread = HandlerThread("Sigil camera").apply { start() }
    private val handler = Handler(thread.looper)
    private var device: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    @Volatile private var encoder: CallEncoder? = null
    private val outgoing = CallVideoSender(send, { encoder?.requestKeyframe() }, { error ->
        android.util.Log.e("SigilTiming", "camera send failed", error)
        if (running.get()) failed()
        close()
    })
    init {
        handler.post {
            try {
                callVideoCodecs(MediaFormat.MIMETYPE_VIDEO_VP9)
                callVideoCodecs(MediaFormat.MIMETYPE_VIDEO_AV1)
                val manager = context.getSystemService(CameraManager::class.java)
                val id = manager.cameraIdList.firstOrNull { manager.getCameraCharacteristics(it).get(CameraCharacteristics.LENS_FACING) == if (front) CameraCharacteristics.LENS_FACING_FRONT else CameraCharacteristics.LENS_FACING_BACK } ?: manager.cameraIdList.first()
                val info = manager.getCameraCharacteristics(id)
                val streams = requireNotNull(info.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP))
                val choices = streams.getOutputSizes(MediaCodec::class.java)
                    .filter { it.width <= CALL_VIDEO_WIDTH && it.height <= CALL_VIDEO_HEIGHT }
                    .sortedByDescending { it.width * it.height }
                val encoderInfo = requireNotNull(callVideoEncoder(MediaFormat.MIMETYPE_VIDEO_AV1))
                val capabilities = encoderInfo.getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_AV1).videoCapabilities
                val ceiling = if (encoderInfo.isHardwareAccelerated) CALL_VIDEO_FPS else 30
                val rates = info.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES).orEmpty()
                    .filter { it.upper in 15..ceiling }.sortedWith(compareByDescending<android.util.Range<Int>> { it.upper }.thenByDescending { it.lower })
                val selected = choices.firstNotNullOfOrNull { size ->
                    val duration = streams.getOutputMinFrameDuration(MediaCodec::class.java, size)
                    rates.firstOrNull { rate ->
                        (duration == 0L || duration <= 1_000_000_000L / rate.upper + 1000L) &&
                            capabilities.areSizeAndRateSupported(size.width, size.height, rate.upper.toDouble())
                    }?.let { size to it }
                } ?: error("No supported camera and AV1 encoder mode")
                val (size, range) = selected
                val fps = range.upper
                val rotation = info.get(CameraCharacteristics.SENSOR_ORIENTATION) ?: 0
                android.util.Log.i("SigilTiming", "video out ${size.width}x${size.height}@$fps ${callVideoBitrate(size.width, size.height, fps) / 1000}kbps")
                val video = CallEncoder(size.width, size.height, rotation, fps, { timestamp, keyframe, bytes ->
                    preview(timestamp, keyframe, bytes)
                    outgoing.offer(timestamp, keyframe, bytes)
                }) { error ->
                    android.util.Log.e("SigilTiming", "camera encoder failed", error)
                    if (running.get()) failed()
                    close()
                }
                encoder = video
                // The preview is this stream decoded back, as a screen share already is. A second
                // camera target must be a size the camera can deliver, and the camera crops the
                // upright picture into that landscape buffer, losing its top and bottom.
                val targets = listOf(video.surface)
                manager.openCamera(id, object : CameraDevice.StateCallback() {
                    override fun onOpened(camera: CameraDevice) {
                        if (!running.get()) { camera.close(); return }
                        device = camera
                        camera.createCaptureSession(targets, object : CameraCaptureSession.StateCallback() {
                            override fun onConfigured(value: CameraCaptureSession) {
                                if (!running.get()) { value.close(); return }
                                session = value
                                try {
                                    val request = camera.createCaptureRequest(CameraDevice.TEMPLATE_RECORD).apply {
                                        targets.forEach { addTarget(it) }
                                        set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO)
                                        range?.let { set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, it) }
                                    }
                                    value.setRepeatingRequest(request.build(), null, handler)
                                } catch (_: Exception) { failed(); close() }
                            }
                            override fun onConfigureFailed(value: CameraCaptureSession) { value.close(); failed(); close() }
                        }, handler)
                    }
                    override fun onDisconnected(camera: CameraDevice) { android.util.Log.e("SigilTiming", "camera disconnected"); camera.close(); if (running.get()) failed(); close() }
                    override fun onError(camera: CameraDevice, error: Int) { android.util.Log.e("SigilTiming", "camera device error=$error"); camera.close(); if (running.get()) failed(); close() }
                }, handler)
            } catch (_: Exception) { if (running.get()) failed(); close() }
        }
    }
    override fun close() {
        if (!running.getAndSet(false)) return
        outgoing.close()
        handler.post { try { session?.stopRepeating() } catch (_: Exception) {}; session?.close(); device?.close(); encoder?.close(); thread.quitSafely() }
    }
}
private data class VideoPacket(val timestamp: Long, val keyframe: Boolean, val bytes: ByteArray)
/// How long a decoded frame waits before it is painted, and how far the sender's clock may run
/// from ours before the schedule is taken again from the frame in hand.
private const val LEAD = 30_000_000L
private const val SLIP = 100_000_000L
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
internal class CallVideoDecoder(private val surface: Surface, private val paced: Boolean, private val geometry: (Int, Int, Int) -> Unit) : AutoCloseable {
    private val running = AtomicBoolean(true)
    private val queue = ArrayBlockingQueue<VideoPacket>(8)
    private val lostFrame = AtomicBoolean(false)
    private val worker = Thread({
        var codec: MediaCodec? = null
        var dimensions: Size? = null
        var rendered: Triple<Int, Int, Int>? = null
        var lastTimestamp = Long.MIN_VALUE
        var shown = 0L; var shownSince = android.os.SystemClock.elapsedRealtime(); var decoding: Byte = 0
        var resynced = 0L; var anchor = 0L; var anchorStamp = Long.MIN_VALUE
        fun reset(reason: String = "reconfigure") {
            if (codec != null) android.util.Log.i("SigilTiming", "video decoder reset=$reason queued=${queue.size} paced=$paced")
            anchorStamp = Long.MIN_VALUE
            lastTimestamp = Long.MIN_VALUE
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
                if (!running.get()) { active.releaseOutputBuffer(output, false); continue }
                // Our own camera took no journey, so there is no jitter to smooth and a schedule
                // would only hold the picture back.
                if (!paced) { active.releaseOutputBuffer(output, true); shown++
                    val at = android.os.SystemClock.elapsedRealtime()
                    if (at - shownSince >= 5000) { android.util.Log.i("SigilTiming", "video in ${dimensions?.width}x${dimensions?.height} fps=${shown * 1000 / (at - shownSince)} paced=false"); shown = 0; shownSince = at }
                    continue }
                // Paint on the sender's cadence, not on arrival. A frame reaches here whenever the
                // network let it, so releasing the moment it decodes turns the jitter of its
                // journey into judder however steady the frame rate itself is. The lead is the
                // whole of the smoothing: frames wait that long and no longer.
                val now = System.nanoTime()
                if (anchorStamp == Long.MIN_VALUE) { anchorStamp = info.presentationTimeUs; anchor = now + LEAD }
                var due = anchor + (info.presentationTimeUs - anchorStamp) * 1000
                if (due < now - SLIP || due > now + LEAD + SLIP) {
                    anchorStamp = info.presentationTimeUs; anchor = now + LEAD; due = anchor; resynced++
                }
                active.releaseOutputBuffer(output, due)
                shown++
                val at = android.os.SystemClock.elapsedRealtime()
                if (at - shownSince >= 5000) { android.util.Log.i("SigilTiming", "video in ${dimensions?.width}x${dimensions?.height} fps=${shown * 1000 / (at - shownSince)} resynced=$resynced"); shown = 0; resynced = 0; shownSince = at }
            }
        }
        try {
            while (running.get()) {
                if (lostFrame.getAndSet(false)) reset("queue overflow")
                try { drainOutput() } catch (_: Exception) { reset("output failed") }
                val packet = queue.poll(10, TimeUnit.MILLISECONDS) ?: continue
                try {
                    if (lostFrame.getAndSet(false)) reset("queue overflow")
                    if (packet.timestamp <= lastTimestamp) { reset("timestamp"); continue }
                    val bytes = packet.bytes
                    require(bytes.size > VIDEO_HEADER)
                    val buffer = ByteBuffer.wrap(bytes)
                    val wire = buffer.get(); val mime = requireNotNull(callVideoMime(wire))
                    val rotation = buffer.short.toInt() and 65535; val width = buffer.short.toInt() and 65535; val height = buffer.short.toInt() and 65535
                    require(rotation in listOf(0, 90, 180, 270) && width in 16..3840 && height in 16..3840 && width * height <= 3840 * 2160)
                    // The header travels inside the authenticated encryption, so its
                    // keyframe flag and dimensions need no bitstream cross-check.
                    val keyframe = packet.keyframe
                    val size = Size(width, height)
                    if (codec == null || size != dimensions || wire != decoding) {
                        if (!keyframe) continue
                        reset()
                        val chosen = callVideoDecoder(mime)
                        android.util.Log.i("SigilTiming", "codec decode ${chosen?.name} hardware=${chosen?.isHardwareAccelerated} ${width}x$height")
                        codec = chosen?.let { MediaCodec.createByCodecName(it.name) } ?: MediaCodec.createDecoderByType(mime)
                        val format = MediaFormat.createVideoFormat(mime, width, height).apply {
                            setInteger(MediaFormat.KEY_PRIORITY, 0)
                            setInteger(MediaFormat.KEY_OPERATING_RATE, 60)
                            if (Build.VERSION.SDK_INT >= 30 && chosen?.getCapabilitiesForType(mime)?.isFeatureSupported(MediaCodecInfo.CodecCapabilities.FEATURE_LowLatency) == true) {
                                setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
                            }
                        }
                        codec!!.configure(format, surface, null, 0); codec!!.start(); dimensions = size; decoding = wire
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
                        val target = requireNotNull(active.getInputBuffer(input)); target.clear(); require(bytes.size - VIDEO_HEADER <= target.remaining()); target.put(bytes, VIDEO_HEADER, bytes.size - VIDEO_HEADER)
                        active.queueInputBuffer(input, 0, bytes.size - VIDEO_HEADER, packet.timestamp, 0)
                        lastTimestamp = packet.timestamp
                    } else {
                        reset("input timeout")
                    }
                    drainOutput()
                } catch (interrupted: InterruptedException) { throw interrupted }
                catch (_: Exception) { reset("frame failed") }
                finally { packet.bytes.fill(0) }
            }
        } catch (_: InterruptedException) { }
        catch (_: Exception) { }
        finally { synchronized(queue) { running.set(false); drain() }; reset(); surface.release() }
    }, "Sigil video decoder").apply { start() }
    private var offered = 0L
    private var awaitingKey = false
    fun offer(timestamp: Long, keyframe: Boolean, bytes: ByteArray) = synchronized(queue) {
        if (!running.get()) return
        // RTP sequencing detects loss; sensor timestamp jitter is not a missing encoded frame.
        if (timestamp <= offered && offered > 0L) {
            if (!keyframe) return
            drain(); lostFrame.set(true)
        }
        if (keyframe) awaitingKey = false
        offered = timestamp
        if (awaitingKey) return
        val packet = VideoPacket(timestamp, keyframe, bytes.copyOf())
        if (!queue.offer(packet)) {
            drain()
            lostFrame.set(true)
            awaitingKey = !keyframe
            if (!keyframe || !queue.offer(packet)) packet.bytes.fill(0)
        }
    }
    private fun drain() { while (true) { val value = queue.poll() ?: break; value.bytes.fill(0) } }
    override fun close() { synchronized(queue) { running.set(false); drain() }; worker.interrupt() }
}
@Composable
internal fun CallVideoView(calls: NativeCalls, member: String, screen: Boolean, modifier: Modifier) {
    var decoder by remember(member, screen) { mutableStateOf<CallVideoDecoder?>(null) }
    var aspect by remember(member, screen) { mutableStateOf(16f / 9f) }
    DisposableEffect(member, screen) { onDispose { calls.videoOutput(member, screen, null); decoder?.close(); decoder = null } }
    org.sigil.CallVideoFrame(aspect, member == "self", modifier) { fitted ->
    AndroidView(factory = { context -> TextureView(context).apply {
        isOpaque = false
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
                val measured = { w: Int, h: Int, rotation: Int -> target.post {
                    if (target.surfaceTexture === texture) {
                        aspect = if (rotation % 180 == 0) w.toFloat() / h else h.toFloat() / w
                        geometry = Triple(w, h, rotation); resize()
                    }
                }; Unit }
                decoder = CallVideoDecoder(Surface(texture), member != "self", measured)
                calls.videoOutput(member, screen, decoder)
            }
            override fun onSurfaceTextureSizeChanged(texture: android.graphics.SurfaceTexture, width: Int, height: Int) { resize() }
            override fun onSurfaceTextureDestroyed(texture: android.graphics.SurfaceTexture): Boolean {
                calls.videoOutput(member, screen, null); decoder?.close(); decoder = null
                return true
            }
            override fun onSurfaceTextureUpdated(texture: android.graphics.SurfaceTexture) {}
        }
    } }, update = { it.scaleX = if (member == "self" && !screen && calls.front) -1f else 1f }, modifier = fitted)
    }
}
