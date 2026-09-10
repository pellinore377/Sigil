package org.sigil.compose

import android.content.Context
import kotlinx.coroutines.*
import org.json.JSONObject
import org.sigil.PushDistributor
import org.sigil.PushSettings
import org.sigil.storage.NativeStorage
import org.sigil.storage.StorageKeyProvider
import org.unifiedpush.android.connector.UnifiedPush
import org.unifiedpush.android.connector.MessagingReceiver
import org.unifiedpush.android.connector.FailedReason
import org.unifiedpush.android.connector.data.PushEndpoint
import org.unifiedpush.android.connector.data.PushMessage
import org.unifiedpush.android.connector.data.PublicKeySet
import org.unifiedpush.android.connector.keys.KeyManager
import java.io.File

internal object NativePush {
    private fun status(context: Context) = context.getSharedPreferences("push_connector", 0)
    fun unavailable(context: Context, reason: String) { status(context).edit().putString("issue", reason).apply(); NativeSync.enqueue(context) }
    fun available(context: Context) { status(context).edit().remove("issue").apply() }
    // The connector forwards sealed payloads; Rust owns the Web Push keys and validation.
    private object Keys : KeyManager {
        override fun decrypt(instance: String, sealed: ByteArray): ByteArray? = null
        override fun generate(instance: String) = Unit
        override fun getPublicKeySet(instance: String): PublicKeySet? = null
        override fun exists(instance: String) = false
        override fun delete(instance: String) = Unit
    }
    fun keys(): KeyManager = Keys
    fun selected(context: Context): String? = UnifiedPush.getSavedDistributor(context)
    fun distributors(context: Context): List<PushDistributor> = UnifiedPush.getDistributors(context).map { name ->
        val label = try { context.packageManager.getApplicationLabel(context.packageManager.getApplicationInfo(name, 0)).toString() } catch (_: Exception) { name }
        PushDistributor(name, label)
    }
    fun execute(context: Context, action: String, fields: Map<String, String> = emptyMap(), replace: Boolean = false): JSONObject {
        check(!NativeSignOut.pending(context) && File(context.noBackupFilesDir, "native/client.db").isFile)
        val request = JSONObject().put("command", "push").put("action", action).put("replace", replace)
        fields.forEach { (key, value) -> request.put(key, value) }
        val result = StorageKeyProvider(context).withKey { directory, key -> JSONObject(NativeStorage.execute(directory.path, key, request.toString())) }
        check(result.getBoolean("ok")) { "Push setup could not complete. Check your server's push configuration and retry." }
        return result.getJSONObject("value")
    }
    fun settings(context: Context): PushSettings {
        val state = execute(context, "status")
        val choice = state.getString("choice")
        val phase = when {
            !state.getBoolean("configured") && status(context).contains("issue") -> status(context).getString("issue", "Push is unavailable").orEmpty()
            choice == "disabled" -> if (state.optString("remote") == "active" || state.getBoolean("pending")) "Turning off push delivery…" else "Periodic background sync"
            choice == "fcm" && !NativeFcm.available(context) -> "Google notifications are unavailable on this device or build"
            choice == "unified_push" && selected(context) !in distributors(context).map { it.id } -> "Your push service is unavailable"
            status(context).contains("issue") -> status(context).getString("issue", "Push is unavailable").orEmpty()
            state.getBoolean("awaiting_endpoint") -> "Waiting for the push service"
            state.getBoolean("pending") -> "Registering with your server…"
            state.optString("remote") == "active" -> "Push delivery is enabled"
            state.optString("remote") == "pending" -> "Confirming delivery…"
            state.optString("remote") in listOf("invalid", "expired") -> "Registration needs attention"
            else -> "Waiting for registration"
        }
        val services = (if (NativeFcm.available(context)) listOf(PushDistributor(NativeFcm.ID, "Google notifications")) else emptyList()) + distributors(context)
        return PushSettings(choice != "disabled", phase, if (choice == "fcm") NativeFcm.ID else selected(context), services)
    }
    fun register(context: Context, distributor: String, registration: JSONObject) {
        require(distributor in distributors(context).map { it.id })
        NativeFcm.stop(context)
        available(context)
        UnifiedPush.saveDistributor(context, distributor)
        UnifiedPush.register(context, instance = registration.getString("connection"), messageForDistributor = "Sigil", vapid = registration.getString("vapid"), keyManager = Keys)
        NativeSync.enqueue(context)
    }
    fun unregister(context: Context, connection: String?) {
        connection?.let { UnifiedPush.unregister(context, it, Keys) }
        NativeSync.enqueue(context)
    }
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    @Volatile private var resumed = false
    @Volatile private var retryAt = 0L
    @Synchronized fun resume(context: Context) {
        if (resumed || android.os.SystemClock.elapsedRealtime() < retryAt || NativeSignOut.pending(context)) return
        resumed = true
        retryAt = android.os.SystemClock.elapsedRealtime() + 60_000
        scope.launch {
            try {
                val registration = execute(context, "status")
                if ((!registration.getBoolean("configured") || registration.optString("choice") == "fcm") && NativeFcm.available(context)) {
                    resumed = NativeFcm.register(context, false)
                    return@launch
                }
                val distributor = selected(context)
                if (!registration.isNull("connection") && distributor in distributors(context).map { it.id }) register(context, distributor!!, registration)
            } catch (cancelled: CancellationException) { resumed = false; throw cancelled }
            catch (_: Exception) { resumed = false }
        }
    }
}

@Suppress("DEPRECATION")
class SigilPushReceiver : MessagingReceiver() {
    override fun getKeyManager(context: Context) = NativePush.keys()
    private fun event(context: Context, action: String, fields: Map<String, String>, issue: String? = null) {
        if (NativeSignOut.pending(context)) return
        val pending = goAsync()
        CoroutineScope(Dispatchers.IO).launch {
            try {
                val result = NativePush.execute(context, action, fields)
                if (issue != null) {
                    if (result.optString("connection") == fields["connection"]) NativePush.unavailable(context, issue)
                    return@launch
                }
                if (action == "endpoint" || action == "receive" && result.optBoolean("accepted")) NativePush.available(context)
                if (action != "receive" || result.optBoolean("accepted")) NativeSync.enqueue(context)
            } catch (_: Exception) { }
            finally { pending.finish() }
        }
    }
    override fun onNewEndpoint(context: Context, endpoint: PushEndpoint, instance: String) = event(context, "endpoint", mapOf("connection" to instance, "endpoint" to endpoint.url))
    override fun onMessage(context: Context, message: PushMessage, instance: String) {
        if (message.decrypted || message.content.size !in 103..4096) return
        event(context, "receive", mapOf("connection" to instance, "payload" to android.util.Base64.encodeToString(message.content, android.util.Base64.URL_SAFE or android.util.Base64.NO_WRAP or android.util.Base64.NO_PADDING)))
    }
    override fun onUnregistered(context: Context, instance: String) = event(context, "unregistered", mapOf("connection" to instance))
    override fun onRegistrationFailed(context: Context, reason: FailedReason, instance: String) = event(context, "status", mapOf("connection" to instance), "The push service could not register. Reconnect to retry.")
    override fun onTempUnavailable(context: Context, instance: String) = event(context, "status", mapOf("connection" to instance), "The push service is temporarily unavailable")
}
