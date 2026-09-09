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
    var signOutStage by mutableStateOf(NativeSignOut.stage(application))
        private set
    var signOutBusy by mutableStateOf(false)
        private set
    var signOutIssue by mutableStateOf<String?>(null)
        private set
    internal val calls = NativeCalls(application, { history, active -> state = state.copy(calls = history, call = active) }, { state = state.copy(issue = it) })
    private val files = NativeFiles(application, scope, { uploads, sent ->
        state = state.copy(transfers = uploads)
        if (sent) scope.launch { serialized(false) { refresh() } }
    }, { state = state.copy(issue = it) })
    private val voice = VoiceRecorder(scope, { peer, bytes, target -> bytes.inputStream().use { files.stage(target + ("peer" to peer), "Voice message.aac", "audio/aac", bytes.size.toLong(), it) } }, { state = state.copy(voice = it) }, { state = state.copy(issue = it) })
    var microphoneRequest by mutableStateOf<Pair<String, Map<String, Any?>>?>(null)
        private set
    fun microphoneResult(granted: Boolean) { val target = microphoneRequest; microphoneRequest = null; if (granted && target != null) voice.start(target.first, target.second) else if (!granted) state = state.copy(issue = "Microphone permission is needed to record a voice message.") }
    var picker by mutableStateOf<Pair<Map<String, Any?>, String>?>(null)
        private set
    var notificationPermission by mutableStateOf(false)
        private set
    var recoveryKey by mutableStateOf<String?>(null)
        private set
    var restoringRecovery by mutableStateOf(false)
        private set
    private fun recoveryPreference(value: Boolean) { getApplication<Application>().getSharedPreferences("recovery", 0).edit().putBoolean("requested", value).apply() }
    fun dismissRestoreRecovery() { if (!state.busy) { restoringRecovery = false; recoveryPreference(false) } }
    var recoveringAccount by mutableStateOf(false)
        private set
    fun dismissAccountRecovery() { if (!state.busy) recoveringAccount = false }
    fun dismissRecovery() { recoveryKey = null }
    var contactQr by mutableStateOf<JSONObject?>(null)
        private set
    private fun contactQr(fields: Map<String, Any?>) {
        val context = mapOf("peer" to (fields["peer"] ?: contactQr?.optional("peer")), "review" to (fields["review"] ?: contactQr?.optional("review")))
        if (fields["action"] == "scan" && fields["qr"] == null) {
            contactQr = JSONObject().put("stage", "scan"); context.forEach { (k,v) -> if (v != null) contactQr!!.put(k,v) }; return
        }
        scope.launch { serialized(true) {
            val result = execute("contact_qr", fields + context)
            result.optional("open")?.let { state = state.copy(selected = it); pages = 1 }
            contactQr = result.takeUnless { it.getString("stage") in listOf("done", "none") }
            context.forEach { (k,v) -> if (v != null) contactQr?.put(k,v) }
            refresh()
        } }
    }
    var deviceLink by mutableStateOf<JSONObject?>(null)
        private set
    private fun linkResult(value: JSONObject) { deviceLink = value.takeUnless { it.getString("stage") == "none" } }
    private fun deviceLink(fields: Map<String, Any?>) {
        if (fields["action"] == "pause") { deviceLink = null; return }
        scope.launch { serialized(true) {
        try { linkResult(execute("device_link", fields)); refresh(); NativeSync.enable(getApplication(), state.phase == "connected") }
        finally { linkResult(execute("device_link", mapOf("action" to "status"))) }
    } } }
    fun notificationPermissionResult() { notificationPermission = false; state = state.copy(notifications = NativeNotifications.settings(getApplication())) }
    fun pickerOpened() { picker = null }
    var wallpaperRevision by mutableStateOf(0L)
        private set
    var photoRevision by mutableStateOf(0L)
        private set
    fun importFile(target: Map<String, Any?>, uri: Uri) {
        if (NativeSignOut.pending(getApplication())) return
        when { target["profile_photo"] == "true" -> changeProfilePhoto(uri); target["wallpaper"] == "true" -> changeWallpaper(target["peer"] as String, uri); else -> files.import(target, uri) }
    }
    private fun changeProfilePhoto(uri: Uri?) { scope.launch { serialized(true) {
        stageProfilePhoto(getApplication(), uri); photoRevision++; state = state.copy(photoPending = true)
        execute("photo_publish"); photoRevision++; refresh()
    } } }
    private fun changeWallpaper(peer: String, uri: Uri?) { scope.launch { serialized(true) { saveWallpaper(getApplication(), peer, uri); wallpaperRevision++ } } }
    fun importPhoto(target: Map<String, Any?>, bytes: ByteArray) { scope.launch(Dispatchers.IO) {
        try { bytes.inputStream().use { files.stage(target, "Photo.jpg", "image/jpeg", bytes.size.toLong(), it) } }
        catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { withContext(Dispatchers.Main) { state = state.copy(issue = "Could not queue this photo.") } }
        finally { bytes.fill(0) }
    } }
    private var foreground = false
    private var nextSync = 0L
    private var nextAccess = 0L
    private var published = false
    private var syncIssue: String? = null
    private var pages = 1
    private var post: Pair<Map<String, Any?>, String>? = null
    private var groupCreate: Pair<Map<String, Any?>, String>? = null
    private var discoveryGeneration = 0L
    private var searchGeneration = 0L
    private var searchAfter: Long? = null
    private var searchCategory = ""
    private var anchor: Pair<String, String>? = null
    private var nextPushStatus = 0L
    private val pendingUiSettings = mutableMapOf<Pair<String?, String>, Any?>()
    private var timelineFilter: Map<String, Any?> = emptyMap()
    var authorizationUrl by mutableStateOf<String?>(null)
        private set

    init {
        if (signOutStage.isEmpty()) scope.launch { serialized(false) { refresh(); linkResult(execute("device_link", mapOf("action" to "status"))) } }
        else NativeSync.enable(application, false)
        scope.launch {
            while (isActive) {
                delay(1000)
                if (signOutStage.isEmpty() && foreground && state.phase == "connected" && System.currentTimeMillis() / 1000 >= nextSync) {
                    serialized(false) {
                        if (!published) { execute("publish"); published = true }
                        val result = execute("sync", mapOf("interactive" to true))
                        nextSync = result.getLong("next_at")
                        val issue = result.optional("issue")
                        if (result.getBoolean("ran")) {
                            if (state.issue == syncIssue || issue != null) state = state.copy(issue = issue)
                            syncIssue = issue
                        }
                        refresh()
                        NativeSync.presence(getApplication(), state.call?.call?.phase in listOf("active", "joining"))
                    }
                    // Rust also persists backoff; an IO failure must not create a busy loop.
                    nextSync = maxOf(nextSync, System.currentTimeMillis() / 1000 + 1)
                }
            }
        }
    }
    fun foreground(value: Boolean) {
        if (NativeSignOut.pending(getApplication())) { foreground = false; files.enabled = false; NativeSync.enable(getApplication(), false); return }
        NativeSync.foreground(value)
        NativeNotifications.foreground(getApplication(), value)
        if (value) state = state.copy(notifications = NativeNotifications.settings(getApplication()))
        foreground = value; files.enabled = value && state.phase == "connected"
        NativeSync.enable(getApplication(), state.phase == "connected")
        if (value) { nextSync = 0; published = false }
        else { if (state.voice.phase == "Recording") voice.stop(); voice.pausePreview(); if (state.phase == "connected") NativeSync.enqueue(getApplication()) }
        if (state.phase == "connected") scope.launch { try { NativeSync.presence(getApplication(), state.call?.call?.phase in listOf("active", "joining")) } catch (cancelled: CancellationException) { throw cancelled } catch (_: Exception) { } }
    }
    fun browserOpened() { authorizationUrl = null }
    fun callback(uri: Uri?) {
        if (uri == null || uri.scheme != "sigil" || uri.host != "oidc" || uri.query != null || uri.fragment != null) return
        val parts = uri.pathSegments
        if (parts.size != 2) return
        command("callback", mapOf("request_id" to parts[0], "completion" to parts[1]))
    }
    fun command(name: String, fields: Map<String, Any?>) {
        if (NativeSignOut.pending(getApplication())) return
        if (name.startsWith("call_")) { calls.command(name, fields + ("name" to (fields["peer"] as? String)?.let { peer -> state.chats.find { it.id == peer }?.name })); return }
        when (name) {
            "recovery_account_open" -> { recoveringAccount = true; return }
            "recover_account" -> recoveringAccount = false
            "recovery_restore_open" -> { restoringRecovery = true; return }
            "sign_out" -> {
                if (state.call != null || calls.occupied) { state = state.copy(issue = "End or leave your call before signing out."); return }
                if (state.voice.phase != "Idle") { state = state.copy(issue = "Send or discard your voice recording before signing out."); return }
                signOutStage = "confirm"; signOutIssue = null; return
            }
            "photo_choose" -> { picker = emptyMap<String, Any?>() to "Profile photo"; return }
            "photo_remove" -> { changeProfilePhoto(null); return }
            "contact_qr" -> { contactQr(fields); return }
            "device_link" -> { deviceLink(fields); return }
            "wallpaper_remove" -> { changeWallpaper(fields["peer"] as String, null); return }
            "notification_settings" -> { notificationPermissionResult(); scope.launch { serialized(true) { state = state.copy(push = withContext(Dispatchers.IO) { NativePush.settings(getApplication()) }) } }; return }
            "push_select", "push_disable" -> {
                scope.launch { serialized(true) {
                    val old = execute("push", mapOf("action" to "status")).optional("connection")
                    if (name == "push_disable") {
                        execute("push", mapOf("action" to "disable"))
                        withContext(Dispatchers.IO) { NativePush.unregister(getApplication(), old) }
                    } else {
                        val distributor = fields["distributor"] as String
                        val registration = execute("push", mapOf("action" to "prepare", "replace" to (distributor != NativePush.selected(getApplication()))))
                        if (registration.optBoolean("unavailable")) { state = state.copy(issue = "Your server has not enabled UnifiedPush. Ask its administrator to enable push delivery."); return@serialized }
                        if (registration.optString("remote") == "invalid" && !registration.getBoolean("awaiting_endpoint")) execute("push", mapOf("action" to "retry"))
                        withContext(Dispatchers.IO) {
                            NativePush.register(getApplication(), distributor, registration)
                            if (old != registration.getString("connection")) NativePush.unregister(getApplication(), old)
                        }
                    }
                    state = state.copy(push = withContext(Dispatchers.IO) { NativePush.settings(getApplication()) })
                } }
                return
            }
            "notification_permission" -> { if (android.os.Build.VERSION.SDK_INT >= 33) notificationPermission = true else NativeNotifications.systemSettings(getApplication()); return }
            "notification_system_settings" -> { NativeNotifications.systemSettings(getApplication()); return }
            "notification_change" -> { NativeNotifications.change(getApplication(), fields["key"] as String, fields["enabled"] as Boolean); notificationPermissionResult(); return }
            "record_start" -> { val peer = fields["peer"] as String; if (getApplication<Application>().checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) == android.content.pm.PackageManager.PERMISSION_GRANTED) voice.start(peer, fields) else microphoneRequest = peer to fields.toMap(); return }
            "record_stop" -> { voice.stop(); return }
            "record_cancel" -> { voice.discard(); return }
            "record_send" -> { voice.send(); return }
            "record_preview" -> { voice.playPreview(); return }
            "attachment_pick" -> { picker = fields.filterKeys { it != "kind" } to (fields["kind"] as String); return }
            "delete_conversation" -> {
                scope.launch { serialized(true) {
                    if (fields["leave"] == true) execute("leave_group", mapOf("peer" to fields["peer"]))
                    execute("clear_conversation", mapOf("peer" to fields["peer"]))
                    if (state.selected == fields["peer"]) { state = state.copy(selected = null, messages = emptyList()); anchor = null; pages = 1 }
                    refresh()
                    NativeSync.enqueue(getApplication())
                } }
                return
            }
            "file_cancel" -> { files.cancel(fields["request"] as String); return }
            "server_changed" -> {
                if (state.phase != "new") return
                discoveryGeneration++
                state = state.copy(loginAddress = fields["server"] as String, loginMethods = null, discovering = false, discoveryIssue = null, issue = null)
                return
            }
            "discover" -> { discover(fields["server"] as String); return }
            "search" -> { search(fields["query"] as String, fields["category"] as? String ?: ""); return }
            "search_more" -> { if (!state.searching && state.searchMore) search(state.searchQuery, searchCategory, true); return }
            "create_collection" -> {
                val bytes = ByteArray(32).also { SecureRandom().nextBytes(it) }
                val id = bytes.joinToString("") { "%02x".format(it) }
                command("organize", mapOf("peer" to null, "value" to mapOf("Collection" to mapOf("id" to id, "name" to fields["name"], "present" to true))))
                command("organize", mapOf("peer" to null, "value" to mapOf("UiSetting" to mapOf("key" to "collection_icon.$id", "value" to fields["icon"]))))
                (fields["peers"] as? List<*>)?.filterIsInstance<String>()?.forEach { command("organize", mapOf("peer" to it, "value" to mapOf("CollectionMember" to mapOf("id" to id, "present" to true)))) }
                return
            }
            "dismiss" -> { state = state.copy(issue = null); return }
            "close" -> { state = state.copy(selected = null, messages = emptyList(), historical = false); anchor = null; pages = 1; return }
            "open" -> {
                anchor = (fields["author"] as? String)?.let { author -> (fields["message"] as? String)?.let { author to it } }
                val target = (fields["thread_author"] as? String)?.let { author -> (fields["thread_message"] as? String)?.let { ThreadTarget(author, it) } }
                timelineFilter = if (target == null) emptyMap() else mapOf("category" to "Timeline", "thread_author" to target.author, "thread_message" to target.id)
                state = state.copy(selected = fields["peer"] as String, messages = emptyList(), historical = anchor != null, threadTarget = target); pages = 1
            }
            "timeline_filter" -> { if (timelineFilter == fields) return; timelineFilter = fields.toMap(); pages = 1; state = state.copy(messages = emptyList()) }
            "latest" -> { anchor = null; state = state.copy(historical = false); pages = 1 }
            "older" -> pages++
            "read" -> if (!foreground) return
        }
        val setting = (fields["value"] as? Map<*, *>)?.get("UiSetting") as? Map<*, *>
        val preference = if (name == "organize") (setting?.get("key") as? String)?.let { (fields["peer"] as? String) to it } else null
        if (preference != null) pendingUiSettings[preference] = setting?.get("value")
        scope.launch {
            serialized(preference == null && name !in listOf("read", "typing", "draft")) {
                if (preference != null) {
                    if (pendingUiSettings[preference] != setting?.get("value")) return@serialized
                }
                if (name == "open" && !(fields["peer"] as String).let { it == "self" || it.startsWith("group:") || it.startsWith("history:") }) execute("organize", mapOf("peer" to fields["peer"], "value" to mapOf("UiSetting" to mapOf("key" to "opened", "value" to "true"))))
                if (name !in listOf("open", "older", "latest", "timeline_filter")) {
                    val retry = name == "post" && post?.first == fields
                    val raw = if (retry) post!!.second else if (name == "group_create" && groupCreate?.first == fields) groupCreate!!.second else request(name, fields).also {
                        if (name == "post") post = fields.toMap() to it
                        if (name == "group_create") groupCreate = fields.toMap() to it
                    }
                    val alreadyQueued = retry && execute("post_status", mapOf("peer" to fields["peer"], "request" to JSONObject(raw).getString("request"))).getBoolean("queued")
                    if (name == "recover_account") recoveryPreference(true)
                    val result = if (alreadyQueued) JSONObject() else native(raw)
                    if (name == "cancel_login") recoveryPreference(false)
                    accountAccess(result)
                    result.optJSONObject("forward_file")?.let { files.forward(fields, it) }
                    if (name == "oidc_account") nextAccess = 0
                    result.optional("open")?.let { state = state.copy(selected = it); groupCreate = null; pages = 1 }
                    if (name in listOf("profile", "set_profile")) state = state.copy(profileName = result.getString("display_name"), profileRevision = result.getLong("revision"))
                    if (name in listOf("photo_publish", "photo_retry", "photo_cancel")) photoRevision++
                    if (name in listOf("devices", "revoke_device")) {
                        val merged = (if (fields["cursor"] != null) state.devices else emptyList()).associateBy { it.id }.toMutableMap()
                        result.getJSONArray("devices").objects().forEach { value ->
                            val id = value.getString("id"); val prior = merged[id]
                            merged[id] = AccountDevice(id, value.getBoolean("current"), if (value.getBoolean("current")) android.os.Build.MODEL else value.optional("label") ?: prior?.label,
                                if (value.isNull("revoked")) prior?.revoked else value.getBoolean("revoked"), if (value.isNull("expires")) prior?.expires else value.getLong("expires"),
                                value.optional("fingerprint") ?: prior?.fingerprint, if (value.isNull("fingerprint")) prior?.verified == true else value.getBoolean("verified"))
                        }
                        state = state.copy(devices = merged.values.sortedByDescending { it.current }, devicesNext = result.optional("next"))
                    }
                    if (name == "contact_policy") state = state.copy(allowRequests = result.getBoolean("enabled"))
                    if (name == "recovery_generate") recoveryKey = result.getString("secret")
                    if (name in listOf("storage", "recovery_enable", "recovery_policy", "recovery_restore")) {
                        storage(result)
                        if (name in listOf("recovery_enable", "recovery_restore")) { dismissRecovery(); restoringRecovery = false; recoveryPreference(false); NativeSync.enqueue(getApplication()) }
                    }
                    result.optional("authorization_url")?.let { authorizationUrl = it }
                    if (name in listOf("post", "edit")) { post = null; state = state.copy(sent = state.sent + 1, sentText = fields["text"] as? String) }
                }
                refresh()
                if (preference != null && pendingUiSettings[preference] == setting?.get("value")) pendingUiSettings.remove(preference)
            }
        }
    }
    private fun search(query: String, category: String, more: Boolean = false) {
        val generation = ++searchGeneration
        if (!more) searchAfter = null
        searchCategory = category
        state = state.copy(searchQuery = query, searchHits = if (more) state.searchHits else emptyList(), searching = true)
        scope.launch {
            try {
                var after = searchAfter
                val hits = state.searchHits.toMutableList()
                val initialSize = hits.size
                do {
                    val result = mutex.withLock { execute("search", mapOf("query" to query, "after" to after, "category" to category)) }
                    if (generation != searchGeneration) return@launch
                    hits += result.getJSONArray("hits").objects().map { hit -> SearchHit(hit.getString("peer"), hit.getString("id"), hit.getString("author"), hit.getString("text"), clock(hit.getLong("timestamp")), hit.getBoolean("pinned"), hit.getBoolean("noted"), hit.getString("kind"), hit.getBoolean("thread"), hit.optional("thread_author")?.let { author -> hit.optional("thread_message")?.let { ThreadTarget(author, it) } }) }
                    after = if (result.isNull("next")) null else result.getLong("next")
                    searchAfter = after
                    state = state.copy(searchHits = hits.toList(), searchMore = after != null, searching = after != null && hits.size - initialSize < 64)
                    yield()
                } while (after != null && hits.size - initialSize < 64)
                searchAfter = after
                state = state.copy(searching = false, searchMore = after != null)
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (error: Exception) { if (generation == searchGeneration) state = state.copy(searching = false, issue = "Search could not finish.") }
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
    fun signOut(action: String) {
        if (signOutBusy) return
        if (action == "cancel") { if (signOutStage == "confirm") signOutStage = ""; return }
        if (action !in listOf("confirm", "retry", "erase") || signOutStage.isEmpty()) return
        signOutBusy = true; signOutIssue = null
        scope.launch { mutex.withLock {
            try {
                if (action == "confirm") {
                    check(signOutStage == "confirm" && state.call == null && !calls.occupied)
                    NativeSignOut.save(getApplication(), "pending"); signOutStage = "pending"
                    files.enabled = false; NativeSync.enable(getApplication(), false)
                }
                if (action != "erase") {
                    check(signOutStage == "pending")
                    check(execute("sign_out").getBoolean("revoked"))
                    NativeSignOut.save(getApplication(), "confirmed"); signOutStage = "confirmed"
                }
                check(signOutStage in listOf("pending", "confirmed"))
                check(NativeSignOut.erase(getApplication()))
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) {
                NativeSignOut.stage(getApplication()).takeIf { it.isNotEmpty() }?.let { signOutStage = it }
                signOutIssue = if (signOutStage == "confirmed") "Android could not remove the app’s data. Try again, or clear Sigil’s storage in Android settings."
                    else if (signOutStage == "confirm") "Could not save the sign-out request. Nothing has been removed."
                    else "Could not confirm revocation. Local data has been kept."
                signOutBusy = false
            }
        } }
    }
    private suspend fun serialized(progress: Boolean, work: suspend () -> Unit) = mutex.withLock {
        if (NativeSignOut.pending(getApplication())) return@withLock
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
        fields.forEach { (key, item) -> value.put(key, JSONObject.wrap(item)) }
        if (name in listOf("oidc", "enroll")) value.put("label", android.os.Build.MODEL.take(60))
        if (name == "card_action") value.put("timestamp", System.currentTimeMillis() / 1000)
        if (name in listOf("post", "place", "group_create", "react", "pin", "read", "mark_read", "snooze", "forward", "organize", "edit", "delete", "clear_conversation", "note", "typing", "draft")) {
            val bytes = ByteArray(32).also { SecureRandom().nextBytes(it) }
            value.put("request", bytes.joinToString("") { "%02x".format(it) })
            value.put("timestamp", System.currentTimeMillis() / 1000)
            if (name == "post") value.put("timezone", java.util.TimeZone.getDefault().id)
        }
        return value.toString()
    }
    private suspend fun execute(name: String, fields: Map<String, Any?> = emptyMap()) = if (name == "sync") NativeSync.run(getApplication(), fields["interactive"] == true) else native(request(name, fields))
    private suspend fun native(raw: String): JSONObject = withContext(Dispatchers.IO) {
        val result = StorageKeyProvider(getApplication()).withKey { directory, key ->
            JSONObject(NativeStorage.execute(directory.path, key, raw))
        }
        if (!result.getBoolean("ok")) throw NativeFailure(result.getString("error"))
        result.getJSONObject("value")
    }
    private fun pendingUi(peer: String?, stored: Map<String, String>) = stored + pendingUiSettings.mapNotNull { (key, value) ->
        if (key.first == peer && value is String) key.second to value else null
    }
    private suspend fun refresh() {
        val value = execute("state")
        val phase = value.getString("phase")
        files.enabled = foreground && phase == "connected"
        if (phase != "connected") { state = state.copy(phase = phase, loginAddress = if (phase == "new") state.loginAddress else value.optional("server") ?: state.loginAddress); return }
        if (contactQr?.optString("stage") == "show") { val status = execute("contact_qr", mapOf("action" to "status")); contactQr = JSONObject(contactQr.toString()).put("consumed", status.getBoolean("consumed")).put("expired", status.getBoolean("expired")) }
        NativePush.resume(getApplication())
        if (state.push != null && android.os.SystemClock.elapsedRealtime() >= nextPushStatus) {
            state = state.copy(push = withContext(Dispatchers.IO) { NativePush.settings(getApplication()) })
            nextPushStatus = android.os.SystemClock.elapsedRealtime() + 10_000
        }
        if (value.optBoolean("recover_history") && getApplication<Application>().getSharedPreferences("recovery", 0).getBoolean("requested", false)) restoringRecovery = true
        if (state.storage != null) storage(execute("storage"))
        if (android.os.SystemClock.elapsedRealtime() >= nextAccess) {
            nextAccess = android.os.SystemClock.elapsedRealtime() + 300_000
            try { accountAccess(execute("account_access")) }
            catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { }
        }
        calls.refresh(execute("calls"))
        state = state.copy(readReceipts = value.getBoolean("read_receipts"), typingIndicators = value.getBoolean("typing_indicators"), presenceSharing = value.getBoolean("presence_sharing"), invitations = value.getJSONArray("invitations").objects().map { GroupInvitation(it.getString("id"), it.getString("peer"), it.getString("group")) })
        val chats = value.getJSONArray("chats").objects().map { chat ->
            ChatSummary(chat.getString("id"), chat.getString("address"), chat.getString("preview"), clock(chat.getLong("timestamp")), chat.getBoolean("verified"),
                chat.getJSONArray("devices").objects().map { device -> ChatDevice(device.getString("id"), device.getString("fingerprint"), device.optBoolean("identity_verified"), device.getBoolean("blocked"), device.getBoolean("changed")) }, chat.optString("name"), chat.optInt("unread"), chat.optBoolean("pinned"), chat.optBoolean("snoozed"), chat.optBoolean("hidden"), chat.optString("presence", "inactive"), chat.optJSONArray("collections")?.strings().orEmpty(), chat.optJSONArray("typing")?.strings().orEmpty(), chat.optional("draft").orEmpty(), chat.optBoolean("group"), avatar = chat.optString("avatar"), ui = pendingUi(chat.getString("id"), chat.optJSONObject("ui")?.stringMap().orEmpty()), contactOnly = chat.optBoolean("contact_only"), readReceipts = chat.optBoolean("read_receipts", true), typingIndicators = chat.optBoolean("typing_indicators", true), presenceSharing = chat.optBoolean("presence_sharing"), request = chat.optString("request", "none"), identityReview = chat.optString("identity_review").takeIf { it.isNotEmpty() && it != "null" })
        }
        state = state.copy(profileAvatar = value.optString("profile_avatar"), photoPending = value.optBoolean("photo_pending"), phase = phase, address = value.getString("address"), device = value.getString("device"), fingerprint = value.getString("fingerprint"), chats = chats, collectionsEnabled = value.optBoolean("collections_enabled"), collections = value.optJSONArray("collections")?.objects()?.map { CollectionItem(it.getString("id"), it.getString("name"), it.optString("icon", "folder")) }.orEmpty(), ui = pendingUi(null, value.optJSONObject("ui")?.stringMap().orEmpty()))
        val peer = state.selected ?: return
        if (peer.startsWith("history:") && state.chats.none { it.id == peer }) state = state.copy(chats = state.chats + ChatSummary(peer, "", "", "", false, emptyList(), displayName = "Saved conversation", archived = true))
        if (peer == "self" && state.chats.none { it.id == "self" }) state = state.copy(chats = state.chats + ChatSummary("self", state.address, "", "", true, emptyList(), displayName = "Note to Self"))
        val messages = mutableListOf<ChatMessage>()
        val filter = timelineFilter
        val initialAnchor = anchor
        val wanted = pages * 64
        var before: Long? = null
        do {
            val timeline = execute("timeline", filter + mapOf("peer" to peer, "before" to before, "author" to if (before == null) initialAnchor?.first else null, "message" to if (before == null) initialAnchor?.second else null))
            if (state.selected != peer || timelineFilter != filter || anchor != initialAnchor) return
            if (before == null && state.selected == peer) state = state.copy(typing = timeline.optJSONArray("typing")?.strings().orEmpty())
            state = state.copy(people = timeline.getJSONObject("people").stringMap())
            messages += timeline.getJSONArray("messages").objects().map { message ->
                ChatMessage(message.getString("id"), message.getString("author"), message.getString("text"), message.getBoolean("mine"), clock(message.getLong("timestamp")),
                    message.getString("delivery"), message.getBoolean("pinned"), message.getJSONArray("reactions").strings(), message.getJSONArray("my_reactions").strings(), message.optional("reply"), message.getBoolean("read_by_me"), message.getLong("timestamp"), separator(message.getLong("timestamp")), message.optJSONArray("readers")?.strings().orEmpty(), message.optBoolean("noted"), message.optional("thread_author"), message.optional("thread_message"), message.optBoolean("editable", true), message.optString("kind", "Text"), peer,
                    message.optJSONObject("attachment")?.let { AttachmentDetails(it.getString("name"), it.getString("media_type"), it.getLong("length")) },
                    message.optJSONArray("parts")?.objects()?.map { part -> MessagePart(part.optString("id"), part.getString("kind"), part.getString("text"), part.optJSONArray("items")?.objects()?.map { item -> CardItem(item.getString("id"), item.getString("text"), item.getBoolean("checked"), item.getBoolean("enabled"), if (item.isNull("count")) null else item.getLong("count"), item.richText()) }.orEmpty(), part.optBoolean("multiple"), part.optBoolean("closed"), if (part.isNull("voters")) null else part.getLong("voters"), if (part.has("at")) separator(part.getLong("at")) else "", part.optInt("latitude_e6") / 1_000_000.0, part.optInt("longitude_e6") / 1_000_000.0, part.richText()) }.orEmpty(), message.optional("thread_preview"))
            }
            before = if (timeline.isNull("next")) null else timeline.getLong("next")
            if (before == null) {
                if (state.selected == peer) state = state.copy(messages = messages, more = false)
                return
            }
            if (messages.isNotEmpty()) state = state.copy(messages = messages.toList(), more = true)
            yield()
        } while (messages.size < wanted)
        if (state.selected == peer) state = state.copy(messages = messages, more = true)
    }
    private fun storage(result: JSONObject) {
        val recovery = result.getJSONObject("recovery")
        state = state.copy(storage = StorageDetails(result.getLong("database"), result.getLong("media"), result.getLong("media_used"), result.getLong("budget"), recovery.getBoolean("enabled"),
            if (recovery.isNull("last")) null else separator(recovery.getLong("last")), recovery.optLong("pending"), if (recovery.isNull("days")) null else recovery.getInt("days"), recovery.optBoolean("restoring")))
    }
    private fun accountAccess(result: JSONObject) {
        result.optJSONObject("access")?.let { access -> state = state.copy(accountAccess = AccountAccess(access.getLong("configuration_revision"), access.getLong("transition_revision"), access.optional("issuer"), access.getBoolean("linked"), access.getBoolean("retiring"), access.getBoolean("invitation_fallback_acknowledged"), result.getBoolean("link_pending"))) }
    }
    override fun onCleared() { calls.close(); voice.close(); scope.cancel() }
    private class NativeFailure(message: String) : Exception(message)
}
private fun JSONObject.optional(key: String): String? = if (isNull(key)) null else optString(key).ifEmpty { null }
private fun JSONArray.objects() = (0 until length()).map { getJSONObject(it) }
private fun JSONArray.strings() = (0 until length()).map { getString(it) }
private fun clock(seconds: Long): String = if (seconds == 0L) "" else DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(seconds * 1000))
private fun separator(seconds: Long): String {
    if (seconds == 0L) return ""
    val time = java.time.Instant.ofEpochSecond(seconds).atZone(java.time.ZoneId.systemDefault())
    val today = java.time.LocalDate.now()
    val day = when (time.toLocalDate()) {
        today -> "Today"
        today.minusDays(1) -> "Yesterday"
        else -> time.format(java.time.format.DateTimeFormatter.ofPattern(if (time.year == today.year) "MMMM d" else "MMMM d, yyyy"))
    }
    return "$day, ${clock(seconds)}"
}

private fun JSONObject.stringMap() = keys().asSequence().associateWith { getString(it) }
