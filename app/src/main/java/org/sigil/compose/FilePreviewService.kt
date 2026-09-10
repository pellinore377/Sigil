package org.sigil.compose

import android.app.Service
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Color
import android.graphics.pdf.PdfRenderer
import android.os.*
import java.io.DataOutputStream
import java.nio.ByteBuffer
import java.util.concurrent.*

internal const val PdfPreviewMagic = 0x53475031
internal const val PdfPreviewPixels = 4 * 1024 * 1024

class FilePreviewService : Service() {
    private val main = Handler(Looper.getMainLooper())
    private val executor = ThreadPoolExecutor(1,1,0L,TimeUnit.MILLISECONDS,ArrayBlockingQueue(1))
    private val timeout = Runnable { android.os.Process.killProcess(android.os.Process.myPid()) }
    private val messenger = Messenger(Handler(Looper.getMainLooper()) { message ->
        val input = message.data.getParcelable<ParcelFileDescriptor>("input")
        val output = message.data.getParcelable<ParcelFileDescriptor>("output")
        val index = message.arg1
        val width = message.arg2
        val format=message.data.getString("format")
        val request=message.data.getString("request")
        val pdf=message.what==1 && index in 0..99_999 && width in 32..2048
        val portable=message.what==2 && format!=null && format.length<=32 && request!=null && request.length<=1024
        if ((!pdf && !portable) || input == null || output == null) {
            input?.close(); output?.close()
        } else {
            try { executor.execute {
                main.postDelayed(timeout, 10_000)
                try {
                    input.use { descriptor ->
                        require(descriptor.statSize in 1..128L*1024*1024)
                        if(portable) output.use { require(NativePreview.render(descriptor.fd,it.fd,format!!,request!!)) }
                        else PdfRenderer(descriptor).use { renderer ->
                            require(renderer.pageCount in 1..100_000 && index < renderer.pageCount)
                            renderer.openPage(index).use { page ->
                                require(page.width > 0 && page.height > 0)
                                val scale = minOf(width.toDouble()/page.width, 2048.0/page.height)
                                val w = (page.width*scale).toInt().coerceAtLeast(1)
                                val h = (page.height*scale).toInt().coerceAtLeast(1)
                                require(w.toLong()*h <= PdfPreviewPixels)
                                val bitmap = Bitmap.createBitmap(w,h,Bitmap.Config.ARGB_8888)
                                try {
                                    bitmap.eraseColor(Color.WHITE)
                                    page.render(bitmap,null,null,PdfRenderer.Page.RENDER_MODE_FOR_DISPLAY)
                                    val bytes = ByteArray(w*h*4)
                                    try {
                                        bitmap.copyPixelsToBuffer(ByteBuffer.wrap(bytes))
                                        DataOutputStream(ParcelFileDescriptor.AutoCloseOutputStream(output)).use { stream ->
                                            stream.writeInt(PdfPreviewMagic); stream.writeInt(w); stream.writeInt(h)
                                            stream.writeInt(renderer.pageCount); stream.writeInt(index); stream.write(bytes)
                                        }
                                    } finally { bytes.fill(0) }
                                } finally { bitmap.recycle() }
                            }
                        }
                    }
                } catch (_: Exception) { runCatching { input.close() }; runCatching { output.close() } }
                finally { main.removeCallbacks(timeout) }
            } } catch (_: RejectedExecutionException) { input.close(); output.close() }
        }
        true
    })
    override fun onBind(intent: Intent): IBinder {
        check(android.os.Process.myUid() != applicationInfo.uid)
        return messenger.binder
    }
    override fun onDestroy() {
        executor.shutdownNow()
        super.onDestroy()
        android.os.Process.killProcess(android.os.Process.myPid())
    }
}
