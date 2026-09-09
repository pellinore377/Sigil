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
        withTimeout(20_000) {
            while (true) {
                NativeSync.files(context)
                if (NativePush.execute(context, "status").optString("remote") == "pending") break
                delay(100)
            }
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
            assertEquals("Instant delivery is enabled", NativePush.settings(context).status)
            val connection = NativePush.execute(context, "status").getString("connection")
            NativePush.execute(context, "disable")
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
