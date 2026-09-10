package org.sigil.compose

import android.Manifest
import android.media.AudioManager
import androidx.compose.ui.test.*
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.*
import org.junit.Assert.*
import org.sigil.storage.StorageKeyProvider
import java.io.File

class FixtureKeyTest {
    @Test fun prepareIsolatedSyntheticStorage() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        Assume.assumeTrue(context.packageName.endsWith(".acceptance"))
        check(!File(context.noBackupFilesDir, "native/client.db").exists())
        StorageKeyProvider(context).withKey { _, key -> File(context.cacheDir, "acceptance.key").apply { writeBytes(key); android.system.Os.chmod(path, 384) } }
    }
}
class CallUiTest {
    @get:Rule val ui = createAndroidComposeRule<MainActivity>()
    @Test fun answerAndHangupUseTheNativeCallAndForegroundService() {
        val instrument = InstrumentationRegistry.getInstrumentation()
        val context = instrument.targetContext
        Assume.assumeTrue(context.packageName.endsWith(".acceptance"))
        instrument.uiAutomation.grantRuntimePermission(context.packageName, Manifest.permission.RECORD_AUDIO)
        if (android.os.Build.VERSION.SDK_INT >= 33) instrument.uiAutomation.grantRuntimePermission(context.packageName, Manifest.permission.POST_NOTIFICATIONS)
        val audio = context.getSystemService(AudioManager::class.java)
        val volume = audio.getStreamVolume(AudioManager.STREAM_VOICE_CALL)
        audio.setStreamVolume(AudioManager.STREAM_VOICE_CALL, 0, 0)
        try {
            ui.waitUntil(15000) { ui.onAllNodesWithText("Answer").fetchSemanticsNodes().isNotEmpty() }
            ui.onNodeWithText("Answer").performClick()
            val elapsed = SemanticsMatcher("Elapsed call time") { node -> node.config.getOrNull(androidx.compose.ui.semantics.SemanticsProperties.Text)?.any { it.text.matches(Regex("[0-9]+:[0-9]{2}")) } == true }
            try { ui.waitUntil(45000) { ui.onAllNodes(elapsed).fetchSemanticsNodes().isNotEmpty() } }
            catch (error: androidx.compose.ui.test.ComposeTimeoutException) { throw AssertionError(ui.onRoot().printToString(), error) }
            Thread.sleep(6000)
            val mode = InstrumentationRegistry.getArguments().getString("call_end")
            if (mode == "group") {
                val messenger = androidx.lifecycle.ViewModelProvider(ui.activity)[Messenger::class.java]
                ui.waitUntil(60000) {
                    val call = messenger.state.call
                    val other = call?.call?.participants?.singleOrNull { !it.own }
                    call?.call?.participants?.size == 2 && other?.name == "charlie" && call.connection == "connected" && call.levels.containsKey(other.id)
                }
                Thread.sleep(6000)
                ui.onNodeWithText("Leave").performClick()
            } else if (mode == "notification") {
                val notification = context.getSystemService(android.app.NotificationManager::class.java).activeNotifications.single { it.id == 7 }.notification
                notification.actions.single { it.title.toString() == "End" }.actionIntent.send()
            } else ui.onNodeWithText("End").performClick()
            ui.waitUntil(15000) { ui.onAllNodesWithText("End").fetchSemanticsNodes().isEmpty() }
            ui.waitUntil(35000) { CallService.owner == null }
        } finally { audio.setStreamVolume(AudioManager.STREAM_VOICE_CALL, volume, 0) }
    }
}
