package org.sigil.compose

import android.content.Context
import com.google.android.gms.common.ConnectionResult
import com.google.android.gms.common.GoogleApiAvailabilityLight
import com.google.firebase.messaging.FirebaseMessaging
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

internal object NativeFcm {
    const val ID = "fcm"
    private val registration=Mutex()
    fun available(context: Context): Boolean =
        GoogleApiAvailabilityLight.getInstance().isGooglePlayServicesAvailable(context) == ConnectionResult.SUCCESS

    suspend fun register(context: Context, replace: Boolean): Boolean = registration.withLock {
        check(available(context)) { "Google notifications require Google Play services." }
        val configuration=NativePush.execute(context,"android",replace=replace)
        if(configuration.optBoolean("ignored"))return@withLock true
        if(configuration.isNull("android")) {
            FirebaseBootstrap.stop(context)
            NativePush.unavailable(context,if(configuration.optBoolean("legacy"))
                "Update the server to configure Google notifications for this app." else
                "Your administrator needs to add the Android Firebase configuration in Notifications.")
            return@withLock false
        }
        if(!FirebaseBootstrap.prepare(context,configuration.getJSONObject("android").toString())) {
            NativePush.unavailable(context,"The notification project changed. Force stop Sigil in Android settings, then reopen it to finish setup.")
            return@withLock false
        }
        val token = withTimeout(20_000) { suspendCancellableCoroutine { continuation ->
            FirebaseMessaging.getInstance().token.addOnCompleteListener { task ->
                if (continuation.isActive) {
                    if (task.isSuccessful) continuation.resume(task.result)
                    else continuation.resumeWithException(IllegalStateException("Google notifications could not register. Try again."))
                }
            }
        } }
        val outcome=FirebaseBootstrap.ifCurrent(context) {
            val before=NativePush.execute(context,"status")
            val result=if(before.optString("choice")==ID && !replace)
                NativePush.execute(context,"fcm_token",mapOf("token" to token))
            else NativePush.execute(context,"fcm",mapOf("token" to token),replace)
            if(!result.optBoolean("ignored") && !result.optBoolean("unavailable"))FirebaseMessaging.getInstance().isAutoInitEnabled=true
            before to result
        } ?: return@withLock false
        val (before,result)=outcome
        if (result.optBoolean("unavailable")) {
            FirebaseBootstrap.stop(context)
            NativePush.unavailable(context, "Your server has not enabled Google notifications.")
            return@withLock false
        }
        if (result.optBoolean("ignored")) {FirebaseBootstrap.stop(context);return@withLock true}
        if (replace && result.optString("remote") == "invalid") NativePush.execute(context, "retry")
        NativePush.available(context)
        NativePush.unregister(context, before.optString("connection").takeIf { !before.isNull("connection") })
        true
    }
    fun stop(context: Context) = FirebaseBootstrap.stop(context)
    fun receive(context: Context, payload: String, urgent: Boolean) {
        if (payload.length > 98 || NativeSignOut.pending(context)) return
        val result = FirebaseBootstrap.ifCurrent(context) {NativePush.execute(context, "fcm_receive", mapOf("payload" to payload))} ?: return
        if (result.optBoolean("accepted")) {
            NativePush.available(context)
            NativeSync.enqueue(context, urgent = urgent)
        }
    }
}

class SigilFcmService : FirebaseMessagingService() {
    override fun onNewToken(token: String) {
        try {
            val result = FirebaseBootstrap.ifCurrent(this) {NativePush.execute(this, "fcm_token", mapOf("token" to token))} ?: return
            if (!result.optBoolean("ignored")) NativeSync.enqueue(this)
        } catch (_: Exception) { }
    }
    override fun onMessageReceived(message: RemoteMessage) {
        val payload = message.data["sigil"] ?: return
        try { NativeFcm.receive(this, payload, message.priority == RemoteMessage.PRIORITY_HIGH) }
        catch (_: Exception) { }
    }
    override fun onDeletedMessages() {
        try { if (FirebaseBootstrap.current(this) && NativePush.execute(this, "status").optString("choice") == NativeFcm.ID) NativeSync.enqueue(this) }
        catch (_: Exception) { }
    }
}
