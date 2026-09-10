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
    @Test fun cameraProducesLiveVp8AndStopsAfterClose() {
        val instrument = InstrumentationRegistry.getInstrumentation()
        instrument.uiAutomation.grantRuntimePermission(instrument.targetContext.packageName, android.Manifest.permission.CAMERA)
        val frames = AtomicInteger(); val failures = AtomicInteger(); val ready = CountDownLatch(12)
        val camera = CallCamera(instrument.targetContext, true, { _, _, bytes -> assertTrue(bytes.size > 6); frames.incrementAndGet(); ready.countDown() }, { failures.incrementAndGet() })
        try { assertTrue("Camera produced ${frames.get()} frames; ${failures.get()} failures", ready.await(15, TimeUnit.SECONDS)); assertEquals(0, failures.get()) }
        finally { camera.close() }
        Thread.sleep(400); val stopped = frames.get(); Thread.sleep(400); assertEquals(stopped, frames.get())
    }
    @Test fun surfaceVp8RoundTripRendersSyntheticFrames() {
        videoRoundTrip(false)
    }
    @Test fun aSingleFrameRendersWithoutWaitingForAnotherInput() {
        val thread = HandlerThread("Single frame acceptance").apply { start() }
        val ready = CountDownLatch(1)
        val reader = ImageReader.newInstance(640, 480, ImageFormat.YUV_420_888, 3)
        reader.setOnImageAvailableListener({ source -> source.acquireLatestImage()?.use { ready.countDown() } }, Handler(thread.looper))
        val decoder = CallVideoDecoder(reader.surface) { _, _, _ -> }
        try {
            Vp8Encoder(640, 480, 0, { time, keyframe, bytes -> decoder.offer(time, keyframe, bytes) }, { throw it }).use { encoder ->
                DrawSurface(encoder.surface).use { draw ->
                    draw.frame(0)
                    assertTrue("Final video frame remained stuck in the decoder", ready.await(5, TimeUnit.SECONDS))
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
        val decoder = CallVideoDecoder(reader.surface) { width, height, rotation -> assertEquals(640, width); assertEquals(480, height); assertEquals(90, rotation) }
        try {
            if (damaged) { decoder.offer(0, true, byteArrayOf(0)); Thread.sleep(100) }
            Vp8Encoder(640, 480, 90, { timestamp, keyframe, bytes -> encoded.incrementAndGet(); decoder.offer(timestamp, keyframe, bytes) }, { failures.incrementAndGet() }).use { encoder ->
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
    fun frame(index: Int) { GLES20.glClearColor((index % 3) / 2f, .3f, .7f, 1f); GLES20.glClear(GLES20.GL_COLOR_BUFFER_BIT); EGLExt.eglPresentationTimeANDROID(display, target, index * 40_000_000L); check(EGL14.eglSwapBuffers(display, target)) }
    override fun close() { EGL14.eglMakeCurrent(display, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_CONTEXT); EGL14.eglDestroySurface(display, target); EGL14.eglDestroyContext(display, context); EGL14.eglReleaseThread(); EGL14.eglTerminate(display) }
}
