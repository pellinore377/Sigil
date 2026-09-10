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
        fun await(phase: String, matches: (VoiceState) -> Boolean = { true }): VoiceState {
            val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(8)
            while (System.nanoTime() < deadline) {
                val value = events.poll(200, TimeUnit.MILLISECONDS)
                assertTrue(issues.toString(), issues.isEmpty())
                if (value?.phase == phase && matches(value)) return value
            }
            throw AssertionError("Microphone did not reach $phase")
        }
        try {
            recorder.start("synthetic", mapOf("thread_message" to "thread"))
            await("Recording")
            recorder.start("another conversation", mapOf("thread_message" to "different"))
            runBlocking { delay(2200) }
            recorder.pauseRecording()
            val paused = await("Recording") { it.paused }
            events.clear()
            runBlocking { delay(1200) }
            assertEquals(paused.seconds, await("Recording") { it.paused }.seconds)
            recorder.pauseRecording()
            await("Recording") { !it.paused }
            runBlocking { delay(1100) }
            recorder.stop()
            val ready = await("Ready")
            assertTrue(ready.seconds in 3..4)
            assertTrue(ready.levels.isNotEmpty())
            assertEquals(0, queued)
            recorder.playPreview()
            assertTrue(await("Ready") { it.playing }.playing)
            assertEquals(0, queued)
            recorder.pausePreview()
            assertFalse(await("Ready") { !it.playing }.playing)
            recorder.seek(1800)
            assertEquals(1800L, await("Ready") { it.position == 1800L }.position)
            recorder.send("An accompanying thought.")
            await("Idle")
            assertEquals(1, queued)
        } finally { recorder.close(); scope.cancel() }
    }
}
