package org.sigil.compose

import android.app.Application
import android.net.Uri
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
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
    private val placeMutex = Mutex()
    private val syncWake = Channel<Unit>(Channel.CONFLATED)
    @Volatile private var forceSync = false
    private var submittingPost = false
    private var shownEarly: String? = null
    private var busyOperations = 0
    var signOutStage by mutableStateOf(NativeSignOut.stage(application))
        private set
    var signOutBusy by mutableStateOf(false)
        private set
    var signOutIssue by mutableStateOf<String?>(null)
        private set
    private val workNotices = WorkNotices()
    private var callExpanded = false
    internal val calls = NativeCalls(application, { history, active -> state = state.copy(calls = history, call = active) }, { state = state.copy(issue = it) }, { if (syncIssue != null && state.issue == syncIssue) { state = state.copy(issue = workNotices.update(state.issue, "sync", null)); syncIssue = null } })
    private val files = NativeFiles(application, scope, { uploads, sent ->
        state = state.copy(transfers = uploads)
        if (sent) { loadTimeline(); syncWake.trySend(Unit); scope.launch { serialized(false) { refresh() } } }
    }, { state = state.copy(issue = it) }, { outcomes ->
        var notice = state.issue
        for ((source, message) in outcomes) notice = workNotices.update(notice, source, fileWorkNotice(source, message))
        state = state.copy(issue = notice)
    })
    private val voice = VoiceRecorder(scope, { peer, bytes, target ->
        bytes.inputStream().use { files.stage(target + ("peer" to peer), "Voice message.aac", "audio/aac", bytes.size.toLong(), it) }
        withContext(Dispatchers.Main) { state = state.copy(sent = state.sent + 1, sentText = target["caption"] as? String, sentMessage = null) }
    }, { state = state.copy(voice = it) }, { state = state.copy(issue = it) })
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
            result.optional("open")?.let { state = state.copy(selected = it); resetPreload() }
            contactQr = result.takeUnless { it.getString("stage") in listOf("done", "none") }
            context.forEach { (k,v) -> if (v != null) contactQr?.put(k,v) }
            refresh()
        } }
    }
    var deviceLink by mutableStateOf<JSONObject?>(null)
        private set
    var deviceLinkIssue by mutableStateOf<String?>(null)
        private set
    var deviceLinkBusy by mutableStateOf(false)
        private set
    private fun linkResult(value: JSONObject) { deviceLink = value.takeUnless { it.getString("stage") == "none" } }
    private fun deviceLink(fields: Map<String, Any?>) {
        if (fields["action"] == "pause") { deviceLink = null; return }
        if(deviceLinkBusy)return
        deviceLinkBusy=true
        scope.launch { mutex.withLock {
            try {
                linkResult(execute("device_link", fields));deviceLinkIssue=null
                if(deviceLink?.optString("stage")=="done") {
                    attempt(false){refresh();NativeSync.enable(getApplication(),state.phase=="connected")}
                    if(deviceLink?.optBoolean("sponsor")==false)linkResult(execute("device_link",mapOf("action" to "close")))
                }
            }catch(cancelled:CancellationException){throw cancelled}
            catch(error:Exception){deviceLinkIssue=if(error is NativeFailure)error.message else "Could not complete this linking step. Retry when connected."}
            finally {
                try{linkResult(execute("device_link",mapOf("action" to "status")))}catch(_:Exception){}
                deviceLinkBusy=false
            }
        } }
    }
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
    private var pendingPlace: Pair<Map<String, Any?>, String>? = null
    suspend fun sharePlace(fields: Map<String, Any?>): Boolean = placeMutex.withLock {
        if (NativeSignOut.pending(getApplication()) || !foreground) return@withLock false
        val live = fields["live"] != null
        try {
            if (live) NativeLocations.prepare(getApplication())
            val raw = pendingPlace?.takeIf { it.first == fields }?.second ?: request("place", fields).also { pendingPlace = fields.toMap() to it }
            native(raw)
            pendingPlace = null
            syncWake.trySend(Unit)
            loadTimeline()
            NativeSync.enqueue(getApplication())
            scope.launch { serialized(false) { refresh() } }
            true
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { false }
        finally { if (live) NativeLocations.prepared() }
    }
    suspend fun importPhoto(target: Map<String, Any?>, bytes: ByteArray): Boolean = withContext(Dispatchers.IO) {
        try { bytes.inputStream().use { files.stage(target + ("draft" to true), "Photo.jpg", "image/jpeg", bytes.size.toLong(), it) }; true }
        catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { withContext(Dispatchers.Main) { state = state.copy(issue = "Could not prepare this photo. You can try again.") }; false }
        finally { bytes.fill(0) }
    }
    private var foreground = false
    private var nextSync = 0L
    private var nextAccess = 0L
    private var published = false
    private var syncIssue: String? = null
    private var viewportEnd = 0
    private var timelineWant = 0
    private var timelineReloadAt = Int.MAX_VALUE
    private var post: Pair<Map<String, Any?>, String>? = null
    private var groupCreate: Pair<Map<String, Any?>, String>? = null
    private var discoveryGeneration = 0L
    private var searchJob: Job? = null
    private var searchGeneration = 0L
    private var searchAfter: Long? = null
    private var searchCategory = ""
    private var anchor: Pair<String, String>? = null
    private var nextPushStatus = 0L
    private val pendingUiSettings = mutableMapOf<Pair<String?, String>, Any?>()
    private var timelineFilter: Map<String, Any?> = mapOf("category" to "Timeline")
    private var timelineJob: Job? = null
    private var viewportChase = false
    var authorizationUrl by mutableStateOf<String?>(null)
        private set

    init {
        NativeSync.foregroundWake = { forceSync = true; syncWake.trySend(Unit) }
        if (signOutStage.isEmpty()) scope.launch { serialized(false) { refresh(); linkResult(execute("device_link", mapOf("action" to "status"))) } }
        else NativeSync.enable(application, false)
        scope.launch {
            while (isActive) {
                val nudged = withTimeoutOrNull(if(foreground && state.phase=="connected") foregroundSyncWait(nextSync, System.currentTimeMillis()) else 1000) { syncWake.receive(); true } == true
                if (signOutStage.isEmpty() && foreground && state.phase == "connected" && (nudged || System.currentTimeMillis() / 1000 >= nextSync)) {
                    attempt(false) {
                        if (!published) { execute("publish"); published = true }
                        val force = forceSync; forceSync = false
                        val result = execute("sync", mapOf("interactive" to true, "wake" to force, "call_setup" to calls.settingUp))
                        nextSync = result.getLong("next_at")
                        val issue = result.optional("issue")
                        if (issue != null && issue != syncIssue) android.util.Log.i("SigilTiming", "issue: ${issue.take(200)}")
                        if(issue!=syncIssue && issue!=null)Regex("sqlite-[0-9]+(?::group-[a-z-]+)?").find(issue)?.value?.let {code->
                            val stage=Regex("Sync: (receiving messages|publishing keys|updating calls|updating invitations|updating groups|sharing history|sending retry controls|recovering sessions|starting conversations|sending messages|sending group messages)").find(issue)?.groupValues?.get(1) ?: if(issue.startsWith("Contact sync:"))"contact sync" else "scheduling"
                            android.util.Log.w("SigilStorage","$stage: $code")
                        }
                        if (issue != null || result.getBoolean("ran")) {
                            state = state.copy(issue = workNotices.update(state.issue, "sync", issue))
                            syncIssue = issue
                        }
                        if ((result.getBoolean("ran") && result.optBoolean("changed", true)) || nudged) serialized(false) { refresh() }
                        NativeSync.presence(getApplication(), state.call?.call?.phase in listOf("active", "joining"))
                    }
                    // Rust also persists backoff; an IO failure must not create a busy loop.
                    if (syncIssue != null) nextSync = maxOf(nextSync, System.currentTimeMillis() / 1000 + 1)
                }
            }
        }
        // Server-held mailbox wait: new mail wakes a sync immediately instead of waiting for the poll.
        scope.launch {
            var fast = 0
            while (isActive) {
                if (!(signOutStage.isEmpty() && foreground && state.phase == "connected")) { delay(500); continue }
                val started = System.currentTimeMillis()
                val hit = try { withContext(Dispatchers.IO) { StorageKeyProvider(getApplication()).withKey { directory, key -> NativeStorage.mailboxWait(directory.path, key, 25) } } }
                catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { false }
                // Re-arm at once; only a run of instant returns (mail nobody acknowledges, or errors) waits.
                fast = if (System.currentTimeMillis() - started < 300) fast + 1 else 0
                if (hit) { forceSync = true; syncWake.trySend(Unit); if (fast >= 3) delay(1000) }
                else if (fast >= 3) delay(2000)
            }
        }
    }
    fun foreground(value: Boolean) {
        if (NativeSignOut.pending(getApplication())) { foreground = false; files.enabled = false; NativeSync.enable(getApplication(), false); return }
        NativeSync.foreground(value)
        NativeNotifications.foreground(getApplication(), value)
        if (value) state = state.copy(notifications = NativeNotifications.settings(getApplication()))
        foreground = value; files.enabled = value && state.phase == "connected"
        // Out of sight, the pooled stores and the key they hold are dropped; the next command unwraps it again.
        if (!value) NativeStorage.closeStore()
        NativeSync.enable(getApplication(), state.phase == "connected")
        if (value) { nextSync = 0; published = false; syncWake.trySend(Unit); scope.launch { serialized(false) { refresh() } } }
        else { if (state.voice.phase == "Recording") voice.stop(); voice.pausePreview(); if (state.phase == "connected") NativeSync.enqueue(getApplication()) }
        if (state.phase == "connected") scope.launch { try { NativeSync.presence(getApplication(), state.call?.call?.phase in listOf("active", "joining")) } catch (cancelled: CancellationException) { throw cancelled } catch (_: Exception) { } }
    }
    fun browserOpened() { authorizationUrl = null }
    /** Notification taps: open a conversation or answer a ringing call once the account is connected. */
    fun handleNotificationIntent(intent: android.content.Intent?) {
        val peer = intent?.getStringExtra(NativeNotifications.EXTRA_PEER)
        val answer = intent?.getStringExtra(NativeNotifications.EXTRA_ANSWER)
        if (peer == null && answer == null) return
        intent.removeExtra(NativeNotifications.EXTRA_PEER); intent.removeExtra(NativeNotifications.EXTRA_ANSWER)
        scope.launch {
            withTimeoutOrNull(15000) { while (state.phase != "connected") delay(100) } ?: return@launch
            when {
                answer != null -> calls.command("call_answer", mapOf("call" to answer, "name" to (state.calls.firstOrNull { it.id == answer }?.name ?: "")))
                peer == "calls" -> {}
                peer != null -> command("open", mapOf("peer" to peer))
            }
        }
    }
    fun callback(uri: Uri?) {
        if (uri == null || uri.scheme != "sigil" || uri.host != "oidc" || uri.query != null || uri.fragment != null) return
        val parts = uri.pathSegments
        if (parts.size != 2) return
        command("callback", mapOf("request_id" to parts[0], "completion" to parts[1]))
    }
    fun command(name: String, fields: Map<String, Any?>) {
        if (NativeSignOut.pending(getApplication())) return
        if (name == "call_display") {
            val expanded = fields["expanded"] == true
            if (expanded != callExpanded) {
                callExpanded = expanded
                if (expanded) timelineJob?.cancel() else loadTimeline()
            }
            return
        }
        if (name.startsWith("call_")) { calls.command(name, fields + ("name" to (fields["peer"] as? String)?.let { peer -> state.chats.find { it.id == peer }?.name })); return }
        when (name) {
            "edit_source_used" -> { state = state.copy(editDraft = null); return }
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
                        withContext(Dispatchers.IO) { NativeFcm.stop(getApplication()); NativePush.unregister(getApplication(), old) }
                    } else {
                        val distributor = fields["distributor"] as String
                        if (distributor == NativeFcm.ID) {
                            withContext(Dispatchers.IO) { NativeFcm.register(getApplication(), true) }
                            state = state.copy(push = withContext(Dispatchers.IO) { NativePush.settings(getApplication()) })
                            return@serialized
                        }
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
            "notification_full_screen" -> { NativeNotifications.fullScreenSettings(getApplication()); return }
            "notification_battery" -> { NativeNotifications.batterySettings(getApplication()); return }
            "notification_content" -> { NativeNotifications.content(getApplication(), fields["level"] as String); notificationPermissionResult(); return }
            "record_start" -> { val peer = fields["peer"] as String; if (getApplication<Application>().checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) == android.content.pm.PackageManager.PERMISSION_GRANTED) voice.start(peer, fields) else microphoneRequest = peer to fields.toMap(); return }
            "record_stop" -> { voice.stop(); return }
            "record_cancel" -> { voice.discard(); return }
            "record_send" -> { voice.send(fields["caption"] as? String ?: ""); return }
            "record_preview" -> { voice.playPreview(); return }
            "record_pause" -> { voice.pauseRecording(); return }
            "record_seek" -> { voice.seek((fields["position"] as Number).toLong()); return }
            "attachment_pick" -> { picker = fields.filterKeys { it != "kind" } to (fields["kind"] as String); return }
            "delete_conversation" -> {
                scope.launch { serialized(true) {
                    if (fields["leave"] == true) execute("leave_group", mapOf("peer" to fields["peer"]))
                    execute("clear_conversation", mapOf("peer" to fields["peer"]))
                    if (state.selected == fields["peer"]) { state = state.copy(selected = null, messages = emptyList()); anchor = null; resetPreload() }
                    refresh()
                    NativeSync.enqueue(getApplication())
                } }
                return
            }
            "file_cancel" -> { files.cancel(fields["request"] as String); return }
            "file_send" -> { val caption = fields["caption"] as String; files.send(fields["request"] as String, caption) { state = state.copy(sent = state.sent + 1, sentText = caption, sentMessage = null) }; return }
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
            "dismiss" -> { workNotices.dismiss(); state = state.copy(issue = null); return }
            "close" -> { timelineJob?.cancel(); state = state.copy(selected = null, messages = emptyList(), historical = false, timelineLoaded=false); anchor = null; resetPreload(); return }
            "open" -> {
                anchor = (fields["author"] as? String)?.let { author -> (fields["message"] as? String)?.let { author to it } }
                val target = (fields["thread_author"] as? String)?.let { author -> (fields["thread_message"] as? String)?.let { ThreadTarget(author, it) } }
                timelineFilter = mapOf("category" to (fields["category"] as? String ?: "Timeline")) + if (target == null) emptyMap() else mapOf("thread_author" to target.author, "thread_message" to target.id)
                // A conversation seen before opens on what it last showed, then refreshes beneath it.
                val peer = fields["peer"] as String
                val recalled = if (anchor == null && target == null && timelineFilter == defaultTimelineFilter) recentTimelines[peer] else null
                state = state.copy(selected = peer, messages = recalled.orEmpty(), historical = anchor != null, threadTarget = target, timelineLoaded = recalled != null); resetPreload()
            }
            "timeline_filter" -> {
                val filter = fields.filter { (key, value) -> key != "peer" && value != null }
                if (timelineFilter == filter) return
                timelineFilter = filter; resetPreload(); state = state.copy(messages = emptyList(),timelineLoaded=false)
            }
            // Clearing history here as every other reset does: a viewport reported against it would re-deepen the target.
            "latest" -> { anchor = null; state = state.copy(messages = emptyList(), historical = false,timelineLoaded=false); resetPreload() }
            "older" -> viewportEnd = maxOf(viewportEnd, timelineWant)
            // The list only reports where it is looking; the core decides when that needs more history.
            "viewport" -> { reportViewport(fields["peer"] as? String, (fields["end"] as? Number)?.toInt() ?: 0); return }
            "read" -> if (!foreground) return
        }
        if (name in listOf("open", "older", "latest", "timeline_filter")) {
            loadTimeline()
            if (name != "open") return
        }
        val setting = (fields["value"] as? Map<*, *>)?.get("UiSetting") as? Map<*, *>
        val preference = if (name == "organize") (setting?.get("key") as? String)?.let { (fields["peer"] as? String) to it } else null
        if (preference != null) pendingUiSettings[preference] = setting?.get("value")
        NativeSync.lastInteraction = android.os.SystemClock.elapsedRealtime()
        // A reaction shows on the message at the tap; the stored one confirms it.
        if (name == "react") {
            val emoji = fields["emoji"] as? String; val active = fields["active"] as? Boolean
            if (emoji != null && active != null) state = state.copy(messages = state.messages.map { m ->
                if (m.id != fields["message"] || m.author != fields["author"]) m
                else m.copy(myReactions = if (active) (m.myReactions + emoji).distinct() else m.myReactions - emoji,
                    reactions = if (active) m.reactions + emoji else m.reactions.toMutableList().also { it.remove(emoji) })
            })
        }
        if (name == "post") {
            if (submittingPost) return
            submittingPost = true
            state = state.copy(busy = true)
            // A plain message is on screen and the composer is clear before any work runs; the stored message lands on the same key.
            val text = fields["text"] as? String; val peer = fields["peer"] as? String
            val author = state.messages.firstOrNull { it.mine }?.author
            if (text != null && peer != null && author != null && "::" !in text && post?.first != fields && state.selected == peer && !state.historical && state.threadTarget == null) {
                val raw = request(name, fields); post = fields.toMap() to raw
                val requestId = JSONObject(raw).getString("request"); val now = System.currentTimeMillis() / 1000
                val shown = ChatMessage(requestId, author, text, true, clock(now), "Sending", false, emptyList(), emptyList(), fields["reply_message"] as? String, true, timestamp = now, peer = peer)
                state = state.copy(messages = listOf(shown) + state.messages, sent = state.sent + 1, sentText = text, sentMessage = requestId)
                recentTimelines[peer] = state.messages
                shownEarly = requestId
            }
        }
        scope.launch {
            try {
            commandWork(preference == null && name !in listOf("read", "typing", "draft", "open"), local = name == "post") {
                if (preference != null) {
                    if (pendingUiSettings[preference] != setting?.get("value")) return@commandWork
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
                    if (name !in setOf("draft","profile","devices","storage","edit_source")) syncWake.trySend(Unit)
                    if (name == "cancel_login") recoveryPreference(false)
                    accountAccess(result)
                    result.optJSONObject("forward_file")?.let { files.forward(fields, it) }
                    if (name == "oidc_account") nextAccess = 0
                    result.optional("open")?.let { state = state.copy(selected = it); groupCreate = null; resetPreload() }
                    if (name == "edit_source" && state.selected == fields["peer"]) state = state.copy(editDraft = EditDraft(fields["peer"] as String, fields["author"] as String, fields["message"] as String, result.getString("edit_source")))
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
                    if (name in listOf("post", "edit")) {
                        post = null
                        val requestId = if (name == "post") JSONObject(raw).getString("request") else null
                        if (shownEarly == requestId) shownEarly = null
                        else state = state.copy(sent = state.sent + 1, sentText = fields["text"] as? String, sentMessage = requestId)
                        loadTimeline()
                    }
                    // Send after the timeline shows the message; the flush pass reports only its own issue.
                    if (name == "post") { val flush = native(request("flush")); android.util.Log.i("SigilTiming", "flush result ${flush.optJSONArray("outbound")}"); if (flush.getInt("sent") > 0) loadTimeline(); flush.optString("issue").takeIf { it.isNotEmpty() && !flush.isNull("issue") }?.let { syncIssue = it; state = state.copy(issue = workNotices.update(state.issue, "sync", it)) } }
                }
                // Typing and draft commands change nothing the inbox shows; everything else refreshes the snapshot once.
                if (name !in listOf("post", "typing", "draft")) refresh()
                if (preference != null && pendingUiSettings[preference] == setting?.get("value")) pendingUiSettings.remove(preference)
            }
            } finally { if (name == "post") { submittingPost = false; state = state.copy(busy = busyOperations > 0) } }
        }
    }
    private fun search(query: String, category: String, more: Boolean = false) {
        searchJob?.cancel()
        val generation = ++searchGeneration
        val previous = state.searchHits
        val sameSearch = state.searchQuery == query && searchCategory == category
        if (!more) searchAfter = null
        searchCategory = category
        state = state.copy(searchQuery = query, searchHits = if (more || sameSearch) previous else emptyList(), searching = true)
        searchJob = scope.launch {
            try {
                var after = searchAfter
                val hits = if (more) previous.toMutableList() else mutableListOf<SearchHit>()
                val initialSize = hits.size
                do {
                    val result = mutex.withLock { execute("search", mapOf("query" to query, "after" to after, "category" to category)) }
                    if (generation != searchGeneration) return@launch
                    hits += result.getJSONArray("hits").objects().map { hit -> SearchHit(hit.getString("peer"), hit.getString("id"), hit.getString("author"), hit.getString("text"), clock(hit.getLong("timestamp")), hit.getBoolean("pinned"), hit.getBoolean("noted"), hit.getString("kind"), hit.getBoolean("thread"), hit.optional("thread_author")?.let { author -> hit.optional("thread_message")?.let { ThreadTarget(author, it) } }) }
                    after = if (result.isNull("next")) null else result.getLong("next")
                    searchAfter = after
                    state = state.copy(searchHits = if (sameSearch && !more && hits.isEmpty() && after != null) previous else hits.toList(), searchMore = after != null, searching = after != null && hits.size - initialSize < 64)
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
    private suspend fun commandWork(progress: Boolean, local: Boolean, work: suspend () -> Unit) =
        if (local) attempt(progress, work) else serialized(progress, work)
    private suspend fun serialized(progress: Boolean, work: suspend () -> Unit) = mutex.withLock { attempt(progress, work) }
    private suspend fun attempt(progress: Boolean, work: suspend () -> Unit) {
        if (NativeSignOut.pending(getApplication())) return
        if (progress) { busyOperations++; state = state.copy(busy = true, issue = null) }
        try { work() } catch (cancelled: CancellationException) { throw cancelled }
        catch (error: Exception) {
            val issue = if (error is NativeFailure) error.message else "Cannot access this device's storage. Stored keys have not been reset."
            if (progress) {
                try { refresh() } catch (cancelled: CancellationException) { throw cancelled } catch (_: Exception) { }
            }
            if (!progress) syncIssue = issue
            state = state.copy(issue = if (progress) issue else workNotices.update(state.issue, "sync", issue))
        } finally { if (progress) { busyOperations--; state = state.copy(busy = busyOperations > 0 || submittingPost) } }
    }
    private fun request(name: String, fields: Map<String, Any?> = emptyMap()): String {
        val value = JSONObject().put("command", name)
        fields.forEach { (key, item) -> value.put(key, if(key=="shared_contact" && item is String)JSONObject(item)else JSONObject.wrap(item)) }
        if (name in listOf("oidc", "enroll")) value.put("label", android.os.Build.MODEL.take(60))
        if (name == "card_action") value.put("timestamp", System.currentTimeMillis() / 1000)
        if (name in listOf("post", "place", "group_create", "react", "pin", "read", "mark_read", "snooze", "forward", "organize", "edit", "delete", "clear_conversation", "note", "typing", "draft")) {
            val bytes = ByteArray(32).also { SecureRandom().nextBytes(it) }
            value.put("request", bytes.joinToString("") { "%02x".format(it) })
            value.put("timestamp", System.currentTimeMillis() / 1000)
            if (name == "post" && value.isNull("timezone")) value.put("timezone", java.util.TimeZone.getDefault().id)
        }
        return value.toString()
    }
    private suspend fun execute(name: String, fields: Map<String, Any?> = emptyMap()) = if (name == "sync") NativeSync.run(getApplication(), fields["interactive"] == true, fields["call_setup"] == true, wake = fields["wake"] == true) else native(request(name, fields))
    suspend fun serviceRequest(raw:String):ServiceResponse=mutex.withLock {
        check(foreground && !NativeSignOut.pending(getApplication()))
        val result=native(raw)
        ServiceResponse(result.toString(),result.optJSONObject("preview")?.let(::previewPart))
    }
    suspend fun recipeView(message: ChatMessage, part: MessagePart, serves: Int): RecipeContent {
        check(!NativeSignOut.pending(getApplication()))
        return execute("recipe_view", mapOf("peer" to message.peer, "author" to message.author, "message" to message.id, "card" to part.id, "serves" to serves)).recipeContent() ?: error("Recipe unavailable")
    }
    private suspend fun native(raw: String): JSONObject = withContext(Dispatchers.IO) {
        val started = android.os.SystemClock.elapsedRealtime()
        var keyed = 0L
        val provider = StorageKeyProvider(getApplication())
        // The pooled store answers without the key; the first command after a cold or backgrounded start unwraps it once.
        val result = NativeStorage.executeCached(provider.directory.path, raw)?.let { JSONObject(it) } ?: provider.withKey { directory, key ->
            keyed = android.os.SystemClock.elapsedRealtime() - started
            JSONObject(NativeStorage.execute(directory.path, key, raw))
        }
        // Durations only; command names are not message content.
        android.util.Log.i("SigilTiming", "${JSONObject(raw).optString("command")} ${android.os.SystemClock.elapsedRealtime() - started}ms key=${keyed}ms")
        if (!result.getBoolean("ok")) throw NativeFailure(result.getString("error"))
        result.getJSONObject("value")
    }
    private fun pendingUi(peer: String?, stored: Map<String, String>) = stored + pendingUiSettings.mapNotNull { (key, value) ->
        if (key.first == peer && value is String) key.second to value else null
    }
    // Refreshes coalesce: a request that arrives while one is running is answered by a single run after it.
    private var refreshing = false
    private var refreshAgain = false
    private suspend fun refresh() {
        if (refreshing) { refreshAgain = true; return }
        refreshing = true
        try { do { refreshAgain = false; refreshOnce() } while (refreshAgain) } finally { refreshing = false }
    }
    private suspend fun refreshOnce() {
        val value = execute("state", mapOf("calls" to true))
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
        calls.refresh(value.getJSONObject("call_state"))
        state = state.copy(readReceipts = value.getBoolean("read_receipts"), typingIndicators = value.getBoolean("typing_indicators"), presenceSharing = value.getBoolean("presence_sharing"), invitations = value.getJSONArray("invitations").objects().map { GroupInvitation(it.getString("id"), it.getString("peer"), it.getString("group")) })
        val chats = value.getJSONArray("chats").objects().map { chat ->
            ChatSummary(chat.getString("id"), chat.getString("address"), chat.getString("preview"), clock(chat.getLong("timestamp")), chat.getBoolean("verified"),
                chat.getJSONArray("devices").objects().map { device -> ChatDevice(device.getString("id"), device.getString("fingerprint"), device.optBoolean("identity_verified"), device.getBoolean("blocked"), device.getBoolean("changed")) }, chat.optString("name"), chat.optInt("unread"), chat.optBoolean("pinned"), chat.optBoolean("snoozed"), chat.optBoolean("hidden"), chat.optString("presence", "inactive"), chat.optJSONArray("collections")?.strings().orEmpty(), chat.optJSONArray("typing")?.strings().orEmpty(), chat.optional("draft").orEmpty(), chat.optBoolean("group"), avatar = chat.optString("avatar"), ui = pendingUi(chat.getString("id"), chat.optJSONObject("ui")?.stringMap().orEmpty()), contactOnly = chat.optBoolean("contact_only"), readReceipts = chat.optBoolean("read_receipts", true), typingIndicators = chat.optBoolean("typing_indicators", true), presenceSharing = chat.optBoolean("presence_sharing"), request = chat.optString("request", "none"), identityReview = chat.optString("identity_review").takeIf { it.isNotEmpty() && it != "null" })
        }
        state = state.copy(profileAvatar = value.optString("profile_avatar"), photoPending = value.optBoolean("photo_pending"), phase = phase, address = value.getString("address"), device = value.getString("device"), fingerprint = value.getString("fingerprint"), chats = chats, collectionsEnabled = value.optBoolean("collections_enabled"), collections = value.optJSONArray("collections")?.objects()?.map { CollectionItem(it.getString("id"), it.getString("name"), it.optString("icon", "folder")) }.orEmpty(), ui = pendingUi(null, value.optJSONObject("ui")?.stringMap().orEmpty()))
        if (timelineJob?.isActive != true) loadTimeline()
    }
    private fun resetPreload() { viewportEnd = 0; timelineWant = 0; timelineReloadAt = Int.MAX_VALUE; viewportChase = false }
    private fun reportViewport(peer: String?, end: Int) {
        if (peer != null && peer != state.selected) return
        if (end <= viewportEnd) return
        viewportEnd = end
        if (!state.more || end < timelineReloadAt) return
        // A running load reads the viewport again on every page; restarting it would throw that page away.
        if (timelineJob?.isActive == true) { viewportChase = true; return }
        loadTimeline()
    }
    // The messages each conversation last showed, so reopening it is immediate.
    private val recentTimelines = HashMap<String, List<ChatMessage>>()
    private val defaultTimelineFilter = mapOf("category" to "Timeline")
    private fun loadTimeline() {
        if (callExpanded) return
        val peer = state.selected ?: return
        val filter = timelineFilter; val initialAnchor = anchor
        viewportChase = false
        timelineJob?.cancel()
        timelineJob = scope.launch {
            try { refreshTimeline(peer, filter, initialAnchor) }
            catch (cancelled: CancellationException) { throw cancelled }
            catch (error: Exception) { if (state.selected == peer) state = state.copy(issue = if (error is NativeFailure) error.message else "Cannot read this conversation from device storage.") }
        }
    }
    private suspend fun refreshTimeline(peer: String, filter: Map<String, Any?>, initialAnchor: Pair<String, String>?) {
        if (NativeSignOut.pending(getApplication())) return
        val started = android.os.SystemClock.elapsedRealtime()
        // One more scan only if the viewport went deeper than the last page could see.
        try { do { viewportChase = false; refreshTimelinePages(peer, filter, initialAnchor) } while (viewportChase && state.selected == peer && state.more && viewportEnd >= timelineReloadAt) }
        finally { android.util.Log.i("SigilTiming", "timeline ${android.os.SystemClock.elapsedRealtime() - started}ms held=${state.messages.size} want=$timelineWant visible=$viewportEnd") }
    }
    private suspend fun refreshTimelinePages(peer: String, filter: Map<String, Any?>, initialAnchor: Pair<String, String>?) {
        if (peer.startsWith("history:") && state.chats.none { it.id == peer }) state = state.copy(chats = state.chats + ChatSummary(peer, "", "", "", false, emptyList(), displayName = "Saved conversation", archived = true))
        if (peer == "self" && state.chats.none { it.id == "self" }) state = state.copy(chats = state.chats + ChatSummary("self", state.address, "", "", true, emptyList(), displayName = "Note to Self"))
        val messages = mutableListOf<ChatMessage>()
        // Publishing fewer messages than are already on screen would drop the reader's place.
        val onScreen = if (state.selected == peer) state.messages.size else 0
        var before: Long? = null
        do {
            val timeline = execute("timeline", filter + mapOf("peer" to peer, "before" to before, "author" to if (before == null) initialAnchor?.first else null, "message" to if (before == null) initialAnchor?.second else null, "visible_end" to viewportEnd))
            if (state.selected != peer || timelineFilter != filter || anchor != initialAnchor) return
            if (before == null) {
                state = state.copy(typing = timeline.optJSONArray("typing")?.strings().orEmpty())
            }
            // Read on every page, so a viewport that deepens mid-load extends this scan instead of restarting it.
            timelineWant = timeline.getInt("want"); timelineReloadAt = timeline.getInt("reload_at")
            val buffer = TimelineBufferDepth(timeline.getInt("cache_ahead") / 10f, timeline.getInt("cache_behind") / 10f)
            state = state.copy(people = timeline.getJSONObject("people").stringMap())
            val decodePart = { part: org.json.JSONObject -> MessagePart(part.optString("id"), part.getString("kind"), part.getString("text"), part.optJSONArray("items")?.objects()?.map { item -> CardItem(item.getString("id"), item.getString("text"), item.getBoolean("checked"), item.getBoolean("enabled"), if (item.isNull("count")) null else item.getLong("count"), item.richText(), item.optBoolean("persistent")) }.orEmpty(), part.optBoolean("multiple"), part.optBoolean("closed"), if (part.isNull("voters")) null else part.getLong("voters"), if (part.has("date")) part.getString("date") else if (part.has("at")) separator(part.getLong("at")) else "", part.optInt("latitude_e6") / 1_000_000.0, part.optInt("longitude_e6") / 1_000_000.0, part.richText(), part.optString("location_mode", "pin"), part.optLong("sampled_at"), if (part.isNull("accuracy_cm")) null else part.getLong("accuracy_cm"), if (part.isNull("until")) null else part.getLong("until"), part.optBoolean("stopped"), part.optBoolean("can_stop"), part.tableContent(), part.recipeContent(), part.chartContent(), part.diagramContent(), part.utilityContent(), part.serviceContent(), part.contactContent(),part.optLong("at"),part.optLong("started_at")) }
            messages += timeline.getJSONArray("messages").objects().map { message ->
                ChatMessage(message.getString("id"), message.getString("author"), message.getString("text"), message.getBoolean("mine"), clock(message.getLong("timestamp")),
                    message.getString("delivery"), message.getBoolean("pinned"), message.getJSONArray("reactions").strings(), message.getJSONArray("my_reactions").strings(), message.optional("reply"), message.getBoolean("read_by_me"), message.getLong("timestamp"), separator(message.getLong("timestamp")), message.optJSONArray("readers")?.strings().orEmpty(), message.optBoolean("noted"), message.optional("thread_author"), message.optional("thread_message"), message.optBoolean("editable", true), message.optString("kind", "Text"), peer,
                    message.optJSONObject("attachment")?.let { AttachmentDetails(it.getString("name"), it.getString("media_type"), it.getLong("length"), it.optString("caption")) },
                    message.optJSONArray("parts")?.objects()?.map(decodePart).orEmpty(), message.optional("thread_preview"), message.optional("reply_author"), message.optBoolean("reply_mine"), message.optional("reply_message"), message.optJSONObject("reply_attachment")?.let { AttachmentDetails(it.getString("name"), it.getString("media_type"), it.getLong("length"), it.optString("caption")) }, message.optJSONArray("reply_parts")?.objects()?.map(decodePart).orEmpty())
            }
            before = if (timeline.isNull("next")) null else timeline.getLong("next")
            if (timelinePublishes(messages.size, onScreen, timelineWant, before == null)) {
                state = state.copy(selected = peer, messages = messages.toList(), more = before != null,timelineLoaded=true,timelineBuffer=buffer)
                if (initialAnchor == null && filter == defaultTimelineFilter) recentTimelines[peer] = state.messages
            } else if (messages.isNotEmpty()) {
                // The first page lands at once over what is on screen: its rows replace their counterparts and the rest stay until the next page.
                val tail = state.messages.indexOfFirst { it.id == messages.last().id && it.author == messages.last().author }
                if (tail >= 0) state = state.copy(messages = messages.toList() + state.messages.drop(tail + 1), timelineLoaded = true)
            }
            if (before == null) return
            yield()
        } while (messages.size < timelineWant)
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
