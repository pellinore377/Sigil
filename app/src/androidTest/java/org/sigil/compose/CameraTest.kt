package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.*
import org.junit.Assert.*

class CameraTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    @Test fun cameraRequiresExplicitCaptureAndSend() {
        val instrument = InstrumentationRegistry.getInstrumentation()
        instrument.uiAutomation.grantRuntimePermission(instrument.targetContext.packageName, android.Manifest.permission.CAMERA)
        var sent = false
        ui.setContent { CameraSheet({}) { bytes ->
            try { assertTrue(bytes.size > 100); assertEquals(255, bytes[0].toInt() and 255); assertEquals(216, bytes[1].toInt() and 255); sent = true }
            finally { bytes.fill(0) }
        } }
        ui.waitUntil(10000) { ui.onAllNodesWithText("Take photo").filter(isEnabled()).fetchSemanticsNodes().isNotEmpty() }
        assertFalse(sent)
        ui.onNodeWithText("Take photo").performClick()
        ui.waitUntil(10000) { ui.onAllNodesWithText("Send photo").fetchSemanticsNodes().isNotEmpty() }
        assertFalse(sent)
        ui.onNodeWithText("Send photo").performClick()
        ui.waitUntil(10000) { sent }
    }
}
