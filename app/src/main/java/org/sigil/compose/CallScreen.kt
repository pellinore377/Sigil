package org.sigil.compose

import android.content.Context
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.projection.MediaProjection
import android.os.*
import java.util.concurrent.atomic.AtomicBoolean

internal class CallScreen(context: Context, private val projection: MediaProjection, private val send: (Long, Boolean, ByteArray) -> Unit, private val ended: (Exception?) -> Unit) : AutoCloseable {
    private val running = AtomicBoolean(true)
    private val thread = HandlerThread("Sigil screen").apply { start() }
    private val handler = Handler(thread.looper)
    private var display: VirtualDisplay? = null
    private var encoder: Vp8Encoder? = null
    private val density = context.resources.displayMetrics.densityDpi
    private var width = 0
    private var height = 0
    private val callback = object : MediaProjection.Callback() {
        override fun onStop() { if (running.get()) { close(); ended(null) } }
        override fun onCapturedContentResize(w: Int, h: Int) { resize(w, h) }
    }
    init {
        projection.registerCallback(callback, handler)
        val metrics = context.resources.displayMetrics
        handler.post { resize(metrics.widthPixels, metrics.heightPixels) }
    }
    private fun resize(w: Int, h: Int) {
        if (!running.get() || w <= 0 || h <= 0) return
        val scale = minOf(1f, 1280f / maxOf(w, h))
        val nextWidth = maxOf(16, (w * scale).toInt() / 2 * 2)
        val nextHeight = maxOf(16, (h * scale).toInt() / 2 * 2)
        if (width == nextWidth && height == nextHeight) return
        try {
            val next = Vp8Encoder(nextWidth, nextHeight, 0, send) { error -> close(); ended(error) }
            try {
                if (display == null) display = projection.createVirtualDisplay("Sigil screen", nextWidth, nextHeight, density, DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, next.surface, null, handler)
                else { display?.surface = null; display?.resize(nextWidth, nextHeight, density); display?.surface = next.surface }
            } catch (error: Exception) { next.close(); throw error }
            encoder?.close(); encoder = next; width = nextWidth; height = nextHeight
        } catch (error: Exception) { close(); ended(error) }
    }
    override fun close() {
        if (!running.getAndSet(false)) return
        handler.post { display?.release(); display = null; encoder?.close(); encoder = null; projection.unregisterCallback(callback); projection.stop(); thread.quitSafely() }
    }
}
