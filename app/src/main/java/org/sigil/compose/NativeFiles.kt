package org.sigil.compose

import android.app.Application
import android.net.Uri
import android.provider.OpenableColumns
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.json.JSONObject
import org.sigil.Transfer
import org.sigil.storage.NativeStorage
import org.sigil.storage.StorageKeyProvider
import java.io.InputStream
import java.security.SecureRandom

internal class NativeFiles(private val app: Application, private val scope: CoroutineScope,
    private val update: (List<Transfer>, Boolean) -> Unit, private val issue: (String) -> Unit) {
    private val mutex = Mutex()
    private val staging = java.util.concurrent.ConcurrentHashMap<String, Job>()
    @Volatile var enabled = false
    private var next = 0L
    init {
        scope.launch(Dispatchers.IO) {
            while (isActive) {
                delay(1000)
                if (!enabled) continue
                try {
                    mutex.withLock {
                        var sent = false
                        if (System.currentTimeMillis() / 1000 >= next) {
                            val work = NativeSync.files(app)
                            next = work.getLong("next_at")
                            sent = work.getInt("sent") > 0
                            if (!work.isNull("issue")) withContext(Dispatchers.Main) { issue(work.getString("issue")) }
                        }
                        publish(sent)
                    }
                } catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { next = System.currentTimeMillis() / 1000 + 10 }
            }
        }
    }
    private fun execute(name: String, fields: Map<String, Any?> = emptyMap()): JSONObject {
        val request = JSONObject().put("command", name)
        fields.forEach { (key, value) -> request.put(key, JSONObject.wrap(value)) }
        val result = StorageKeyProvider(app).withKey { directory, key -> JSONObject(NativeStorage.execute(directory.path, key, request.toString())) }
        check(result.getBoolean("ok")) { result.optString("error", "File operation failed") }
        return result.getJSONObject("value")
    }
    private suspend fun publish(sent: Boolean = false) {
        val rows = execute("files").getJSONArray("uploads")
        val files = (0 until rows.length()).map { index -> rows.getJSONObject(index).let { Transfer(it.getString("request"), it.getString("peer"), it.getString("name"), it.getLong("length"), it.getString("phase"), it.optBoolean("draft"), it.getString("media_type")) } }
        withContext(Dispatchers.Main) { update(files, sent) }
    }
    fun cancel(request: String) { scope.launch(Dispatchers.IO) { mutex.withLock {
        try { execute("file_cancel", mapOf("request" to request)); staging[request]?.cancel(); publish() }
        catch (_: Exception) { withContext(Dispatchers.Main) { issue("The attachment could not be cancelled. It may already have been sent.") } }
    } } }
    fun send(request: String, caption: String, committed: () -> Unit) { scope.launch(Dispatchers.IO) { mutex.withLock {
        try {
            execute("file_send", mapOf("request" to request, "caption" to caption)); next = 0; publish()
            withContext(Dispatchers.Main) { committed() }
            NativeSync.enqueue(app)
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { withContext(Dispatchers.Main) { issue("Could not queue this attachment. Your draft is still available.") } }
    } } }
    fun import(target: Map<String, Any?>, uri: Uri) { scope.launch(Dispatchers.IO) {
        try {
            var name = "Attachment"
            var size = -1L
            app.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { row ->
                if (row.moveToFirst()) { name = row.getString(0) ?: name; if (!row.isNull(1)) size = row.getLong(1) }
            }
            val type = app.contentResolver.getType(uri) ?: "application/octet-stream"
            app.contentResolver.openInputStream(uri)?.use { stream ->
                if (size >= 0) stage(target + ("draft" to true), name, type, size, stream)
                else {
                    val bytes = stream.chunk(32 * 1024 * 1024 + 1)
                    try { check(bytes.size <= 32 * 1024 * 1024); bytes.inputStream().use { stage(target + ("draft" to true), name, type, bytes.size.toLong(), it) } }
                    finally { bytes.fill(0) }
                }
            } ?: error("Cannot open selected file")
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { withContext(Dispatchers.Main) { issue("Could not import this file. Check access, available storage, and its size.") } }
    } }
    fun forward(fields: Map<String, Any?>, metadata: JSONObject) { scope.launch(Dispatchers.IO) {
        try {
            val source = fields["source"] as String
            val author = fields["author"] as String
            val message = fields["message"] as String
            val size = metadata.getLong("length")
            EncryptedMedia(app, source, author, message, size).use { media ->
                val stream = object : InputStream() {
                    var position = 0L
                    override fun read(): Int { val b = ByteArray(1); return if (read(b, 0, 1) == -1) -1 else b[0].toInt() and 255 }
                    override fun read(buffer: ByteArray, offset: Int, length: Int): Int = media.readAt(position, buffer, offset, length).also { if (it > 0) position += it }
                }
                stage(mapOf("peer" to fields["peer"], "caption" to metadata.optString("caption")), metadata.getString("name"), metadata.getString("media_type"), size, stream) {
                    NativeSync.enqueue(app)
                    withTimeout(120_000) {
                        while (true) {
                            check(!NativeSignOut.pending(app))
                            val file = execute("file_get", mapOf("peer" to source, "author" to author, "message" to message))
                            if (file.getString("phase") in listOf("Complete", "Published", "Restored")) break
                            delay(1000)
                        }
                    }
                }
            }
        } catch (cancelled: CancellationException) { if (cancelled !is TimeoutCancellationException) throw cancelled; withContext(Dispatchers.Main) { issue("The attachment download timed out. You can retry forwarding it.") } }
        catch (_: Exception) { withContext(Dispatchers.Main) { issue("Could not forward this attachment. Check its availability and your connection.") } }
    } }
    suspend fun stage(target: Map<String, Any?>, name: String, type: String, size: Long, stream: InputStream, prepare: suspend () -> Unit = {}) {
        val request = ByteArray(32).also { SecureRandom().nextBytes(it) }.joinToString("") { "%02x".format(it) }
        try {
            mutex.withLock { execute("file_begin", target + mapOf("request" to request, "timestamp" to System.currentTimeMillis() / 1000, "length" to size, "name" to name, "media_type" to type)); staging[request] = currentCoroutineContext().job; publish() }
            prepare()
            var total = 0L
            var index = 0
            while (total < size) {
                currentCoroutineContext().ensureActive()
                val count = minOf(1024 * 1024L, size - total).toInt()
                val bytes = stream.chunk(count)
                try {
                    check(bytes.size == count)
                    mutex.withLock { check(StorageKeyProvider(app).withKey { directory, key -> NativeStorage.stageFile(directory.path, key, request, index, bytes) }) }
                } finally { bytes.fill(0) }
                total += count; index++
            }
            check(stream.read() == -1)
            mutex.withLock { execute("file_finish", mapOf("request" to request)); next = 0; publish() }
            NativeSync.enqueue(app)
        } catch (error: Exception) {
            withContext(NonCancellable) { mutex.withLock { runCatching { execute("file_cancel", mapOf("request" to request)) }; publish() } }
            throw error
        } finally { staging.remove(request) }
    }
}

private fun InputStream.chunk(limit: Int): ByteArray {
    val buffer = ByteArray(limit)
    var used = 0
    try {
        while (used < limit) {
            val count = read(buffer, used, limit - used)
            if (count < 0) break
            if (count == 0) { val byte = read(); if (byte < 0) break; buffer[used++] = byte.toByte() }
            else used += count
        }
        return if (used == limit) buffer else buffer.copyOf(used).also { buffer.fill(0) }
    } catch (error: Exception) { buffer.fill(0); throw error }
}
