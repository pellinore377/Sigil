package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.*
import org.junit.*
import org.junit.Assert.*
import org.sigil.VoiceState
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit

class VoiceRecorderTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    @Test fun captureUsesMemoryAndOnlyQueuesAfterSend() {
        val instrument = InstrumentationRegistry.getInstrumentation()
        instrument.uiAutomation.grantRuntimePermission(instrument.targetContext.packageName, android.Manifest.permission.RECORD_AUDIO)
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
        val events = LinkedBlockingQueue<VoiceState>()
        val issues = LinkedBlockingQueue<String>()
        var queued = 0
        val recorder = VoiceRecorder(scope, { peer, bytes, target ->
            assertEquals("synthetic", peer)
            assertEquals("thread", target["thread_message"])
            assertEquals("An accompanying thought.", target["caption"])
            assertTrue(bytes.size > 7)
            assertEquals(255, bytes[0].toInt() and 255)
            assertEquals(240, bytes[1].toInt() and 240)
            queued++
        }, events::offer, issues::offer)
        fun await(phase: String): VoiceState {
            val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(8)
            while (System.nanoTime() < deadline) {
                val value = events.poll(200, TimeUnit.MILLISECONDS)
                assertTrue(issues.toString(), issues.isEmpty())
                if (value?.phase == phase) return value
            }
            throw AssertionError("Microphone did not reach $phase")
        }
        try {
            recorder.start("synthetic", mapOf("thread_message" to "thread"))
            await("Recording")
            recorder.start("another conversation", mapOf("thread_message" to "different"))
            runBlocking { delay(2200) }
            recorder.stop()
            val ready = await("Ready")
            assertTrue(ready.seconds >= 2)
            assertTrue(ready.levels.isNotEmpty())
            assertEquals(0, queued)
            recorder.playPreview()
            assertTrue(await("Ready").playing)
            assertEquals(0, queued)
            recorder.pausePreview()
            assertFalse(await("Ready").playing)
            recorder.send("An accompanying thought.")
            await("Idle")
            assertEquals(1, queued)
        } finally { recorder.close(); scope.cancel() }
    }
}
