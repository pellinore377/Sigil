package org.sigil.compose

import android.app.Application
import android.net.Uri
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.json.JSONArray
import org.json.JSONObject
import org.sigil.*
import org.sigil.storage.NativeStorage
import org.sigil.storage.StorageKeyProvider
import java.security.SecureRandom
import java.text.DateFormat
import java.util.Date

/** Android owns lifecycle and IO dispatch; Rust owns credentials, trust and messages. */
class Messenger(application: Application) : AndroidViewModel(application) {
    var state by mutableStateOf(MessengerState())
        private set
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val mutex = Mutex()
    private var foreground = false
    private var nextSync = 0L
    private var published = false
    private var syncIssue: String? = null
    private var pages = 1
    private var post: Pair<Map<String, Any?>, String>? = null
    private var discoveryGeneration = 0L
    var authorizationUrl by mutableStateOf<String?>(null)
        private set

    init {
        scope.launch { serialized(false) { refresh() } }
        scope.launch {
            while (isActive) {
                delay(1000)
                if (foreground && state.phase == "connected" && System.currentTimeMillis() / 1000 >= nextSync) {
                    serialized(false) {
                        if (!published) { execute("publish"); published = true }
                        val result = execute("sync")
                        nextSync = result.getLong("next_at")
                        val issue = result.optional("issue")
                        if (result.getBoolean("ran")) {
                            if (state.issue == syncIssue || issue != null) state = state.copy(issue = issue)
                            syncIssue = issue
                        }
                        refresh()
                    }
                    // Rust also persists backoff; an IO failure must not create a busy loop.
                    nextSync = maxOf(nextSync, System.currentTimeMillis() / 1000 + 5)
                }
            }
        }
    }
    fun foreground(value: Boolean) { foreground = value; if (value) { nextSync = 0; published = false } }
    fun browserOpened() { authorizationUrl = null }
    fun callback(uri: Uri?) {
        if (uri == null || uri.scheme != "sigil" || uri.host != "oidc" || uri.query != null || uri.fragment != null) return
        val parts = uri.pathSegments
        if (parts.size != 2) return
        command("callback", mapOf("request_id" to parts[0], "completion" to parts[1]))
    }
    fun command(name: String, fields: Map<String, Any?>) {
        when (name) {
            "server_changed" -> {
                if (state.phase != "new") return
                discoveryGeneration++
                state = state.copy(loginAddress = fields["server"] as String, loginMethods = null, discovering = false, discoveryIssue = null, issue = null)
                return
            }
            "discover" -> { discover(fields["server"] as String); return }
            "dismiss" -> { state = state.copy(issue = null); return }
            "close" -> { state = state.copy(selected = null, messages = emptyList()); pages = 1; return }
            "open" -> { state = state.copy(selected = fields["peer"] as String, messages = emptyList()); pages = 1 }
            "older" -> pages++
            "read" -> if (!foreground) return
        }
        scope.launch {
            serialized(name != "read") {
                if (name !in listOf("open", "older")) {
                    val raw = if (name == "post" && post?.first == fields) post!!.second else request(name, fields).also {
                        if (name == "post") post = fields.toMap() to it
                    }
                    val result = native(raw)
                    result.optional("authorization_url")?.let { authorizationUrl = it }
                    if (name == "post") { post = null; state = state.copy(sent = state.sent + 1) }
                }
                refresh()
            }
        }
    }
    private fun discover(address: String) {
        if (state.phase != "new" || state.loginAddress != address || state.discovering) return
        val generation = ++discoveryGeneration
        state = state.copy(discovering = true, loginMethods = null, discoveryIssue = null)
        scope.launch {
            try {
                val result = mutex.withLock { execute("discover", mapOf("server" to address)) }
                if (generation == discoveryGeneration && state.phase == "new") state = state.copy(discovering = false,
                    loginMethods = LoginMethods(result.getString("server_name"), result.getBoolean("sso"), result.getBoolean("password"), result.getBoolean("invitation")))
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) {
                if (generation == discoveryGeneration) state = state.copy(discovering = false, discoveryIssue = "Couldn't verify this server. Check its address and your connection.")
            }
        }
    }
    private suspend fun serialized(progress: Boolean, work: suspend () -> Unit) = mutex.withLock {
        if (progress) state = state.copy(busy = true, issue = null)
        try { work() } catch (cancelled: CancellationException) { throw cancelled }
        catch (error: Exception) {
            val issue = if (error is NativeFailure) error.message else "Cannot access this device's storage. Stored keys have not been reset."
            if (progress) {
                try { refresh() } catch (cancelled: CancellationException) { throw cancelled } catch (_: Exception) { }
            }
            if (!progress) syncIssue = issue
            state = state.copy(issue = issue)
        } finally { if (progress) state = state.copy(busy = false) }
    }
    private fun request(name: String, fields: Map<String, Any?> = emptyMap()): String {
        val value = JSONObject().put("command", name)
        fields.forEach { (key, item) -> value.put(key, item ?: JSONObject.NULL) }
        if (name in listOf("post", "react", "pin", "read")) {
            val bytes = ByteArray(32).also { SecureRandom().nextBytes(it) }
            value.put("request", bytes.joinToString("") { "%02x".format(it) })
            value.put("timestamp", System.currentTimeMillis() / 1000)
        }
        return value.toString()
    }
    private suspend fun execute(name: String, fields: Map<String, Any?> = emptyMap()) = native(request(name, fields))
    private suspend fun native(raw: String): JSONObject = withContext(Dispatchers.IO) {
        val result = StorageKeyProvider(getApplication()).withKey { directory, key ->
            JSONObject(NativeStorage.execute(directory.path, key, raw))
        }
        if (!result.getBoolean("ok")) throw NativeFailure(result.getString("error"))
        result.getJSONObject("value")
    }
    private suspend fun refresh() {
        val value = execute("state")
        val phase = value.getString("phase")
        if (phase != "connected") { state = state.copy(phase = phase, loginAddress = if (phase == "new") state.loginAddress else value.optional("server") ?: state.loginAddress); return }
        val chats = value.getJSONArray("chats").objects().map { chat ->
            ChatSummary(chat.getString("id"), chat.getString("address"), chat.getString("preview"), clock(chat.getLong("timestamp")), chat.getBoolean("verified"),
                chat.getJSONArray("devices").objects().map { device -> ChatDevice(device.getString("id"), device.getString("fingerprint"), device.getBoolean("verified"), device.getBoolean("blocked"), device.getBoolean("changed")) })
        }
        state = state.copy(phase = phase, address = value.getString("address"), device = value.getString("device"), fingerprint = value.getString("fingerprint"), chats = chats)
        val peer = state.selected ?: return
        val messages = mutableListOf<ChatMessage>()
        var before: Long? = null
        repeat(pages) {
            val timeline = execute("timeline", mapOf("peer" to peer, "before" to before))
            messages += timeline.getJSONArray("messages").objects().map { message ->
                ChatMessage(message.getString("id"), message.getString("author"), message.getString("text"), message.getBoolean("mine"), clock(message.getLong("timestamp")),
                    message.getString("delivery"), message.getBoolean("pinned"), message.getJSONArray("reactions").strings(), message.getJSONArray("my_reactions").strings(), message.optional("reply"), message.getBoolean("read_by_me"))
            }
            before = if (timeline.isNull("next")) null else timeline.getLong("next")
            if (before == null) {
                if (state.selected == peer) state = state.copy(messages = messages, more = false)
                return
            }
        }
        if (state.selected == peer) state = state.copy(messages = messages, more = before != null)
    }
    override fun onCleared() { scope.cancel() }
    private class NativeFailure(message: String) : Exception(message)
}
private fun JSONObject.optional(key: String): String? = if (isNull(key)) null else optString(key).ifEmpty { null }
private fun JSONArray.objects() = (0 until length()).map { getJSONObject(it) }
private fun JSONArray.strings() = (0 until length()).map { getString(it) }
private fun clock(seconds: Long): String = if (seconds == 0L) "" else DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(seconds * 1000))
