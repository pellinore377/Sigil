package org.sigil.compose

import android.Manifest
import android.app.Application
import android.content.Intent
import android.content.pm.PackageManager
import android.media.AudioManager
import android.os.SystemClock
import androidx.compose.runtime.*
import kotlinx.coroutines.*
import org.json.JSONObject
import org.sigil.*
import org.sigil.storage.NativeStorage
import org.sigil.storage.StorageKeyProvider
import java.nio.ByteBuffer
import java.security.SecureRandom

internal class NativeCalls(private val app: Application, private val update: (List<CallSummary>, ActiveCall?) -> Unit, private val issue: (String) -> Unit) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var desired: String? = null
    private var history = emptyList<CallSummary>()
    private var visible: ActiveCall? = null
    private var name = ""
    private var video = false
    private var cameraReady = false
    private var starting = false
    val occupied get() = starting || desired != null || ending?.isActive == true || permissions != null
    private var closing = false
    private var muted = false
    private var loud = false
    private var started = 0L
    private var generation = 0L
    private var serviceReady = CompletableDeferred<Unit>()
    private var control: Job? = null
    private var ending: Job? = null
    private var media: Job? = null
    @Volatile private var token = 0L
    private var microphone: CallMicrophone? = null
    private var camera: CallCamera? = null
    private var screen: CallScreen? = null
    private var sharing = false
    var projectionRequest by mutableStateOf<String?>(null)
        private set
    private var front = true
    private val videoOutputs = java.util.concurrent.ConcurrentHashMap<String, CallVideoDecoder>()
    private val speakers = mutableMapOf<String, CallSpeaker>()
    private var levels = emptyMap<String, Float>()
    private val audio = app.getSystemService(AudioManager::class.java)
    private var previousAudioMode = AudioManager.MODE_NORMAL
    var permissions by mutableStateOf<Pair<String, Map<String, Any?>>?>(null)
        private set
    fun permissionResult(granted: Boolean) { val pending = permissions; permissions = null; if (granted && pending != null) command(pending.first, pending.second) else if (!granted) issue("Allow microphone access to join the call.") }
    fun ready() { serviceReady.complete(Unit) }
    fun foregroundTypes() = android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE or (if (video) android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA else 0) or (if (sharing) android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION else 0)
    fun projectionResult(id: String?, data: Intent?) {
        projectionRequest = null
        if (data == null || id == null || desired != id || sharing) return
        sharing = true
        scope.launch {
            try {
                serviceReady = CompletableDeferred(); app.startForegroundService(Intent(app, CallService::class.java)); withTimeout(5000) { serviceReady.await() }
                if (desired != id || !sharing) return@launch
                val projection = requireNotNull(app.getSystemService(android.media.projection.MediaProjectionManager::class.java).getMediaProjection(android.app.Activity.RESULT_OK, data))
                try { screen = CallScreen(app, projection, { timestamp, keyframe, bytes -> send(2, timestamp, keyframe, bytes); videoOutputs["self:2"]?.offer(timestamp, keyframe, bytes) }, { scope.launch { stopScreen() } }) }
                catch (error: Exception) { projection.stop(); throw error }
                applyTracks()
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { stopScreen(); issue("Could not share the screen.") }
        }
    }
    private fun stopScreen() { sharing = false; projectionRequest = null; screen?.close(); screen = null; applyTracks() }
    fun videoOutput(member: String, screen: Boolean, decoder: CallVideoDecoder?) { val key = "$member:${if (screen) 2 else 1}"; if (decoder == null) videoOutputs.remove(key) else videoOutputs[key] = decoder }
    fun refresh(value: JSONObject) {
        val array = value.getJSONArray("calls")
        history = (0 until array.length()).map { index ->
            val call = array.getJSONObject(index); val people = call.getJSONArray("participants")
            CallSummary(call.getString("id"), call.getString("phase"), call.getBoolean("direct"), call.getLong("created"), (0 until people.length()).map { i ->
                val person = people.getJSONObject(i)
                CallParticipant(person.getString("member"), person.getString("peer"), person.getString("name"), person.getBoolean("own"), person.getBoolean("verified"), person.getBoolean("audio"), person.getBoolean("camera"), person.getBoolean("screen"), person.getString("fingerprint"))
            }, call.getBoolean("can_invite"), call.optString("name"), call.optBoolean("outgoing"), java.text.DateFormat.getDateTimeInstance(java.text.DateFormat.MEDIUM, java.text.DateFormat.SHORT).format(java.util.Date(call.getLong("created") * 1000)))
        }
        val current = desired?.let { id -> history.find { it.id == id } } ?: history.firstOrNull { it.phase == "ringing" }
        visible = current?.let { call ->
            ActiveCall(call, name.takeIf { desired == call.id && it.isNotBlank() } ?: call.participants.filter { !it.own }.joinToString(", ") { it.name }.ifEmpty { "Call" },
                visible?.connection?.takeIf { visible?.call?.id == call.id } ?: "connecting", if (started == 0L) 0 else (SystemClock.elapsedRealtime() - started) / 1000, muted, loud, video, screen = sharing, levels = levels)
        }
        emit()
    }
    private fun emit() { update(history, visible) }
    fun command(action: String, fields: Map<String, Any?>) {
        if (NativeSignOut.pending(app)) return
        when (action) {
            "call_start", "call_redial", "call_answer", "call_resume" -> {
                val needsCamera = fields["video"] == true
                if (app.checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED || needsCamera && app.checkSelfPermission(Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) { permissions = action to fields; return }
                if (desired != null || starting) { issue("End the current call before starting another."); return }
                if (ending?.isActive == true) { issue("Finishing the previous call. Try again shortly."); return }
                starting = true
                generation++; val current = generation
                video = needsCamera; muted = false; loud = needsCamera; started = 0; name = fields["name"] as? String ?: ""
                scope.launch {
                    try {
                        val id = if (action == "call_start" || action == "call_redial") {
                            val request = ByteArray(32).also { SecureRandom().nextBytes(it) }.joinToString("") { "%02x".format(it) }
                            native(action, mapOf((if (action == "call_start") "peer" else "call") to fields[if (action == "call_start") "peer" else "call"], "request" to request, "timestamp" to System.currentTimeMillis() / 1000)).getString("call")
                        } else (fields["call"] as String).also { if (action == "call_answer") native("call_answer", mapOf("call" to it, "accept" to true)) }
                        if (generation != current) { end(id); return@launch }
                        desired = id
                        serviceReady = CompletableDeferred()
                        CallService.owner = this@NativeCalls
                        app.startForegroundService(Intent(app, CallService::class.java))
                        withTimeout(5000) { serviceReady.await() }; cameraReady = video
                        previousAudioMode = audio.mode; audio.mode = AudioManager.MODE_IN_COMMUNICATION; audio.isSpeakerphoneOn = loud
                        refresh(native("calls"))
                        control = scope.launch { controlLoop(id, current) }
                    } catch (cancelled: CancellationException) { throw cancelled }
                    catch (_: Exception) { issue("Could not start the call. Check connectivity and device verification."); end() }
                    finally { starting = false; if (closing && ending?.isActive != true) scope.cancel() }
                }
            }
            "call_decline" -> scope.launch { try { refresh(native("call_answer", mapOf("call" to fields["call"], "accept" to false))) } catch (_: Exception) { issue("Could not decline the call yet.") } }
            "call_end" -> end(fields["call"] as? String)
            "call_mute" -> { muted = !muted; microphone?.muted = muted; applyTracks() }
            "call_speaker" -> { loud = !loud; audio.isSpeakerphoneOn = loud; visible = visible?.copy(speaker = loud); emit() }
            "call_camera" -> {
                if (!video && app.checkSelfPermission(Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) { permissions = action to mapOf("video" to true); return }
                video = !video; cameraReady = false; camera?.close(); camera = null
                scope.launch {
                    try {
                        if (desired != null) { serviceReady = CompletableDeferred(); app.startForegroundService(Intent(app, CallService::class.java)); withTimeout(5000) { serviceReady.await() }; cameraReady = video }
                        applyTracks()
                    } catch (cancelled: CancellationException) { throw cancelled }
                    catch (_: Exception) { video = false; cameraReady = false; applyTracks(); issue("Could not enable the camera.") }
                }
            }
            "call_flip" -> { front = !front; camera?.close(); camera = null }
            "call_screen" -> { if (sharing) stopScreen() else desired?.let { projectionRequest = it } }
            "call_invite" -> scope.launch { try { refresh(native("call_invite", fields)); issue("Call invitation queued.") } catch (_: Exception) { issue("Could not invite this person. Check their device verification.") } }
        }
    }
    private suspend fun controlLoop(id: String, current: Long) {
        while (scope.isActive && desired == id && generation == current) {
            try {
                val result = native("sync", mapOf("interactive" to true))
                refresh(native("calls"))
                val call = history.find { it.id == id }
                if (call == null || call.phase in listOf("ended", "left", "declined")) { end(); return }
                NativeSync.presence(app, true)
                if (call.phase == "active" && media == null) media = scope.launch { mediaLoop(id, current) }
                val until = (result.getLong("next_at") * 1000 - System.currentTimeMillis()).coerceIn(1000, 300000)
                delay(until)
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { visible = visible?.copy(connection = "reconnecting"); emit(); delay(5000) }
        }
    }
    private suspend fun mediaLoop(id: String, current: Long) {
        var retry = 1000L
        try {
            while (scope.isActive && desired == id && generation == current) {
                var enabled = bits()
                val handle = withContext(Dispatchers.IO) { keys { directory, key -> NativeStorage.openCall(directory, key, id.chunked(2).map { it.toInt(16).toByte() }.toByteArray(), enabled) } }
                if (desired != id || generation != current) { NativeStorage.closeCall(handle); return }
                if (handle <= 0L) { delay(if (handle < 0) (-handle).coerceAtMost(86400) * 1000 else retry); retry = (retry * 2).coerceAtMost(30000); continue }
                token = handle
                var disconnectedAt = 0L
                try {
                    while (scope.isActive && desired == id && generation == current) {
                        val status = withContext(Dispatchers.IO) { NativeStorage.callState(handle) }
                        if (status == -1 || status == 3 || status == 4) break
                        val next = bits()
                        if (next != enabled && withContext(Dispatchers.IO) { NativeStorage.callTracks(handle, next) }) enabled = next
                        if (status == 2) { if (disconnectedAt == 0L) disconnectedAt = SystemClock.elapsedRealtime(); if (SystemClock.elapsedRealtime() - disconnectedAt > 5000) break } else disconnectedAt = 0
                        val state = if (status == 1) "connected" else if (status == 2) "reconnecting" else if (status == 5) "securing call" else "connecting"
                        visible = visible?.copy(connection = state, seconds = if (started == 0L) 0 else (SystemClock.elapsedRealtime() - started) / 1000, levels = levels); emit()
                        if (status == 1) {
                            retry = 1000
                            if (started == 0L && history.find { it.id == id }?.participants?.size?.let { it > 1 } == true) started = SystemClock.elapsedRealtime()
                            if (microphone == null) microphone = CallMicrophone({ timestamp, bytes -> send(0, timestamp, false, bytes) }, { amplitude -> scope.launch { levels = levels + ("self" to amplitude) } }, { scope.launch { issue("Microphone capture stopped."); end() } }).apply { muted = this@NativeCalls.muted }
                            if (video && cameraReady && camera == null) camera = CallCamera(app, front, { timestamp, keyframe, bytes -> send(1, timestamp, keyframe, bytes); videoOutputs["self:1"]?.offer(timestamp, keyframe, bytes) }, { scope.launch { video = false; camera?.close(); camera = null; applyTracks(); issue("Camera capture stopped.") } })
                            withContext(Dispatchers.IO) { NativeStorage.receiveCallFrames(handle)?.let { bytes -> try { receive(handle, bytes) } finally { bytes.fill(0) } } }
                        }
                        delay(20)
                    }
                } finally { token = 0; microphone?.close(); microphone = null; camera?.close(); camera = null; synchronized(speakers) { speakers.values.forEach { it.close() }; speakers.clear() }; NativeStorage.closeCall(handle) }
                delay(500)
            }
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { visible = visible?.copy(connection = "reconnecting"); emit(); delay(1000) }
        finally { media = null }
    }
    private fun send(kind: Int, timestamp: Long, keyframe: Boolean, bytes: ByteArray) {
        val handle = token
        if (handle != 0L) NativeStorage.sendCallFrame(handle, kind, timestamp, keyframe, bytes)
    }
    private fun receive(handle: Long, bytes: ByteArray) {
        val buffer = ByteBuffer.wrap(bytes)
        while (buffer.hasRemaining()) {
            require(buffer.remaining() >= 46)
            val sender = ByteArray(32).also { buffer.get(it) }.joinToString("") { "%02x".format(it) }
            val kind = buffer.get().toInt(); val keyframe = buffer.get().toInt() != 0; val timestamp = buffer.long; val size = buffer.int
            require(size in 1..minOf(1024 * 1024, buffer.remaining()))
            val frame = ByteArray(size)
            try {
                buffer.get(frame)
                if (kind == 0) synchronized(speakers) {
                    if (token != handle) return
                    require(speakers.size < 8 || speakers.containsKey(sender))
                    speakers.getOrPut(sender) { CallSpeaker({ amplitude -> scope.launch { levels = levels + (sender to amplitude) } }, { scope.launch { issue("Audio playback stopped.") } }) }.offer(timestamp, frame)
                } else videoOutputs["$sender:$kind"]?.offer(timestamp, keyframe, frame)
            } finally { frame.fill(0) }
        }
    }
    private fun bits() = (if (muted) 0 else 1) or (if (video) 2 else 0) or (if (sharing) 4 else 0)
    private fun applyTracks() {
        visible = visible?.copy(muted = muted, camera = video, screen = sharing); emit()
    }
    fun end(requested: String? = null) {
        if (ending?.isActive == true) return
        if (requested != null && desired != null && requested != desired) return
        val id = requested ?: desired
        generation++; desired = null; cameraReady = false; token = 0; control?.cancel(); control = null; media?.cancel()
        microphone?.close(); microphone = null
        camera?.close(); camera = null
        sharing = false; projectionRequest = null; screen?.close(); screen = null
        synchronized(speakers) { speakers.values.forEach { it.close() }; speakers.clear() }
        audio.isSpeakerphoneOn = false; audio.mode = previousAudioMode
        visible = null; emit()
        if (id == null) { stopService(); if (closing && !starting) scope.cancel(); return }
        ending = scope.launch {
            var saved = false
            try {
                refresh(native("call_leave", mapOf("call" to id)))
                saved = true
                NativeSync.presence(app, false)
                NativeSync.enqueue(app)
                val until = SystemClock.elapsedRealtime() + 30000
                while (SystemClock.elapsedRealtime() < until) {
                    val result = native("sync", mapOf("interactive" to true))
                    if (result.getBoolean("ran") && result.isNull("issue")) break
                    val wait = (result.getLong("next_at") * 1000 - System.currentTimeMillis()).coerceAtLeast(1000)
                    NativeSync.enqueue(app, wait)
                    if (wait >= until - SystemClock.elapsedRealtime()) break
                    delay(wait)
                }
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { NativeSync.enqueue(app); issue(if (saved) "Call status delivery will retry when connected." else "Could not save the call status. Retry ending the call.") }
            finally { ending = null; stopService(); if (closing) scope.cancel() }
        }
    }
    private fun stopService() { if (CallService.owner === this) { CallService.owner = null; app.stopService(Intent(app, CallService::class.java)) } }
    private fun <T> keys(work: (String, ByteArray) -> T): T = StorageKeyProvider(app).withKey { directory, key -> work(directory.path, key) }
    private suspend fun native(command: String, fields: Map<String, Any?> = emptyMap()): JSONObject = if (command == "sync") NativeSync.run(app, fields["interactive"] == true) else withContext(Dispatchers.IO) {
        val request = JSONObject().put("command", command); fields.forEach { (key, value) -> request.put(key, JSONObject.wrap(value)) }
        val result = keys { directory, key -> JSONObject(NativeStorage.execute(directory, key, request.toString())) }
        check(result.getBoolean("ok")); result.getJSONObject("value")
    }
    fun close() { closing = true; end() }
}
