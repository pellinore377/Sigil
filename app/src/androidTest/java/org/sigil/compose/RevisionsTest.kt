package org.sigil.compose

import android.graphics.Bitmap
import androidx.activity.ComponentActivity
import androidx.compose.runtime.*
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.core.view.WindowInsetsCompat
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.Before
import org.sigil.*
import java.io.File

class RevisionsTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    @Before fun matchApplicationWindow() { ui.runOnUiThread {
        ui.activity.window.setSoftInputMode(android.view.WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE)
    } }
    private val chat = ChatSummary("peer", "@sam:example.com", "A little correspondence", "9:33am", true, emptyList(), displayName = "Sam")
    private fun show(content: @Composable () -> Unit) { ui.runOnUiThread { ui.activity.setSigilContent(content) }; ui.waitForIdle() }
    private fun capture(name: String) {
        ui.waitForIdle(); Thread.sleep(300)
        val bitmap = InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()
        File(ui.activity.cacheDir, "revision-$name.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }; bitmap.recycle()
    }
    @Test fun mainTabsKeepNavigationAndAppearanceStartsWithTimeline() {
        show { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", chats = listOf(chat), profileName = "Alex", address = "@alex:example.com"), { _, _ -> }) }
        val header = ui.onNodeWithTag("main-header").fetchSemanticsNode()
        listOf("Calls", "Settings", "Messages", "Settings").forEach { tab ->
            ui.mainClock.autoAdvance = false
            ui.onNodeWithContentDescription(tab).performClick()
            ui.mainClock.advanceTimeBy(64)
            assertEquals(header.id, ui.onNodeWithTag("main-header").fetchSemanticsNode().id)
            assertEquals(header.boundsInWindow, ui.onNodeWithTag("main-header").fetchSemanticsNode().boundsInWindow)
            ui.mainClock.autoAdvance = true
            ui.waitForIdle()
            listOf("Calls", "Settings", "Messages").forEach { ui.onNodeWithContentDescription(it).assertIsDisplayed() }
            ui.onNodeWithContentDescription("Back").assertDoesNotExist()
            val footer = ui.onNodeWithTag("main-navigation").captureToImage().toPixelMap()
            val headerPixels = ui.onNodeWithTag("main-header").captureToImage().toPixelMap()
            assertEquals(headerPixels[2, 2], footer[2, 2])
            assertNotEquals(footer[2, 2], footer[footer.width / 2, 2])
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
    @Test fun delayedThemeSavesNeverReplaceTheLatestSelection() {
        val state = mutableStateOf(MessengerState(phase = "connected", chats = listOf(chat)))
        val saved = mutableListOf<String>()
        show { SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { name, fields ->
            if (name == "organize") ((fields["value"] as? Map<*, *>)?.get("UiSetting") as? Map<*, *>)?.get("value")?.let { saved += it as String }
        }) }
        ui.onNodeWithContentDescription("Settings").performClick()
        ui.onNodeWithText("Theme, typography, and layout").performScrollTo().performClick()
        for (accent in listOf("Rose", "Moss", "Lavender")) ui.onNodeWithContentDescription(accent).performScrollTo().performClick()
        assertEquals(3, saved.size)
        for (old in saved.take(2)) {
            ui.runOnIdle { state.value = state.value.copy(ui = mapOf("appearance" to old)) }
            ui.onNodeWithContentDescription("Lavender").assertIsSelected()
        }
        ui.runOnIdle { state.value = state.value.copy(ui = mapOf("appearance" to saved.last())) }
        ui.onNodeWithContentDescription("Lavender").assertIsSelected()
    }
    @Test fun conversationReusesTheMainHeaderContainer() {
        val state = mutableStateOf(MessengerState(phase = "connected", chats = listOf(chat)))
        show { SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { _, _ -> }) }
        val header = ui.onNodeWithTag("main-header").fetchSemanticsNode()
        for (selected in listOf("peer", null, "peer", null)) {
            ui.mainClock.autoAdvance = false
            ui.runOnUiThread { state.value = state.value.copy(selected = selected) }
            ui.mainClock.advanceTimeBy(96)
            val current = ui.onNodeWithTag("main-header").fetchSemanticsNode()
            assertEquals(header.id, current.id)
            assertEquals(header.boundsInWindow, current.boundsInWindow)
            ui.mainClock.autoAdvance = true
            ui.waitForIdle()
        }
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
    @Test fun closingAttachmentsAnimatesTheComposerDown() {
        show { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", chats = listOf(chat), selected = "peer"), { _, _ -> }) }
        val closed = ui.onNodeWithTag("composer").fetchSemanticsNode().boundsInWindow.top
        ui.onNodeWithContentDescription("Attachments").performClick()
        val expanded = ui.onNodeWithTag("composer").fetchSemanticsNode().boundsInWindow.top
        assertTrue(expanded < closed)
        ui.mainClock.autoAdvance = false
        ui.onNodeWithContentDescription("Close attachment panel").performClick()
        ui.mainClock.advanceTimeBy(96)
        val closing = ui.onNodeWithTag("composer").fetchSemanticsNode().boundsInWindow.top
        assertTrue("Closing jumped to its final position", closing > expanded && closing < closed)
        ui.onNodeWithContentDescription("Photos").assertExists()
        ui.mainClock.autoAdvance = true
        ui.waitForIdle()
        assertEquals(closed, ui.onNodeWithTag("composer").fetchSemanticsNode().boundsInWindow.top)
        ui.onNodeWithContentDescription("Photos").assertDoesNotExist()
    }
    @Test fun createFormsAnimateOutBeforeRemoval() {
        show { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", chats = listOf(chat), selected = "peer"), { _, _ -> }) }
        ui.onNodeWithContentDescription("Attachments").performClick()
        ui.onNodeWithContentDescription("Create").performClick()
        for (kind in listOf("Note", "Checklist", "Poll", "Reminder", "Task", "Timer")) {
            ui.onNodeWithContentDescription(kind).performScrollTo().performClick()
            ui.onAllNodes(isDialog()).assertCountEquals(0)
            val opened = ui.onNodeWithContentDescription("Back to create").fetchSemanticsNode().boundsInWindow.left
            ui.mainClock.autoAdvance = false
            ui.onNodeWithContentDescription("Back to create").performClick()
            ui.mainClock.advanceTimeBy(64)
            assertTrue(ui.onNodeWithContentDescription("Back to create").fetchSemanticsNode().boundsInWindow.left > opened)
            ui.mainClock.autoAdvance = true
            ui.waitForIdle()
            ui.onNodeWithContentDescription("Back to create").assertDoesNotExist()
        }
    }
    @Test fun formattingThenClosingKeyboardKeepsFullAttachmentHeight() {
        show { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", chats = listOf(chat), selected = "peer"), { _, _ -> }) }
        val view = ui.activity.window.decorView
        fun imeHeight() = WindowInsetsCompat.toWindowInsetsCompat(view.rootWindowInsets).getInsets(WindowInsetsCompat.Type.ime()).bottom
        ui.runOnUiThread { androidx.core.view.WindowCompat.getInsetsController(ui.activity.window, view).hide(WindowInsetsCompat.Type.ime()) }
        ui.waitUntil(5000) { imeHeight() == 0 }
        ui.onNodeWithContentDescription("Attachments").performClick()
        ui.onNodeWithContentDescription("Format").performClick()
        ui.onNodeWithText("Continue writing").performClick()
        ui.waitUntil(5000) { imeHeight() > 0 }
        Thread.sleep(600); ui.waitForIdle()
        val keyboard = ui.onNodeWithTag("composer").fetchSemanticsNode().boundsInWindow.top
        ui.runOnUiThread { androidx.core.view.WindowCompat.getInsetsController(ui.activity.window, view).hide(WindowInsetsCompat.Type.ime()) }
        ui.waitUntil(5000) { imeHeight() == 0 }
        Thread.sleep(300); ui.waitForIdle()
        ui.onNodeWithContentDescription("Attachments").performClick()
        assertEquals(keyboard, ui.onNodeWithTag("composer").fetchSemanticsNode().boundsInWindow.top, 3f)
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
