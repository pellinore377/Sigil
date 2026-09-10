package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.*
import org.junit.Assert.*
import org.sigil.*

class CameraTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    @Test fun cameraStaysInsideTheComposerAndRequiresCaptureThenUse() {
        val instrument = InstrumentationRegistry.getInstrumentation()
        instrument.uiAutomation.grantRuntimePermission(instrument.targetContext.packageName, android.Manifest.permission.CAMERA)
        var prepared = false
        var attempts = 0
        val commands = mutableListOf<String>()
        val chat = ChatSummary("self", "@sam:example.test", "", "", true, emptyList(), displayName = "Sam")
        ui.setContent { androidx.compose.runtime.CompositionLocalProvider(LocalCameraPanel provides { target, back, done -> CameraPanel(back) { bytes ->
            try {
                assertEquals("self", target["peer"]); assertTrue(bytes.size > 100); assertEquals(255, bytes[0].toInt() and 255); assertEquals(216, bytes[1].toInt() and 255)
                if (attempts++ == 0) throw java.io.IOException("Synthetic staging failure")
                prepared = true; done()
            }
            finally { bytes.fill(0) }
        } }) { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", selected = "self", chats = listOf(chat)), { name, _ -> commands += name }) } }
        val header = ui.onNodeWithTag("main-header").fetchSemanticsNode()
        ui.onNodeWithContentDescription("Attachments").performClick()
        ui.onNodeWithContentDescription("Camera").performClick()
        ui.waitUntil(10000) { ui.onAllNodesWithContentDescription("Take photo").filter(isEnabled()).fetchSemanticsNodes().isNotEmpty() }
        val currentHeader = ui.onNodeWithTag("main-header").fetchSemanticsNode()
        assertEquals(header.id, currentHeader.id)
        assertEquals(header.boundsInRoot, currentHeader.boundsInRoot)
        val composer = ui.onNodeWithTag("composer").fetchSemanticsNode().boundsInRoot
        assertTrue(ui.onNodeWithContentDescription("Take photo").fetchSemanticsNode().boundsInRoot.top > composer.bottom)
        assertFalse(prepared)
        ui.onNodeWithContentDescription("Take photo").performClick()
        ui.waitUntil(10000) { ui.onAllNodesWithText("Use photo").fetchSemanticsNodes().isNotEmpty() }
        assertFalse(prepared)
        ui.onNodeWithText("Use photo").performClick()
        ui.waitUntil(5000) { ui.onAllNodesWithText("Could not prepare this photo. You can try again.").fetchSemanticsNodes().isNotEmpty() }
        assertFalse(prepared)
        ui.onNodeWithText("Retake").assertIsDisplayed()
        ui.onNodeWithText("Use photo").performClick()
        ui.waitUntil(10000) { prepared }
        ui.onNodeWithTag("composer").assertIsDisplayed()
        assertFalse(commands.any { it in listOf("post", "file_send") })
    }
}
