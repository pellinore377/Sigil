package org.sigil.compose

import androidx.test.platform.app.InstrumentationRegistry
import com.google.firebase.FirebaseApp
import com.google.firebase.messaging.FirebaseMessaging
import org.json.JSONObject
import org.junit.Test
import org.junit.Assert.*
import org.junit.Assume.assumeTrue

class FirebaseBootstrapTest {
    private val context get()=InstrumentationRegistry.getInstrumentation().targetContext.also {assumeTrue(it.packageName.endsWith(".acceptance"))}
    private fun source(project:String,sender:String)=JSONObject().put("project_id",project)
        .put("application_id","1:$sender:android:0123456789abcdef")
        .put("api_key","AIza"+"x".repeat(35)).put("sender_id",sender).toString()
    @Test fun project_change_stages_a_restart_and_rejects_old_callbacks() {
        assertTrue(FirebaseApp.getApps(context).isEmpty())
        assertTrue(FirebaseBootstrap.prepare(context,source("synthetic-first","123456789")))
        assertTrue(FirebaseBootstrap.current(context))
        assertFalse(FirebaseMessaging.getInstance().isAutoInitEnabled)
        assertFalse(FirebaseBootstrap.prepare(context,source("synthetic-second","987654321")))
        assertFalse(FirebaseBootstrap.current(context))
        var called=false
        assertNull(FirebaseBootstrap.ifCurrent(context) {called=true})
        assertFalse(called)
        assertEquals("synthetic-first",FirebaseApp.getInstance().options.projectId)
    }
    @Test fun application_initializes_saved_default_before_services_then_honors_opt_out() {
        assertTrue(FirebaseBootstrap.current(context))
        assertEquals("synthetic-second",FirebaseApp.getInstance().options.projectId)
        assertEquals("987654321",FirebaseApp.getInstance().options.gcmSenderId)
        assertFalse(FirebaseMessaging.getInstance().isAutoInitEnabled)
        FirebaseBootstrap.stop(context)
        assertFalse(FirebaseBootstrap.current(context))
    }
    @Test fun disabled_delivery_does_not_initialize_google_on_cold_start() {
        assertTrue(FirebaseApp.getApps(context).isEmpty())
        assertFalse(FirebaseBootstrap.current(context))
        val invalid=JSONObject(source("synthetic-first","123456789")).put("private_key","synthetic secret").toString()
        assertTrue(runCatching {FirebaseBootstrap.prepare(context,invalid)}.isFailure)
        assertTrue(FirebaseApp.getApps(context).isEmpty())
        assertTrue(FirebaseBootstrap.prepare(context,source("synthetic-first","123456789")))
        NativePush.execute(context,"disable")
    }
    @Test fun durable_opt_out_wins_over_a_stale_bootstrap_after_crash() {
        assertEquals("disabled",NativePush.execute(context,"status").getString("choice"))
        assertTrue(FirebaseApp.getApps(context).isEmpty())
        assertFalse(FirebaseBootstrap.current(context))
    }
}
