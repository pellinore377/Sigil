package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.*
import org.junit.Assert.*
import org.sigil.*
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat

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
        val header = ui.onNodeWithTag("conversation-header").fetchSemanticsNode()
        ui.onNodeWithContentDescription("Attachments").performClick()
        ui.onNodeWithContentDescription("Camera").performClick()
        ui.waitUntil(10000) { ui.onAllNodesWithContentDescription("Take photo").filter(isEnabled()).fetchSemanticsNodes().isNotEmpty() }
        val currentHeader = ui.onNodeWithTag("conversation-header").fetchSemanticsNode()
        assertEquals(header.id, currentHeader.id)
        assertEquals(header.boundsInRoot, currentHeader.boundsInRoot)
        val composer = ui.onNodeWithTag("composer").fetchSemanticsNode().boundsInRoot
        assertTrue(ui.onNodeWithContentDescription("Take photo").fetchSemanticsNode().boundsInRoot.bottom <= composer.top)
        fun assertViewfinderClearance() {
            val viewfinder=ui.onNodeWithTag("camera-viewfinder").fetchSemanticsNode().boundsInRoot
            val headerBounds=ui.onNodeWithTag("conversation-header").fetchSemanticsNode().boundsInRoot
            val input=ui.onNodeWithTag("composer").fetchSemanticsNode().boundsInRoot
            assertTrue("Camera must begin below conversation header: viewfinder=$viewfinder header=$headerBounds panel=${ui.onNodeWithTag("composer-panel").fetchSemanticsNode().boundsInRoot} composer=$input",viewfinder.top>=headerBounds.bottom)
            assertTrue("Camera must leave composer visible",viewfinder.bottom<=input.top)
        }
        assertViewfinderClearance()
        ui.onNodeWithTag("composer").performClick().performTextInput("Synthetic caption")
        ui.waitUntil(5000) {ViewCompat.getRootWindowInsets(ui.activity.window.decorView)?.isVisible(WindowInsetsCompat.Type.ime())==true}
        ui.waitForIdle()
        assertViewfinderClearance()
        ui.runOnUiThread {WindowInsetsControllerCompat(ui.activity.window,ui.activity.window.decorView).hide(WindowInsetsCompat.Type.ime())}
        ui.waitUntil(5000) {ViewCompat.getRootWindowInsets(ui.activity.window.decorView)?.isVisible(WindowInsetsCompat.Type.ime())==false}
        ui.waitForIdle()
        assertViewfinderClearance()
        assertFalse(prepared)
        ui.onNodeWithContentDescription("Take photo").performClick()
        ui.waitUntil(10000) { ui.onAllNodesWithContentDescription("Attach photo").fetchSemanticsNodes().isNotEmpty() }
        assertFalse(prepared)
        ui.onNodeWithContentDescription("Attach photo").performClick()
        ui.waitUntil(5000) { ui.onAllNodesWithText("Could not prepare this photo. You can try again.").fetchSemanticsNodes().isNotEmpty() }
        assertFalse(prepared)
        ui.onNodeWithContentDescription("Retake").assertIsDisplayed()
        ui.onNodeWithContentDescription("Attach photo").performClick()
        ui.waitUntil(10000) { prepared }
        ui.onNodeWithTag("composer").assertIsDisplayed()
        assertFalse(commands.any { it in listOf("post", "file_send") })
    }
}
