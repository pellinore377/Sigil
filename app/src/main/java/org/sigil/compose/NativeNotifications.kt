package org.sigil.compose

import android.Manifest
import android.app.*
import android.content.*
import android.content.pm.PackageManager
import android.media.AudioAttributes
import android.media.RingtoneManager
import android.graphics.drawable.Icon
import android.os.Build
import android.provider.Settings
import kotlinx.coroutines.*
import org.json.JSONObject
import org.sigil.NotificationSettings
import org.sigil.storage.NativeStorage
import org.sigil.storage.StorageKeyProvider
import java.security.SecureRandom

/** Device-local notifications: content is decrypted here after a wake; push never carries it. */
internal object NativeNotifications {
    @Volatile private var visible = false
    private const val MESSAGES = 40
    private const val INCOMING = 41
    private const val MISSED = 44
    private const val GROUP = "sigil-messages"
    const val EXTRA_PEER = "org.sigil.open_peer"
    const val EXTRA_ANSWER = "org.sigil.answer_call"
    private fun preferences(context: Context) = context.getSharedPreferences("notifications", Context.MODE_PRIVATE)
    fun settings(context: Context): NotificationSettings {
        val manager = context.getSystemService(NotificationManager::class.java)
        val preferences = preferences(context)
        return NotificationSettings(manager.areNotificationsEnabled(), preferences.getBoolean("messages", true), preferences.getBoolean("incoming", true), preferences.getString("content", "full") ?: "full",
            Build.VERSION.SDK_INT < 34 || manager.canUseFullScreenIntent())
    }
    fun fullScreenSettings(context: Context) {
        if (Build.VERSION.SDK_INT >= 34) context.startActivity(Intent(Settings.ACTION_MANAGE_APP_USE_FULL_SCREEN_INTENT).setData(android.net.Uri.parse("package:" + context.packageName)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }
    /** Contact photo when cached, else a lettered disc in the same spirit as the app's avatars. */
    private fun avatar(context: Context, name: String, reference: String): Icon {
        val bytes = reference.takeIf { it.isNotBlank() }?.let { runCatching { StorageKeyProvider(context).withKey { directory, key -> NativeStorage.profilePhoto(directory.path, key, it) } }.getOrNull() }
        val photo = bytes?.let { android.graphics.BitmapFactory.decodeByteArray(it, 0, it.size) }
        if (photo != null) return Icon.createWithAdaptiveBitmap(photo)
        val size = 108
        val bitmap = android.graphics.Bitmap.createBitmap(size, size, android.graphics.Bitmap.Config.ARGB_8888)
        val canvas = android.graphics.Canvas(bitmap)
        val hues = intArrayOf(0xff6750a4.toInt(), 0xff386641.toInt(), 0xff9a3b3b.toInt(), 0xff1f5f8b.toInt(), 0xff7a4f01.toInt(), 0xff4a4e69.toInt())
        val paint = android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG).apply { color = hues[(name.hashCode() and 0x7fffffff) % hues.size] }
        canvas.drawCircle(size / 2f, size / 2f, size / 2f, paint)
        paint.color = 0xffffffff.toInt(); paint.textSize = size * 0.5f; paint.textAlign = android.graphics.Paint.Align.CENTER
        paint.typeface = android.graphics.Typeface.create(android.graphics.Typeface.DEFAULT, android.graphics.Typeface.NORMAL)
        val initial = name.trim().firstOrNull()?.uppercaseChar()?.toString() ?: "?"
        canvas.drawText(initial, size / 2f, size / 2f - (paint.descent() + paint.ascent()) / 2, paint)
        return Icon.createWithBitmap(bitmap)
    }
    fun change(context: Context, key: String, enabled: Boolean) {
        require(key == "messages" || key == "incoming")
        preferences(context).edit().putBoolean(key, enabled).apply()
        if (!enabled) { if (key == "messages") clearMessages(context) else context.getSystemService(NotificationManager::class.java).cancel(INCOMING) }
        if (key == "incoming") preferences(context).edit().remove("incoming_call").apply()
    }
    fun content(context: Context, level: String) {
        require(level in listOf("full", "name", "none"))
        preferences(context).edit().putString("content", level).remove("message_revision").apply()
        clearMessages(context)
    }
    fun systemSettings(context: Context) { context.startActivity(Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
    fun foreground(context: Context, value: Boolean) {
        visible = value
        if (value) {
            clearMessages(context)
            context.getSystemService(NotificationManager::class.java).cancel(INCOMING)
            preferences(context).edit().remove("incoming_call").remove("message_revision").apply()
        }
    }
    private fun clearMessages(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.cancel(MESSAGES)
        preferences(context).getStringSet("active", emptySet())?.forEach { manager.cancel(it.toInt()) }
        preferences(context).edit().remove("active").apply()
    }
    private fun chatNotificationId(peer: String) = 1000 + (peer.hashCode() and 0x7fff)
    fun update(context: Context) {
        if (NativeSignOut.pending(context) || visible || Build.VERSION.SDK_INT >= 33 && context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return
        val state = StorageKeyProvider(context).withKey { directory, key -> JSONObject(NativeStorage.execute(directory.path, key, "{\"command\":\"notifications\"}")) }
        if (!state.getBoolean("ok") || visible) return
        val value = state.getJSONObject("value")
        android.util.Log.i("SigilTiming", "notifications unread=${value.optLong("unread")} calls=${value.optJSONArray("calls")?.length() ?: 0} missed=${!value.isNull("missed")}")
        show(context, value)
    }
    private fun channels(manager: NotificationManager) {
        manager.createNotificationChannel(NotificationChannel("messages", "Messages", NotificationManager.IMPORTANCE_HIGH))
        manager.createNotificationChannel(NotificationChannel("incoming_calls", "Incoming calls", NotificationManager.IMPORTANCE_HIGH).apply {
            setSound(RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE), AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_NOTIFICATION_RINGTONE).build())
            enableVibration(true)
            lockscreenVisibility = Notification.VISIBILITY_PUBLIC
        })
    }
    private fun label(message: JSONObject): String {
        val text = message.optString("text")
        if (text.isNotBlank()) return text
        return when (message.optString("kind")) { "Images" -> "Photo"; "Videos" -> "Video"; "Files" -> "Attachment"; "Voice" -> "Voice message"; else -> "New message" }
    }
    internal fun show(context: Context, value: JSONObject) {
        if (visible || NativeSignOut.pending(context)) return
        val manager = context.getSystemService(NotificationManager::class.java)
        channels(manager)
        val settings = settings(context)
        if (!settings.enabled) return
        val preferences = preferences(context)
        val activity = { code: Int, extras: Intent.() -> Unit -> PendingIntent.getActivity(context, code, Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP).apply(extras), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT) }
        if (value.getLong("unread") > 0 && settings.messages) {
            if (preferences.getString("message_revision", null) != value.getString("revision")) {
                val chats = value.getJSONArray("chats")
                val active = mutableSetOf<String>()
                val you = Person.Builder().setName("You").build()
                for (index in 0 until chats.length()) {
                    val chat = chats.getJSONObject(index)
                    val peer = chat.getString("id")
                    val id = chatNotificationId(peer)
                    active += id.toString()
                    val hidden = settings.content == "none"
                    val name = if (hidden) "Sigil" else chat.optString("name").ifBlank { "Conversation" }
                    val sender = Person.Builder().setName(name).setKey(peer).setIcon(if (hidden) null else avatar(context, name, chat.optString("avatar"))).build()
                    val style = Notification.MessagingStyle(you).setGroupConversation(chat.optBoolean("group") && !hidden)
                    if (chat.optBoolean("group") && !hidden) style.conversationTitle = name
                    val messages = chat.getJSONArray("messages")
                    if (messages.length() == 0) style.addMessage("New message", System.currentTimeMillis(), sender)
                    for (m in 0 until messages.length()) {
                        val message = messages.getJSONObject(m)
                        style.addMessage(if (settings.content == "full") label(message) else "New message", message.optLong("timestamp") * 1000, sender)
                    }
                    val open = activity(id) { putExtra(EXTRA_PEER, peer) }
                    val builder = Notification.Builder(context, "messages").setSmallIcon(android.R.drawable.sym_action_chat).setStyle(style)
                        .setContentIntent(open).setAutoCancel(true).setOnlyAlertOnce(true).setGroup(GROUP).setCategory(Notification.CATEGORY_MESSAGE)
                        .setVisibility(Notification.VISIBILITY_PRIVATE).setShortcutId(peer).setWhen(chat.optJSONArray("messages")?.let { if (it.length() > 0) it.getJSONObject(it.length() - 1).optLong("timestamp") * 1000 else null } ?: System.currentTimeMillis()).setShowWhen(true)
                        .setPublicVersion(Notification.Builder(context, "messages").setSmallIcon(android.R.drawable.sym_action_chat).setContentTitle("Sigil").setContentText("New messages").build())
                    if (!hidden) {
                        val reply = PendingIntent.getBroadcast(context, id + 1, Intent(context, NotificationActionReceiver::class.java).setAction("reply").putExtra("peer", peer), PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
                        val read = PendingIntent.getBroadcast(context, id + 2, Intent(context, NotificationActionReceiver::class.java).setAction("read").putExtra("peer", peer), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
                        builder.addAction(Notification.Action.Builder(null, "Reply", reply).addRemoteInput(RemoteInput.Builder("reply").setLabel("Reply").build()).setAllowGeneratedReplies(false).build())
                        builder.addAction(Notification.Action.Builder(null, "Mark read", read).build())
                    }
                    manager.notify(id, builder.build())
                }
                preferences.getStringSet("active", emptySet())?.filter { it !in active }?.forEach { manager.cancel(it.toInt()) }
                if (chats.length() > 1) manager.notify(MESSAGES, Notification.Builder(context, "messages").setSmallIcon(android.R.drawable.sym_action_chat).setContentTitle("Sigil")
                    .setContentText("${value.getLong("unread")} new messages").setGroup(GROUP).setGroupSummary(true).setAutoCancel(true).setOnlyAlertOnce(true).setContentIntent(activity(MESSAGES) {}).setVisibility(Notification.VISIBILITY_PRIVATE).build())
                else manager.cancel(MESSAGES)
                preferences.edit().putStringSet("active", active).putString("message_revision", value.getString("revision")).apply()
            }
        } else clearMessages(context)
        val calls = value.getJSONArray("calls")
        val call = (0 until calls.length()).map { calls.getJSONObject(it) }.firstOrNull { it.optLong("until") * 1000 > System.currentTimeMillis() }
        if (call != null && settings.calls && preferences.getString("incoming_call", null) != call.getString("id")) {
            val id = call.getString("id")
            val caller = call.optString("name").ifBlank { "Sigil call" }
            val decline = PendingIntent.getBroadcast(context, 41, Intent(context, CallNotificationReceiver::class.java).putExtra("call", id), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
            val answer = activity(42) { putExtra(EXTRA_ANSWER, id) }
            val show = activity(43) { putExtra(EXTRA_PEER, "calls") }
            val timeout = (call.getLong("until") * 1000 - System.currentTimeMillis()).coerceAtLeast(1)
            val builder = if (Build.VERSION.SDK_INT >= 31) Notification.Builder(context, "incoming_calls").setStyle(Notification.CallStyle.forIncomingCall(Person.Builder().setName(caller).setImportant(true).build(), decline, answer))
                else Notification.Builder(context, "incoming_calls").setContentTitle("Incoming Sigil call").setContentText(caller)
                    .addAction(Notification.Action.Builder(null, "Decline", decline).build()).addAction(Notification.Action.Builder(null, "Answer", answer).build())
            manager.notify(INCOMING, builder.setSmallIcon(android.R.drawable.sym_action_call).setCategory(Notification.CATEGORY_CALL).setContentIntent(show)
                .setFullScreenIntent(show, true).setOngoing(true).setOnlyAlertOnce(true).setVisibility(Notification.VISIBILITY_PUBLIC).setTimeoutAfter(timeout).build())
            preferences.edit().putString("incoming_call", id).apply()
        } else if (call == null || !settings.calls) { manager.cancel(INCOMING); preferences.edit().remove("incoming_call").apply() }
        val missed = value.optJSONObject("missed")
        if (missed != null && settings.calls && preferences.getString("missed_call", null) != missed.optString("id")) {
            manager.notify(MISSED, Notification.Builder(context, "messages").setSmallIcon(android.R.drawable.sym_action_call).setContentTitle("Missed call")
                .setContentText(missed.optString("name").ifBlank { "Sigil call" }).setCategory(Notification.CATEGORY_MISSED_CALL).setContentIntent(activity(MISSED) { putExtra(EXTRA_PEER, "calls") })
                .setAutoCancel(true).setWhen(missed.optLong("created") * 1000).setShowWhen(true).setVisibility(Notification.VISIBILITY_PRIVATE).build())
            preferences.edit().putString("missed_call", missed.optString("id")).apply()
        }
    }
}

/** Inline reply and mark-as-read from a message notification; runs the same native commands as the app. */
class NotificationActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (NativeSignOut.pending(context)) return
        val peer = intent.getStringExtra("peer")?.takeIf { it.length <= 128 } ?: return
        val text = RemoteInput.getResultsFromIntent(intent)?.getCharSequence("reply")?.toString()?.trim().orEmpty()
        val action = intent.action
        val pending = goAsync()
        CoroutineScope(Dispatchers.IO).launch {
            try {
                withTimeout(15000) {
                    StorageKeyProvider(context).withKey { directory, key ->
                        fun run(request: JSONObject) = JSONObject(NativeStorage.execute(directory.path, key, request.toString()))
                        fun stamped(command: String) = JSONObject().put("command", command).put("peer", peer)
                            .put("request", ByteArray(32).also { SecureRandom().nextBytes(it) }.joinToString("") { "%02x".format(it) }).put("timestamp", System.currentTimeMillis() / 1000)
                        if (action == "reply" && text.isNotEmpty()) {
                            run(stamped("post").put("text", text).put("timezone", java.util.TimeZone.getDefault().id))
                            run(JSONObject().put("command", "flush"))
                        }
                        run(stamped("mark_read"))
                    }
                    NativeNotifications.update(context)
                    NativeSync.enqueue(context)
                }
            } catch (_: Exception) { NativeSync.enqueue(context) }
            finally { pending.finish() }
        }
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
