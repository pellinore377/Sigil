package org.sigil.compose

import android.app.Notification
import android.app.NotificationManager
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Test
import org.junit.Assert.*
import org.junit.Assume.assumeTrue

class NotificationTest {
    @Test fun alertsArePrivateAndDismissedMessagesDoNotReturnWithoutAChange() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        assumeTrue(context.packageName.endsWith(".acceptance"))
        val manager = context.getSystemService(NotificationManager::class.java)
        fun await(id: Int, present: Boolean) {
            val until = android.os.SystemClock.elapsedRealtime() + 3000
            while (manager.activeNotifications.any { it.id == id } != present && android.os.SystemClock.elapsedRealtime() < until) android.os.SystemClock.sleep(30)
            assertEquals(present, manager.activeNotifications.any { it.id == id })
        }
        if (android.os.Build.VERSION.SDK_INT >= 33) InstrumentationRegistry.getInstrumentation().uiAutomation.grantRuntimePermission(context.packageName, android.Manifest.permission.POST_NOTIFICATIONS)
        val preferences = context.getSharedPreferences("notifications", 0)
        preferences.edit().clear().commit()
        NativeNotifications.foreground(context, false)
        val state = JSONObject().put("unread", 1).put("revision", "synthetic-a").put("calls", JSONArray())
        try {
            NativeNotifications.show(context, state)
            await(40, true)
            val alert = manager.activeNotifications.single { it.id == 40 }.notification
            assertEquals("New messages", alert.extras.getCharSequence(Notification.EXTRA_TEXT).toString())
            assertEquals(Notification.VISIBILITY_PRIVATE, alert.visibility)
            manager.cancel(40)
            await(40, false)
            NativeNotifications.show(context, state)
            assertTrue(manager.activeNotifications.none { it.id == 40 })
            NativeNotifications.show(context, state.put("revision", "synthetic-b"))
            await(40, true)
            NativeNotifications.change(context, "messages", false)
            await(40, false)
            NativeNotifications.show(context, state.put("calls", JSONArray().put(JSONObject().put("id", "ab".repeat(32)).put("until", System.currentTimeMillis() / 1000 + 50))))
            await(41, true)
            val call = manager.activeNotifications.single { it.id == 41 }.notification
            assertEquals("Incoming Sigil call", call.extras.getCharSequence(Notification.EXTRA_TITLE).toString())
            assertEquals(listOf("Decline", "Open call"), call.actions.map { it.title.toString() })
            NativeNotifications.foreground(context, true)
            await(41, false)
            NativeNotifications.foreground(context, false)
            NativeNotifications.show(context, state)
            await(41, true)
            NativeNotifications.change(context, "incoming", false)
            await(41, false)
            NativeNotifications.change(context, "incoming", true)
            NativeNotifications.show(context, state)
            await(41, true)
        } finally { manager.cancel(40); manager.cancel(41); preferences.edit().clear().commit() }
    }
}
