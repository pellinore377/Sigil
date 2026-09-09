package org.sigil.compose

import android.app.Activity
import android.content.Intent
import android.media.projection.MediaProjectionManager
import android.view.accessibility.AccessibilityNodeInfo
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material3.Text
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.*
import org.junit.Assert.*

class CallScreenTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    @Test fun screenCaptureRequiresConsentAndStopsWithTheService() {
        val instrument = InstrumentationRegistry.getInstrumentation()
        val activity = ui.activity
        ProjectionFixture.frames.set(0); ProjectionFixture.failed.set(false); ProjectionFixture.stopped.set(false)
        ui.setContent { Box(Modifier.fillMaxSize().background(Color(0xffecebe8))) { Text("Synthetic screen sharing acceptance") } }
        lateinit var launch: androidx.activity.result.ActivityResultLauncher<Intent>
        ui.runOnIdle {
            launch = activity.activityResultRegistry.register("projection-fixture", ActivityResultContracts.StartActivityForResult()) { result ->
                if (result.resultCode == Activity.RESULT_OK) activity.startForegroundService(Intent(activity, ProjectionFixture::class.java).putExtra("result", result.data))
            }
            launch.launch(activity.getSystemService(MediaProjectionManager::class.java).createScreenCaptureIntent())
        }
        try {
            var consent: AccessibilityNodeInfo? = null
            val until = android.os.SystemClock.elapsedRealtime() + 10000
            while (consent == null && android.os.SystemClock.elapsedRealtime() < until) {
                val root = instrument.uiAutomation.rootInActiveWindow
                consent = listOf("Start now", "Share screen").asSequence().flatMap { root?.findAccessibilityNodeInfosByText(it).orEmpty().asSequence() }.firstOrNull { it.isClickable }
                if (consent == null) Thread.sleep(100)
            }
            assertEquals("Capture began before consent", 0, ProjectionFixture.frames.get())
            assertNotNull("System screen-sharing consent was not shown", consent)
            assertTrue(consent!!.performAction(AccessibilityNodeInfo.ACTION_CLICK))
            ui.waitUntil(15000) { ProjectionFixture.stopped.get() }
            assertFalse("Screen capture failed: ${ProjectionFixture.failure}", ProjectionFixture.failed.get()); assertTrue(ProjectionFixture.frames.get() >= 20)
            Thread.sleep(300); val stopped = ProjectionFixture.frames.get(); Thread.sleep(300); assertEquals(stopped, ProjectionFixture.frames.get())
        } finally { activity.stopService(Intent(activity, ProjectionFixture::class.java)); ui.runOnIdle { launch.unregister() } }
    }
}
