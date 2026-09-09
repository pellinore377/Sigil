package org.sigil.compose

import android.Manifest
import android.app.*
import android.content.*
import android.content.pm.PackageManager
import android.media.AudioAttributes
import android.media.RingtoneManager
import android.os.Build
import android.provider.Settings
import kotlinx.coroutines.*
import org.json.JSONObject
import org.sigil.NotificationSettings
import org.sigil.storage.NativeStorage
import org.sigil.storage.StorageKeyProvider

internal object NativeNotifications {
    @Volatile private var visible = false
    private const val MESSAGES = 40
    private const val INCOMING = 41
    private fun preferences(context: Context) = context.getSharedPreferences("notifications", Context.MODE_PRIVATE)
    fun settings(context: Context): NotificationSettings {
        val manager = context.getSystemService(NotificationManager::class.java)
        return NotificationSettings(manager.areNotificationsEnabled(), preferences(context).getBoolean("messages", true), preferences(context).getBoolean("incoming", true))
    }
    fun change(context: Context, key: String, enabled: Boolean) {
        require(key == "messages" || key == "incoming")
        preferences(context).edit().putBoolean(key, enabled).apply()
        if (!enabled) context.getSystemService(NotificationManager::class.java).cancel(if (key == "messages") MESSAGES else INCOMING)
        if (key == "incoming") preferences(context).edit().remove("incoming_call").apply()
    }
    fun systemSettings(context: Context) { context.startActivity(Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
    fun foreground(context: Context, value: Boolean) {
        visible = value
        if (value) {
            context.getSystemService(NotificationManager::class.java).apply { cancel(MESSAGES); cancel(INCOMING) }
            preferences(context).edit().remove("incoming_call").apply()
        }
    }
    fun update(context: Context) {
        if (NativeSignOut.pending(context) || visible || Build.VERSION.SDK_INT >= 33 && context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return
        val state = StorageKeyProvider(context).withKey { directory, key -> JSONObject(NativeStorage.execute(directory.path, key, "{\"command\":\"notifications\"}")) }
        if (!state.getBoolean("ok") || visible) return
        show(context, state.getJSONObject("value"))
    }
    internal fun show(context: Context, value: JSONObject) {
        if (visible || NativeSignOut.pending(context)) return
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel("messages", "Messages", NotificationManager.IMPORTANCE_DEFAULT))
        manager.createNotificationChannel(NotificationChannel("incoming_calls", "Incoming calls", NotificationManager.IMPORTANCE_HIGH).apply {
            setSound(RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE), AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_NOTIFICATION_RINGTONE).build())
        })
        val open = PendingIntent.getActivity(context, 40, Intent(context, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val settings = settings(context)
        if (!settings.enabled) return
        val preferences = preferences(context)
        if (value.getLong("unread") > 0 && settings.messages) {
            if (preferences.getString("message_revision", null) != value.getString("revision")) manager.notify(MESSAGES, Notification.Builder(context, "messages").setSmallIcon(android.R.drawable.sym_action_chat).setContentTitle("Sigil")
                .setContentText("New messages").setContentIntent(open).setAutoCancel(true).setOnlyAlertOnce(true).setVisibility(Notification.VISIBILITY_PRIVATE).build())
        } else manager.cancel(MESSAGES)
        preferences.edit().putString("message_revision", value.getString("revision")).apply()
        val calls = value.getJSONArray("calls")
        val call = (0 until calls.length()).map { calls.getJSONObject(it) }.firstOrNull { it.optLong("until") * 1000 > System.currentTimeMillis() }
        if (call != null && settings.calls && preferences.getString("incoming_call", null) != call.getString("id")) {
            val decline = PendingIntent.getBroadcast(context, 41, Intent(context, CallNotificationReceiver::class.java).putExtra("call", call.getString("id")), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            manager.notify(INCOMING, Notification.Builder(context, "incoming_calls").setSmallIcon(android.R.drawable.sym_action_call).setContentTitle("Incoming Sigil call")
                .setContentText("Open Sigil to answer").setCategory(Notification.CATEGORY_CALL).setContentIntent(open).setOnlyAlertOnce(true).setVisibility(Notification.VISIBILITY_PRIVATE)
                .setTimeoutAfter((call.getLong("until") * 1000 - System.currentTimeMillis()).coerceAtLeast(1))
                .addAction(Notification.Action.Builder(null, "Decline", decline).build()).addAction(Notification.Action.Builder(null, "Open call", open).build()).build())
            preferences.edit().putString("incoming_call", call.getString("id")).apply()
        } else if (call == null || !settings.calls) { manager.cancel(INCOMING); preferences.edit().remove("incoming_call").apply() }
    }
}

class CallNotificationReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (NativeSignOut.pending(context)) return
        val id = intent.getStringExtra("call")?.takeIf { it.length == 64 && it.all { ch -> ch in '0'..'9' || ch in 'a'..'f' } } ?: return
        val pending = goAsync()
        CoroutineScope(Dispatchers.IO).launch {
            try {
                withTimeout(8000) {
                    val request = JSONObject().put("command", "call_answer").put("call", id).put("accept", false)
                    StorageKeyProvider(context).withKey { directory, key -> NativeStorage.execute(directory.path, key, request.toString()) }
                    NativeSync.enqueue(context)
                    NativeNotifications.update(context)
                }
            } catch (_: Exception) { NativeSync.enqueue(context) }
            finally { pending.finish() }
        }
    }
}
