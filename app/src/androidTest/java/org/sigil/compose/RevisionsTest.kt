package org.sigil.compose

import android.graphics.Bitmap
import androidx.activity.ComponentActivity
import androidx.compose.runtime.*
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.core.view.WindowInsetsCompat
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*
import java.io.File

class RevisionsTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    private val chat = ChatSummary("peer", "@sam:example.com", "A little correspondence", "9:33am", true, emptyList(), displayName = "Sam")
    private fun show(content: @Composable () -> Unit) { ui.runOnUiThread { ui.activity.setSigilContent(content) }; ui.waitForIdle() }
    private fun capture(name: String) {
        ui.waitForIdle(); Thread.sleep(300)
        val bitmap = InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()
        File(ui.activity.cacheDir, "revision-$name.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }; bitmap.recycle()
    }
    @Test fun mainTabsKeepNavigationAndAppearanceStartsWithTimeline() {
        show { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", chats = listOf(chat), profileName = "Alex", address = "@alex:example.com"), { _, _ -> }) }
        listOf("Calls", "Settings", "Messages", "Settings").forEach { tab ->
            ui.onNodeWithContentDescription(tab).performClick()
            listOf("Calls", "Settings", "Messages").forEach { ui.onNodeWithContentDescription(it).assertIsDisplayed() }
        }
        capture("settings")
        ui.onNodeWithText("Theme, typography, and layout").performScrollTo().performClick()
        ui.onNodeWithText("Dinner still on for tonight?").assertIsDisplayed()
        capture("appearance")
    }
    @Test fun structuredEntriesGrowAndRemainAboveTheKeyboardInBothFonts() {
        val state = mutableStateOf(MessengerState(phase = "connected", chats = listOf(chat), selected = "peer"))
        val posts = mutableListOf<Map<String, Any?>>()
        show { SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { name, fields -> if (name == "post") posts += fields }) }
        for (font in listOf("Newsreader", "Google Sans Flex")) {
            ui.runOnIdle { state.value = state.value.copy(ui = mapOf("appearance" to "$font|Light|555555|false")) }
            ui.onNodeWithContentDescription("Attachments").performClick()
            ui.onNodeWithContentDescription("Close attachment panel").assertIsDisplayed()
            ui.onNodeWithContentDescription("Create").performClick()
            ui.onNodeWithContentDescription("Poll").performClick()
            ui.onNodeWithText("Question").performClick().performTextReplacement("Where shall we meet?")
            ui.onNodeWithText("Option 1").performScrollTo().performClick().performTextReplacement("Library")
            ui.onNodeWithText("Option 2").performScrollTo().performClick().performTextReplacement("Garden")
            ui.onNodeWithText("Option 3").performScrollTo().performClick().performTextReplacement("Gallery")
            ui.onNodeWithText("Option 4").assertExists()
            ui.waitForIdle(); Thread.sleep(500)
            val field = ui.onNodeWithText("Option 3").fetchSemanticsNode().boundsInWindow
            val view = ui.activity.window.decorView
            val ime = WindowInsetsCompat.toWindowInsetsCompat(view.rootWindowInsets).getInsets(WindowInsetsCompat.Type.ime()).bottom
            assertTrue("Keyboard should be open", ime > 0)
            assertTrue("Entry is under the keyboard: $field", field.bottom <= view.height - ime + 2)
            capture("poll-${font.replace(' ', '-')}")
            ui.onNodeWithText("Send poll").performScrollTo().assertIsEnabled().performClick()
            val expected = "poll::Where shall we meet?\n- Library\n- Garden\n- Gallery;"
            assertEquals(expected, posts.last()["text"])
            assertEquals(true, posts.last()["rich"])
            ui.runOnIdle { state.value = state.value.copy(sent = state.value.sent + 1, sentText = expected) }
            ui.onNodeWithContentDescription("Attachments").assertIsDisplayed()
        }
        assertEquals(2, posts.size)
    }
    @Test fun stoppedRecordingAttachesToComposerUntilExplicitSend() {
        val state = mutableStateOf(MessengerState(phase = "connected", chats = listOf(chat), selected = "peer"))
        val commands = mutableListOf<String>()
        show { SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { name, _ -> commands += name }) }
        ui.onNodeWithContentDescription("Voice message").performClick()
        capture("voice")
        ui.onNodeWithText("Record").performClick()
        ui.runOnIdle { state.value = state.value.copy(voice = VoiceState("Recording", "peer", 3, listOf(.1f, .5f, .8f, .3f))) }
        ui.onNodeWithText("Done").performClick()
        assertTrue("record_stop" in commands)
        assertFalse("record_send" in commands)
        ui.runOnIdle { state.value = state.value.copy(voice = state.value.voice.copy(phase = "Ready")) }
        ui.onNodeWithContentDescription("Play voice preview").assertIsDisplayed().performClick()
        ui.onNodeWithContentDescription("Discard voice message").assertIsDisplayed()
        assertTrue("record_preview" in commands)
        assertFalse("record_send" in commands)
        capture("voice-draft")
        ui.onNodeWithContentDescription("Send voice message").performClick()
        assertTrue("record_send" in commands)
    }
    @Test fun scannerStaysBetweenInstructionsAndCodeSwitch() {
        val instrument = InstrumentationRegistry.getInstrumentation()
        instrument.uiAutomation.grantRuntimePermission(instrument.targetContext.packageName, android.Manifest.permission.CAMERA)
        val flow = JSONObject().put("stage", "show_offer").put("width", 21).put("cells", "0".repeat(441))
        show { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(), { _, _ -> }, overlay = { DeviceLinkDialog(flow, false, null) { _, _ -> } }) }
        ui.onNodeWithText("Scan the other device").performClick()
        ui.onNodeWithText("Show my code").assertIsDisplayed()
        val finder = ui.onNodeWithTag("link-viewfinder").fetchSemanticsNode().boundsInWindow
        val button = ui.onNodeWithText("Show my code").fetchSemanticsNode().boundsInWindow
        assertTrue(finder.bottom <= button.top)
        ui.onNodeWithText("Show my code").performClick()
        ui.onNodeWithContentDescription("Device linking QR code").assertIsDisplayed()
    }
}
