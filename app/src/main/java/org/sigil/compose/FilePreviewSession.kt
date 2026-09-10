package org.sigil.compose

import android.content.*
import android.graphics.Bitmap
import android.os.*
import kotlinx.coroutines.*
import java.io.DataInputStream
import java.nio.ByteBuffer
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

internal data class PdfPage(val bitmap: Bitmap, val pages: Int, val index: Int)

internal class FilePreviewSession(private val context: Context) : AutoCloseable {
    private val ready = CompletableDeferred<android.os.Messenger>()
    private val closed = AtomicBoolean(false)
    private val executor = Executors.newSingleThreadExecutor()
    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName, binder: IBinder) { ready.complete(Messenger(binder)) }
        override fun onServiceDisconnected(name: ComponentName) { ready.completeExceptionally(java.io.IOException("File viewer stopped")) }
        override fun onBindingDied(name: ComponentName) { ready.completeExceptionally(java.io.IOException("File viewer stopped")); close() }
        override fun onNullBinding(name: ComponentName) { ready.completeExceptionally(java.io.IOException("File viewer unavailable")); close() }
    }
    private var bound = false
    init {
        val intent = Intent(context,FilePreviewService::class.java)
        bound = if (Build.VERSION.SDK_INT >= 29) context.bindIsolatedService(intent,Context.BIND_AUTO_CREATE,"preview" + java.util.UUID.randomUUID().toString().replace("-", ""),context.mainExecutor,connection)
            else context.bindService(intent,connection,Context.BIND_AUTO_CREATE)
        if(!bound) { close(); error("File viewer unavailable") }
    }
    suspend fun render(input: ParcelFileDescriptor, index: Int, width: Int): PdfPage = exchange(input, {
        require(index in 0..99_999 && width in 32..2048)
        Message.obtain(null,1,index,width)
    }, { it.bitmap.recycle() }) { stream ->
        check(stream.readInt()==PdfPreviewMagic)
        val w=stream.readInt(); val h=stream.readInt(); val pages=stream.readInt(); val receivedIndex=stream.readInt()
        check(w in 1..width && h in 1..2048 && w.toLong()*h<=PdfPreviewPixels && pages in 1..100_000 && receivedIndex==index && index<pages)
        PdfPage(readPreviewBitmap(stream,w,h),pages,index)
    }
    suspend fun render(input: ParcelFileDescriptor, format: String, request: String): FilePreview = exchange(input, {
        require(format.length<=32 && request.length<=1024)
        Message.obtain(null,2).apply { data=Bundle().apply { putString("format",format);putString("request",request) } }
    }, { it.close() }) { readFilePreview(it,request) }

    private suspend fun <T> exchange(input: ParcelFileDescriptor, message: () -> Message, dispose: (T) -> Unit, read: (DataInputStream) -> T): T {
        try {
            check(!closed.get())
            val request=message()
            val service=withTimeout(3000) { ready.await() }
            return withTimeout(12_000) { suspendCancellableCoroutine { continuation ->
                val pipe=ParcelFileDescriptor.createReliablePipe()
                continuation.invokeOnCancellation { runCatching { pipe[0].close() };runCatching { pipe[1].close() };close() }
                try {
                    request.data.apply { putParcelable("input",input);putParcelable("output",pipe[1]) }
                    service.send(request)
                    pipe[1].close()
                    executor.execute {
                        var value:T?=null
                        try {
                            DataInputStream(ParcelFileDescriptor.AutoCloseInputStream(pipe[0])).use { stream ->
                                value=read(stream)
                                check(stream.read()==-1)
                            }
                            val result=value!!; value=null
                            continuation.resume(result) { _, item, _ -> dispose(item) }
                        } catch(error:Exception) { value?.let(dispose);if(continuation.isActive)continuation.resumeWithException(error) }
                    }
                } catch(error:Exception) { pipe.forEach { runCatching { it.close() } };if(continuation.isActive)continuation.resumeWithException(error) }
            } }
        } finally { input.close() }
    }
    override fun close() {
        if(closed.compareAndSet(false,true)) {
            ready.cancel()
            if(bound) runCatching { context.unbindService(connection) }
            executor.shutdownNow()
        }
    }
}

internal fun readPreviewBitmap(stream: DataInputStream, width: Int, height: Int): Bitmap {
    val bytes=ByteArray(width*height*4)
    try {
        stream.readFully(bytes)
        val bitmap=Bitmap.createBitmap(width,height,Bitmap.Config.ARGB_8888)
        try { bitmap.copyPixelsFromBuffer(ByteBuffer.wrap(bytes)); return bitmap }
        catch(error:Exception) { bitmap.recycle();throw error }
    } finally { bytes.fill(0) }
}
