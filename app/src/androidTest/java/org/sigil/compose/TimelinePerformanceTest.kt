package org.sigil.compose

import android.os.Handler
import android.os.HandlerThread
import android.os.SystemClock
import android.view.FrameMetrics
import android.view.Window
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*
import java.util.Collections

class TimelinePerformanceTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    @Test fun long_timeline_keeps_a_bounded_rendered_window_while_scrolling() {
        val state = timelineFixture()
        ui.runOnUiThread { ui.activity.setSigilContent { SigilApp(NativeCore::palette, NativeCore::analyze, state, { _, _ -> }) } }
        ui.onNodeWithText(state.messages.first().text).assertIsDisplayed()
        assertTrue(ui.onAllNodes(hasText("Letter ", substring = true), useUnmergedTree = true).fetchSemanticsNodes().size < 40)
        ui.onNodeWithTag("timeline").performScrollToIndex(998)
        assertTrue(ui.onAllNodes(hasText("Letter ", substring = true), useUnmergedTree = true).fetchSemanticsNodes().size < 40)
        ui.onNodeWithTag("timeline").performScrollToIndex(0)
        ui.onNodeWithText(state.messages.first().text).assertIsDisplayed()
    }
}
class RealTimelinePerformanceTest {
    @Test fun sample_android_frames_without_a_compose_test_clock() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        org.junit.Assume.assumeTrue(instrumentation.targetContext.packageName.endsWith(".acceptance"))
        ActivityScenario.launch(TimelineFixtureActivity::class.java).use { scenario ->
            val frames = Collections.synchronizedList(mutableListOf<Long>())
            val thread = HandlerThread("test-frame-metrics").apply { start() }
            var width = 0
            var height = 0
            val listener = Window.OnFrameMetricsAvailableListener { _, metrics, _ ->
                val duration = metrics.getMetric(FrameMetrics.TOTAL_DURATION)
                if (duration > 0) frames += duration
            }
            SystemClock.sleep(1000)
            scenario.onActivity { width = it.window.decorView.width; height = it.window.decorView.height }
            fun swipe() {
                val command = "input swipe ${width / 2} ${height / 3} ${width / 2} ${height * 3 / 4} 450"
                android.os.ParcelFileDescriptor.AutoCloseInputStream(instrumentation.uiAutomation.executeShellCommand(command)).use { while (it.read() != -1) {} }
            }
            repeat(2) { swipe() }
            SystemClock.sleep(500)
            scenario.onActivity { it.window.addOnFrameMetricsAvailableListener(listener, Handler(thread.looper)) }
            try { repeat(8) { swipe() }; SystemClock.sleep(500) }
            finally {
                scenario.onActivity { it.window.removeOnFrameMetricsAvailableListener(listener) }
                thread.quitSafely(); thread.join()
            }
            val sorted = synchronized(frames) { frames.sorted() }
            assertTrue("No frame metrics were delivered", sorted.size >= 20)
            fun percentile(p: Int) = sorted[(sorted.size - 1) * p / 100] / 1_000_000.0
            instrumentation.sendStatus(0, android.os.Bundle().apply {
                putString("stream", "\nSIGIL_TIMELINE_REAL_DEBUG frames=${sorted.size} p50_ms=${percentile(50)} p95_ms=${percentile(95)} over_16ms=${sorted.count { it > 16_666_667 }}\n")
            })
        }
    }
}
