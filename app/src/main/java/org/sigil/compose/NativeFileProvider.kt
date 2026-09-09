package org.sigil.compose

import android.content.*
import android.database.MatrixCursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.os.Handler
import android.os.HandlerThread
import android.os.ProxyFileDescriptorCallback
import android.os.storage.StorageManager
import android.system.ErrnoException
import android.system.OsConstants
import android.provider.OpenableColumns
import org.sigil.ChatMessage
import java.security.SecureRandom
import java.util.concurrent.ConcurrentHashMap

class NativeFileProvider : ContentProvider() {
    companion object {
        private val grants = ConcurrentHashMap<String, Pair<Long, ChatMessage>>()
        private val readers = java.util.concurrent.Semaphore(8)
        private val handler by lazy { Handler(HandlerThread("attachment-reader").apply { start() }.looper) }
        fun open(context: Context, message: ChatMessage) {
            val now = SystemClock.elapsedRealtime()
            grants.entries.removeIf { it.value.first < now }
            if (grants.size >= 8) return
            val token = ByteArray(24).also { SecureRandom().nextBytes(it) }.joinToString("") { "%02x".format(it) }
            grants[token] = now + 900000 to message
            val uri = Uri.Builder().scheme("content").authority("${context.packageName}.files").appendPath(token).build()
            val intent = Intent(Intent.ACTION_VIEW).setDataAndType(uri, message.attachment!!.mediaType).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION).apply { clipData = ClipData.newRawUri("Attachment", uri) }
            try { context.startActivity(Intent.createChooser(intent, "Open attachment")) }
            catch (_: ActivityNotFoundException) { grants.remove(token) }
        }
    }
    private fun message(uri: Uri): ChatMessage {
        if (uri.pathSegments.size != 1 || uri.query != null || uri.fragment != null) throw java.io.FileNotFoundException()
        val value = grants[uri.lastPathSegment] ?: throw java.io.FileNotFoundException()
        if (value.first < SystemClock.elapsedRealtime()) { grants.remove(uri.lastPathSegment); throw java.io.FileNotFoundException() }
        return value.second
    }
    override fun onCreate() = true
    override fun getType(uri: Uri) = message(uri).attachment!!.mediaType
    override fun query(uri: Uri, projection: Array<out String>?, selection: String?, selectionArgs: Array<out String>?, sortOrder: String?): android.database.Cursor {
        val file = message(uri).attachment!!
        val columns = projection ?: arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE)
        return MatrixCursor(columns).apply { addRow(columns.map { when(it) { OpenableColumns.DISPLAY_NAME -> file.name; OpenableColumns.SIZE -> file.bytes; else -> null } }) }
    }
    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
        if (mode != "r") throw java.io.FileNotFoundException()
        val message = message(uri)
        if (!readers.tryAcquire()) throw java.io.FileNotFoundException("Too many open attachments")
        val media = EncryptedMedia(context!!, message)
        try {
            return context!!.getSystemService(StorageManager::class.java).openProxyFileDescriptor(ParcelFileDescriptor.MODE_READ_ONLY, object : ProxyFileDescriptorCallback() {
                override fun onGetSize() = media.size
                override fun onRead(offset: Long, size: Int, data: ByteArray): Int {
                    try {
                        var count = 0
                        while (count < size) { val read = media.readAt(offset + count, data, count, size - count); if (read < 0) break; count += read }
                        return count
                    } catch (_: Exception) { data.fill(0); throw ErrnoException("read", OsConstants.EIO) }
                }
                override fun onRelease() { media.close(); readers.release() }
            }, handler)
        } catch (error: Exception) { media.close(); readers.release(); throw error }
    }
    override fun insert(uri: Uri, values: ContentValues?): Uri? = throw UnsupportedOperationException()
    override fun update(uri: Uri, values: ContentValues?, selection: String?, selectionArgs: Array<out String>?) = throw UnsupportedOperationException()
    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?) = throw UnsupportedOperationException()
}
