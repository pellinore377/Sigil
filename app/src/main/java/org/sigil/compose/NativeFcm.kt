package org.sigil.compose

import android.content.Context
import com.google.android.gms.common.ConnectionResult
import com.google.android.gms.common.GoogleApiAvailabilityLight
import com.google.firebase.FirebaseApp
import com.google.firebase.messaging.FirebaseMessaging
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeout
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

internal object NativeFcm {
    const val ID = "fcm"
    fun available(context: Context): Boolean = FirebaseApp.getApps(context).any { it.name == FirebaseApp.DEFAULT_APP_NAME } &&
        GoogleApiAvailabilityLight.getInstance().isGooglePlayServicesAvailable(context) == ConnectionResult.SUCCESS

    suspend fun register(context: Context, replace: Boolean): Boolean {
        check(available(context)) { "Google notifications are not configured in this build." }
        val token = withTimeout(20_000) { suspendCancellableCoroutine { continuation ->
            FirebaseMessaging.getInstance().token.addOnCompleteListener { task ->
                if (continuation.isActive) {
                    if (task.isSuccessful) continuation.resume(task.result)
                    else continuation.resumeWithException(IllegalStateException("Google notifications could not register. Try again."))
                }
            }
        } }
        val before = NativePush.execute(context, "status")
        val result = if (before.optString("choice") == ID && !replace)
            NativePush.execute(context, "fcm_token", mapOf("token" to token))
        else NativePush.execute(context, "fcm", mapOf("token" to token), replace)
        if (result.optBoolean("unavailable")) {
            NativePush.unavailable(context, "Your server has not enabled Google notifications.")
            return false
        }
        if (result.optBoolean("ignored")) return true
        if (replace && result.optString("remote") == "invalid") NativePush.execute(context, "retry")
        FirebaseMessaging.getInstance().isAutoInitEnabled = true
        NativePush.available(context)
        NativePush.unregister(context, before.optString("connection").takeIf { !before.isNull("connection") })
        return true
    }
    fun stop(context: Context) {
        if (available(context)) FirebaseMessaging.getInstance().isAutoInitEnabled = false
    }
    fun receive(context: Context, payload: String, urgent: Boolean) {
        if (payload.length > 98 || NativeSignOut.pending(context)) return
        val result = NativePush.execute(context, "fcm_receive", mapOf("payload" to payload))
        if (result.optBoolean("accepted")) {
            NativePush.available(context)
            NativeSync.enqueue(context, urgent = urgent)
        }
    }
}

class SigilFcmService : FirebaseMessagingService() {
    override fun onNewToken(token: String) {
        try {
            val result = NativePush.execute(this, "fcm_token", mapOf("token" to token))
            if (!result.optBoolean("ignored")) NativeSync.enqueue(this)
        } catch (_: Exception) { }
    }
    override fun onMessageReceived(message: RemoteMessage) {
        val payload = message.data["sigil"] ?: return
        try { NativeFcm.receive(this, payload, message.priority == RemoteMessage.PRIORITY_HIGH) }
        catch (_: Exception) { }
    }
    override fun onDeletedMessages() {
        try { if (NativePush.execute(this, "status").optString("choice") == NativeFcm.ID) NativeSync.enqueue(this) }
        catch (_: Exception) { }
    }
}
