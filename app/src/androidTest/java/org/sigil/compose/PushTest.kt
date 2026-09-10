package org.sigil.compose

import android.content.*
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.*
import org.junit.*
import org.junit.Assert.*
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

class FixturePushActivity : android.app.Activity() {
    override fun onCreate(state: android.os.Bundle?) {
        super.onCreate(state)
        packageManager.setComponentEnabledSetting(ComponentName(this, FixturePushDistributor::class.java), if (intent.getBooleanExtra("enabled", false)) android.content.pm.PackageManager.COMPONENT_ENABLED_STATE_ENABLED else android.content.pm.PackageManager.COMPONENT_ENABLED_STATE_DISABLED, android.content.pm.PackageManager.DONT_KILL_APP)
        finish()
    }
}

// Runs in the test APK's process without the target APK's Kotlin runtime.
class FixturePushDistributor : BroadcastReceiver() {
    override fun onReceive(context: Context?, intent: Intent?) {
        if (context == null || intent == null) return
        val store = context.getSharedPreferences("push-fixture", 0)
        when (intent.action) {
            "sigil.fixture.CONFIGURE" -> { store.edit().clear().putString("endpoint", intent.getStringExtra("endpoint")).commit(); resultCode = 1 }
            "org.unifiedpush.android.distributor.REGISTER" -> {
                val app = intent.getStringExtra("application") ?: return
                if (!java.util.regex.Pattern.matches("org\\.sigil\\.compose\\.acceptance", app)) return
                val token = intent.getStringExtra("token") ?: return
                store.edit().putString("app", app).putString("token", token).commit()
                context.sendBroadcast(Intent("org.unifiedpush.android.connector.NEW_ENDPOINT").setPackage(app).putExtra("token", token).putExtra("endpoint", store.getString("endpoint", null)))
            }
            "sigil.fixture.DELIVER" -> {
                val app = store.getString("app", null) ?: return
                context.sendBroadcast(Intent("org.unifiedpush.android.connector.MESSAGE").setPackage(app).putExtra("token", store.getString("token", null)).putExtra("bytesMessage", intent.getByteArrayExtra("payload")))
                resultCode = 1
            }
            "org.unifiedpush.android.distributor.UNREGISTER" -> store.edit().remove("app").remove("token").apply()
        }
    }
}

class PushTest {
    private val instrumentation get() = InstrumentationRegistry.getInstrumentation()
    private val context get() = instrumentation.targetContext
    private val distributor get() = ComponentName(instrumentation.context.packageName, FixturePushDistributor::class.java.name)
    @Before fun isolated() { Assume.assumeTrue(context.packageName.endsWith(".acceptance")) }
    @Test fun foregroundSyncTimings() {
        val unwrap = mutableListOf<Long>(); val sync = mutableListOf<Long>(); val state = mutableListOf<Long>()
        repeat(6) {
            val begin = android.os.SystemClock.elapsedRealtimeNanos()
            org.sigil.storage.StorageKeyProvider(context).withKey { directory, key ->
                val ready = android.os.SystemClock.elapsedRealtimeNanos()
                val result = org.json.JSONObject(org.sigil.storage.NativeStorage.execute(directory.path, key, "{\"command\":\"sync\",\"interactive\":true}"))
                assertTrue(result.toString(), result.getBoolean("ok"))
                val synced = android.os.SystemClock.elapsedRealtimeNanos()
                val snapshot = org.json.JSONObject(org.sigil.storage.NativeStorage.execute(directory.path, key, "{\"command\":\"state\",\"calls\":true}"))
                assertTrue(snapshot.toString(), snapshot.getBoolean("ok"))
                assertTrue(snapshot.getJSONObject("value").getJSONObject("call_state").has("calls"))
                if (result.getJSONObject("value").getBoolean("ran")) {
                    unwrap += ready - begin; sync += synced - ready; state += android.os.SystemClock.elapsedRealtimeNanos() - synced
                }
            }
            Thread.sleep(1100)
        }
        assertTrue(sync.isNotEmpty())
        fun p95(samples: List<Long>) = samples.sorted()[(samples.size * 0.95).toInt().coerceAtMost(samples.lastIndex)] / 1_000_000.0
        android.util.Log.i("SigilAcceptance", "Foreground fixture sync: ${sync.size} passes, p95 unwrap=${p95(unwrap)} ms, sync=${p95(sync)} ms, state=${p95(state)} ms")
    }
    private suspend fun enable(enabled: Boolean) {
        context.startActivity(Intent().setComponent(ComponentName(distributor.packageName, FixturePushActivity::class.java.name)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK).putExtra("enabled", enabled))
        val expected = if (enabled) android.content.pm.PackageManager.COMPONENT_ENABLED_STATE_ENABLED else android.content.pm.PackageManager.COMPONENT_ENABLED_STATE_DISABLED
        withTimeout(5000) { while (context.packageManager.getComponentEnabledSetting(distributor) != expected) delay(50) }
    }
    private fun broadcast(intent: Intent) {
        val done = CountDownLatch(1)
        var result = 0
        context.sendOrderedBroadcast(intent.setComponent(distributor), null, object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) { result = resultCode; done.countDown() }
        }, null, 0, null, null)
        assertTrue(done.await(10, TimeUnit.SECONDS)); assertEquals(1, result)
    }
    @Test fun register() = runBlocking {
        enable(true)
        broadcast(Intent("sigil.fixture.CONFIGURE").putExtra("endpoint", File(context.cacheDir, "push-endpoint").readText()))
        val registration = NativePush.execute(context, "prepare")
        NativePush.register(context, distributor.packageName, registration)
        var issue:String?=null
        try {withTimeout(20_000) {
            while (true) {
                issue=NativeSync.files(context).optString("issue").takeUnless {it=="null" || it.isEmpty()}
                if (NativePush.execute(context, "status").optString("remote") == "pending") break
                delay(100)
            }
        }} catch(error:TimeoutCancellationException) {
            val status=NativePush.execute(context,"status")
            throw AssertionError("Push registration timed out: remote=${status.optString("remote")}, awaiting_endpoint=${status.optBoolean("awaiting_endpoint")}, pending=${status.optBoolean("pending")}, issue=$issue",error)
        }
        assertFalse(NativePush.settings(context).status.contains("enabled"))
    }
    @Test fun encryptedProofReachesRustThroughTheDistributorAndReceiver() = runBlocking {
        try {
            val sealed = File(context.cacheDir, "push-sealed").readBytes()
            val damaged = sealed.clone().also { it[it.lastIndex] = (it.last().toInt() xor 1).toByte() }
            broadcast(Intent("sigil.fixture.DELIVER").putExtra("payload", damaged))
            delay(500)
            assertEquals("pending", NativePush.execute(context, "status").optString("remote"))
            broadcast(Intent("sigil.fixture.DELIVER").putExtra("payload", sealed))
            withTimeout(20_000) {
                while (true) {
                    NativeSync.files(context)
                    if (NativePush.execute(context, "status").optString("remote") == "active") break
                    delay(100)
                }
            }
            assertEquals("Push delivery is enabled", NativePush.settings(context).status)
            assertTrue(NativePush.execute(context, "fcm_token", mapOf("token" to "synthetic-late-google-token")).getBoolean("ignored"))
            NativeFcm.receive(context, "U0dQVwAAAAAA", true)
            assertEquals("unified_push", NativePush.execute(context, "status").getString("choice"))
            val connection = NativePush.execute(context, "status").getString("connection")
            NativePush.execute(context, "disable")
            assertTrue(NativePush.execute(context, "fcm_token", mapOf("token" to "synthetic-disabled-google-token")).getBoolean("ignored"))
            NativePush.unregister(context, connection)
            withTimeout(20_000) {
                while (NativePush.execute(context, "status").optString("remote") != "disabled") { NativeSync.files(context); delay(100) }
            }
        } finally {
            enable(false)
            File(context.cacheDir, "push-sealed").delete(); File(context.cacheDir, "push-endpoint").delete()
        }
    }
}
