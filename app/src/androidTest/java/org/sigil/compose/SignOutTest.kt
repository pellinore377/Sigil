package org.sigil.compose

import android.app.job.JobScheduler
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.*
import org.junit.Assert.*
import org.sigil.storage.NativeStorage
import org.sigil.storage.StorageKeyProvider

class SignOutTest {
    @Test fun revokeAndRemoveSyntheticAppData() = runBlocking {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        check(context.packageName == "org.sigil.compose.acceptance")
        assertTrue(java.io.File(context.noBackupFilesDir, "native/client.db").isFile)
        NativeSync.enable(context, true)
        assertTrue(context.getSystemService(JobScheduler::class.java).allPendingJobs.isNotEmpty())
        NativeSignOut.save(context, "pending")
        NativeSync.enable(context, true)
        NativeSync.enqueue(context)
        assertTrue(context.getSystemService(JobScheduler::class.java).allPendingJobs.isEmpty())
        assertTrue(NativeSignOut.pending(context))
        assertTrue(runCatching { NativeSync.run(context) }.isFailure)
        val result = StorageKeyProvider(context).withKey { directory, key -> JSONObject(NativeStorage.execute(directory.path, key, "{\"command\":\"sign_out\"}")) }
        assertTrue(result.getBoolean("ok"))
        assertTrue(result.getJSONObject("value").getBoolean("revoked"))
        NativeSignOut.save(context, "confirmed")
        assertEquals("confirmed", NativeSignOut.stage(context))
        // Clearing app data terminates instrumentation; the shell verifies removal.
        InstrumentationRegistry.getInstrumentation().sendStatus(0, android.os.Bundle().apply { putString("stream", "SIGIL_SIGN_OUT_REVOKED\n") })
        assertTrue(NativeSignOut.erase(context))
    }
    @Test fun removalClearedKeysAndBackgroundWork() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        check(context.packageName == "org.sigil.compose.acceptance")
        assertFalse(java.io.File(context.noBackupFilesDir, "native/client.db").exists())
        assertFalse(java.io.File(context.noBackupFilesDir, "native/storage.key").exists())
        assertFalse(NativeSignOut.pending(context))
        assertTrue(context.getSystemService(JobScheduler::class.java).allPendingJobs.isEmpty())
        val keyStore = java.security.KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        assertFalse(keyStore.containsAlias("${context.packageName}/storage/native/v0"))
        val result = StorageKeyProvider(context).withKey { directory, key -> JSONObject(NativeStorage.execute(directory.path, key, "{\"command\":\"state\"}")) }
        assertTrue(result.getBoolean("ok"))
        assertEquals("new", result.getJSONObject("value").getString("phase"))
    }
}
