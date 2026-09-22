package org.sigil.compose

import android.graphics.ImageFormat
import android.media.ImageReader
import android.opengl.*
import android.os.Handler
import android.os.HandlerThread
import android.view.Surface
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.*
import org.junit.Assert.*
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

class CallVideoTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    @Test fun blockedVideoSendDropsDependenciesAndRecoversAtKeyframe() {
        val entered = CountDownLatch(1); val release = CountDownLatch(1); val delivered = CountDownLatch(1)
        val requested = AtomicInteger()
        val stamps = java.util.Collections.synchronizedList(mutableListOf<Long>())
        val failures = AtomicInteger()
        val sender = CallVideoSender({ stamp, _, bytes ->
            if (stamp == 0L) { entered.countDown(); assertTrue(release.await(5, TimeUnit.SECONDS)) }
            assertEquals(7, bytes[0].toInt())
            stamps.add(stamp)
            if (stamp == 9L) delivered.countDown()
        }, { requested.incrementAndGet() }, { failures.incrementAndGet() })
        try {
            val bytes = byteArrayOf(7)
            sender.offer(0, true, bytes)
            assertTrue(entered.await(2, TimeUnit.SECONDS))
            for (stamp in 1L..8L) sender.offer(stamp, false, bytes)
            assertEquals(1, requested.get())
            release.countDown()
            sender.offer(9, true, bytes)
            bytes.fill(0)
            assertTrue(delivered.await(2, TimeUnit.SECONDS))
            assertEquals(listOf(0L, 9L), stamps.toList())
            assertEquals(0, failures.get())
        } finally { release.countDown(); sender.close() }
    }
    @Test fun shortSendPausePreservesTheReferenceChain() {
        val entered = CountDownLatch(1); val release = CountDownLatch(1); val delivered = CountDownLatch(6)
        val requested = AtomicInteger(); val failures = AtomicInteger()
        val stamps = java.util.Collections.synchronizedList(mutableListOf<Long>())
        val sender = CallVideoSender({ stamp, _, _ ->
            if (stamp == 0L) { entered.countDown(); assertTrue(release.await(2, TimeUnit.SECONDS)) }
            stamps.add(stamp); delivered.countDown()
        }, { requested.incrementAndGet() }, { failures.incrementAndGet() })
        try {
            sender.offer(0, true, byteArrayOf(7))
            assertTrue(entered.await(2, TimeUnit.SECONDS))
            for (stamp in 1L..5L) sender.offer(stamp, false, byteArrayOf(7))
            release.countDown()
            assertTrue(delivered.await(2, TimeUnit.SECONDS))
            assertEquals((0L..5L).toList(), stamps.toList())
            assertEquals(0, requested.get()); assertEquals(0, failures.get())
        } finally { release.countDown(); sender.close() }
    }
    @Test fun rotationPreservesAspectAndFitsInsideTheView() {
        for ((width, height) in listOf(640 to 480, 1280 to 720)) {
            for ((viewWidth, viewHeight) in listOf(400 to 800, 800 to 400, 120 to 120)) {
                for (rotation in listOf(0, 90, 180, 270)) {
                    val bounds = android.graphics.RectF(0f, 0f, viewWidth.toFloat(), viewHeight.toFloat())
                    callVideoTransform(width, height, rotation, viewWidth, viewHeight).mapRect(bounds)
                    val expected = if (rotation % 180 == 0) width.toFloat() / height else height.toFloat() / width
                    assertEquals("$rotation degrees in $viewWidth x $viewHeight", expected, bounds.width() / bounds.height(), .001f)
                    assertTrue(bounds.left >= -.01f && bounds.top >= -.01f && bounds.right <= viewWidth + .01f && bounds.bottom <= viewHeight + .01f)
                }
            }
        }
    }
    @Test fun cameraProducesLiveAv1AndStopsAfterClose() {
        val instrument = InstrumentationRegistry.getInstrumentation()
        instrument.uiAutomation.grantRuntimePermission(instrument.targetContext.packageName, android.Manifest.permission.CAMERA)
        val frames = AtomicInteger(); val failures = AtomicInteger(); val ready = CountDownLatch(12)
        val camera = CallCamera(instrument.targetContext, true, { _, _, bytes -> assertTrue(bytes.size > 6); frames.incrementAndGet(); ready.countDown() }, { failures.incrementAndGet() })
        try { assertTrue("Camera produced ${frames.get()} frames; ${failures.get()} failures", ready.await(15, TimeUnit.SECONDS)); assertEquals(0, failures.get()) }
        finally { camera.close() }
        Thread.sleep(400); val stopped = frames.get(); Thread.sleep(400); assertEquals(stopped, frames.get())
    }
    @Test fun cameraPreviewRunsAlongsideEncoding() {
        val instrument = InstrumentationRegistry.getInstrumentation()
        instrument.uiAutomation.grantRuntimePermission(instrument.targetContext.packageName, android.Manifest.permission.CAMERA)
        val ready = CountDownLatch(1); val seen = CountDownLatch(60); val encoded = AtomicInteger(); val failures = AtomicInteger()
        var preview: CallCameraPreview? = null
        var dimensions = ""
        ui.setContent {
            androidx.compose.ui.viewinterop.AndroidView(factory = { context ->
                android.view.TextureView(context).apply {
                    surfaceTextureListener = object : android.view.TextureView.SurfaceTextureListener {
                        override fun onSurfaceTextureAvailable(texture: android.graphics.SurfaceTexture, w: Int, h: Int) {
                            preview = CallCameraPreview(texture) { width, height, rotation ->
                                assertTrue(width > 0 && height > 0)
                                assertTrue(rotation in listOf(0, 90, 180, 270))
                                dimensions = "Dims: $width x $height,"
                                // Layout used to replace the selected camera buffer with widget pixels.
                                ui.runOnUiThread { layout(0, 0, 352, 288) }
                            }
                            ready.countDown()
                        }
                        override fun onSurfaceTextureUpdated(texture: android.graphics.SurfaceTexture) { seen.countDown() }
                        override fun onSurfaceTextureSizeChanged(texture: android.graphics.SurfaceTexture, w: Int, h: Int) { preview?.restoreSize() }
                        override fun onSurfaceTextureDestroyed(texture: android.graphics.SurfaceTexture) = true
                    }
                }
            })
        }
        assertTrue(ready.await(5, TimeUnit.SECONDS))
        val camera = CallCamera(instrument.targetContext, true, { _, _, _ -> encoded.incrementAndGet() }, { failures.incrementAndGet() }, preview)
        try {
            assertTrue("Direct camera preview did not update", seen.await(10, TimeUnit.SECONDS))
            assertTrue("Encoding stopped while preview ran", encoded.get() >= 30)
            assertEquals(0, failures.get())
            val dump = android.os.ParcelFileDescriptor.AutoCloseInputStream(
                instrument.uiAutomation.executeShellCommand("dumpsys media.camera")
            ).bufferedReader().use { it.readText() }
            val previewStream = Regex("Consumer name: SurfaceTexture[^\\n]*[\\s\\S]*?Dims: [^\\n]*").find(dump)?.value
            assertTrue("Camera preview buffer changed after layout", previewStream?.contains(dimensions) == true)
        } finally { camera.close(); Thread.sleep(400); preview?.close() }
    }
    @Test fun surfaceAv1RoundTripRendersSyntheticFrames() {
        videoRoundTrip(false)
    }
    @Test fun resolutionRampKeepsRenderingSyntheticFrames() {
        val thread = HandlerThread("Resolution ramp acceptance").apply { start() }
        val reader = ImageReader.newInstance(1920, 1080, ImageFormat.YUV_420_888, 4)
        val decoded = AtomicInteger(); val failures = AtomicInteger()
        val shapes = java.util.Collections.synchronizedSet(mutableSetOf<Pair<Int, Int>>())
        reader.setOnImageAvailableListener({ source -> source.acquireLatestImage()?.use { image ->
            shapes.add(image.width to image.height); decoded.incrementAndGet()
        } }, Handler(thread.looper))
        val decoder = CallVideoDecoder(reader.surface, false) { _, _, _ -> }
        try {
            var stamp = 0
            for ((width, height) in listOf(640 to 360, 1280 to 720, 1920 to 1080, 640 to 360)) {
                val before = decoded.get()
                CallEncoder(width, height, 0, 30, { time, key, bytes -> decoder.offer(time, key, bytes) }, { failures.incrementAndGet() }).use { encoder ->
                    DrawSurface(encoder.surface).use { draw -> repeat(36) { draw.frame(stamp++, 30); Thread.sleep(34) } }
                    val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(3)
                    while (decoded.get() < before + 25 && System.nanoTime() < deadline) Thread.sleep(10)
                    assertTrue("Resolution $width x $height stalled: ${decoded.get() - before} frames", decoded.get() >= before + 25)
                }
            }
            assertTrue(shapes.containsAll(listOf(640 to 360, 1280 to 720, 1920 to 1080)))
            assertEquals(0, failures.get())
        } finally { decoder.close(); Thread.sleep(100); reader.close(); thread.quitSafely() }
    }
    @Test fun aSingleFrameRendersWithoutWaitingForAnotherInput() {
        val thread = HandlerThread("Single frame acceptance").apply { start() }
        val ready = CountDownLatch(1)
        val reader = ImageReader.newInstance(640, 480, ImageFormat.YUV_420_888, 3)
        reader.setOnImageAvailableListener({ source -> source.acquireLatestImage()?.use { ready.countDown() } }, Handler(thread.looper))
        val decoder = CallVideoDecoder(reader.surface, false) { _, _, _ -> }
        try {
            CallEncoder(640, 480, 0, 30, { time, keyframe, bytes -> decoder.offer(time, keyframe, bytes) }, { throw it }).use { encoder ->
                DrawSurface(encoder.surface).use { draw ->
                    draw.frame(0)
                    assertTrue("Final video frame remained stuck in the decoder", ready.await(5, TimeUnit.SECONDS))
                }
            }
        } finally { decoder.close(); Thread.sleep(100); reader.close(); thread.quitSafely() }
    }
    @Test fun hardwareAv1Sustains1080p60() { hardwareRoundTrip(1920, 1080) }
    @Test fun hardwareAv1Sustains4k60() { hardwareRoundTrip(3840, 2160) }
    private fun hardwareRoundTrip(width: Int, height: Int) {
        val mime = android.media.MediaFormat.MIMETYPE_VIDEO_AV1
        val encoderInfo = requireNotNull(callVideoEncoder(mime))
        assertTrue("Hardware AV1 encoder required for this acceptance target", encoderInfo.isHardwareAccelerated)
        assertTrue(encoderInfo.getCapabilitiesForType(mime).videoCapabilities.areSizeAndRateSupported(width, height, 60.0))
        val thread = HandlerThread("60 fps acceptance").apply { start() }
        val decoded = AtomicInteger(); val encoded = AtomicInteger(); val failures = AtomicInteger()
        val reader = ImageReader.newInstance(width, height, ImageFormat.YUV_420_888, 4)
        reader.setOnImageAvailableListener({ source -> source.acquireNextImage()?.use { decoded.incrementAndGet() } }, Handler(thread.looper))
        val decoder = CallVideoDecoder(reader.surface, false) { _, _, _ -> }
        try {
            CallEncoder(width, height, 0, 60, { timestamp, keyframe, bytes -> encoded.incrementAndGet(); decoder.offer(timestamp, keyframe, bytes) }, { failures.incrementAndGet() }).use { encoder ->
                DrawSurface(encoder.surface).use { draw ->
                    val start = System.nanoTime()
                    repeat(600) { index ->
                        val remaining = start + index * 1_000_000_000L / 60 - System.nanoTime()
                        if (remaining > 0) TimeUnit.NANOSECONDS.sleep(remaining)
                        draw.frame(index, 60)
                    }
                    val elapsed = (System.nanoTime() - start) / 1_000_000
                    Thread.sleep(1000)
                    android.util.Log.i("SigilTiming", "acceptance AV1 ${width}x${height}@60 source_ms=$elapsed encoded=${encoded.get()} decoded=${decoded.get()} failures=${failures.get()}")
                    assertTrue("Capture could not sustain 60 fps: $elapsed ms", elapsed < 11000)
                    assertTrue("Only ${encoded.get()} encoded frames", encoded.get() >= 590)
                    assertTrue("Only ${decoded.get()} decoded frames", decoded.get() >= 570)
                    assertEquals(0, failures.get())
                }
            }
        } finally { decoder.close(); Thread.sleep(100); reader.close(); thread.quitSafely() }
    }
    @Test fun invalidFrameDoesNotPreventLaterVideoFromRendering() {
        videoRoundTrip(true)
    }
    private fun videoRoundTrip(damaged: Boolean) {
        val thread = HandlerThread("Codec acceptance").apply { start() }
        val decoded = CountDownLatch(10); val failures = AtomicInteger(); val encoded = AtomicInteger()
        val reader = ImageReader.newInstance(640, 480, ImageFormat.YUV_420_888, 3)
        reader.setOnImageAvailableListener({ source -> source.acquireLatestImage()?.use { decoded.countDown() } }, Handler(thread.looper))
        val decoder = CallVideoDecoder(reader.surface, false) { width, height, rotation -> assertEquals(640, width); assertEquals(480, height); assertEquals(90, rotation) }
        try {
            if (damaged) { decoder.offer(0, true, byteArrayOf(0)); Thread.sleep(100) }
            CallEncoder(640, 480, 90, 30, { timestamp, keyframe, bytes -> encoded.incrementAndGet(); decoder.offer(timestamp, keyframe, bytes) }, { failures.incrementAndGet() }).use { encoder ->
                DrawSurface(encoder.surface).use { draw -> repeat(48) { draw.frame(it); Thread.sleep(45) } }
                assertTrue("Only ${encoded.get()} encoded frames; ${failures.get()} failures", encoded.get() >= 20)
                assertTrue("Decoder did not render frames", decoded.await(5, TimeUnit.SECONDS))
                assertEquals(0, failures.get())
            }
        } finally { decoder.close(); Thread.sleep(100); reader.close(); thread.quitSafely() }
    }
}
internal class DrawSurface(surface: Surface) : AutoCloseable {
    private val display = EGL14.eglGetDisplay(EGL14.EGL_DEFAULT_DISPLAY)
    private val context: android.opengl.EGLContext
    private val target: android.opengl.EGLSurface
    init {
        check(EGL14.eglInitialize(display, IntArray(2), 0, IntArray(2), 0))
        val configs = arrayOfNulls<android.opengl.EGLConfig>(1)
        check(EGL14.eglChooseConfig(display, intArrayOf(EGL14.EGL_RED_SIZE, 8, EGL14.EGL_GREEN_SIZE, 8, EGL14.EGL_BLUE_SIZE, 8, EGL14.EGL_RENDERABLE_TYPE, EGL14.EGL_OPENGL_ES2_BIT, 0x3142, 1, EGL14.EGL_NONE), 0, configs, 0, 1, IntArray(1), 0))
        context = EGL14.eglCreateContext(display, configs[0], EGL14.EGL_NO_CONTEXT, intArrayOf(EGL14.EGL_CONTEXT_CLIENT_VERSION, 2, EGL14.EGL_NONE), 0)
        target = EGL14.eglCreateWindowSurface(display, configs[0], surface, intArrayOf(EGL14.EGL_NONE), 0)
        check(EGL14.eglMakeCurrent(display, target, target, context))
    }
    fun frame(index: Int, fps: Int = 25) { GLES20.glClearColor((index % 3) / 2f, .3f, .7f, 1f); GLES20.glClear(GLES20.GL_COLOR_BUFFER_BIT); EGLExt.eglPresentationTimeANDROID(display, target, index * 1_000_000_000L / fps); check(EGL14.eglSwapBuffers(display, target)) }
    override fun close() { EGL14.eglMakeCurrent(display, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_CONTEXT); EGL14.eglDestroySurface(display, target); EGL14.eglDestroyContext(display, context); EGL14.eglReleaseThread(); EGL14.eglTerminate(display) }
}
