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
    @Test fun mainTabsKeepNavigationAndAppearancePagesShareTheHeader() {
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
        ui.mainClock.autoAdvance = false
        ui.onNodeWithText("Theme, typography, and layout").performScrollTo().performClick()
        ui.mainClock.advanceTimeBy(80)
        capture("appearance-opening")
        ui.mainClock.advanceTimeBy(80)
        capture("appearance-opening-160")
        ui.mainClock.autoAdvance = true; ui.waitForIdle()
        ui.onNodeWithText("Colors & backgrounds").performClick()
        assertEquals(header.id, ui.onNodeWithTag("main-header").fetchSemanticsNode().id)
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
            ui.onNodeWithContentDescription("Attach").assertIsEnabled().performClick()
            ui.onNodeWithContentDescription("Send message").performClick()
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
        ui.onNodeWithText("Colors & backgrounds").performClick()
        for (accent in listOf("Rose", "Moss", "Lavender")) ui.onNodeWithContentDescription(accent).performScrollTo().performClick()
        assertEquals(3, saved.size)
        for (old in saved.take(2)) {
            ui.runOnIdle { state.value = state.value.copy(ui = mapOf("appearance" to old)) }
            ui.onNodeWithContentDescription("Lavender").assertIsSelected()
        }
        ui.runOnIdle { state.value = state.value.copy(ui = mapOf("appearance" to saved.last())) }
        ui.onNodeWithContentDescription("Lavender").assertIsSelected()
    }
    @Test fun replyGestureShowsItsActionBeforeReleaseAndCanBeCancelled() {
        val message = ChatMessage("message", "sam", "A short thought.", false, "9:33am", "Read", false, emptyList(), emptyList(), null, true)
        show { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", chats = listOf(chat), selected = "peer", messages = listOf(message)), { _, _ -> }) }
        val bubble = ui.onNodeWithText("A short thought.")
        bubble.performTouchInput { down(center); moveBy(androidx.compose.ui.geometry.Offset(180f, 0f)) }
        ui.onNodeWithText("Reply", substring = false).assertIsDisplayed()
        bubble.performTouchInput { cancel() }
        ui.onNodeWithText("Replying to A short thought.").assertDoesNotExist()
        bubble.performTouchInput { down(center); moveBy(androidx.compose.ui.geometry.Offset(180f, 0f)); up() }
        ui.onNodeWithText("Replying to A short thought.").assertIsDisplayed()
    }
    @Test fun conversationEntersBeforeItsFloatingChromeAndReversesOnBack() {
        val state = mutableStateOf(MessengerState(phase = "connected", chats = listOf(chat)))
        show { SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { _, _ -> }) }
        repeat(2) {
            ui.mainClock.autoAdvance = false
            ui.runOnUiThread { state.value = state.value.copy(selected = "peer") }
            ui.mainClock.advanceTimeBy(96)
            ui.onNodeWithTag("timeline-body").assertExists()
            ui.onNodeWithTag("conversation-header").assertIsNotDisplayed()
            ui.onNodeWithTag("conversation-footer").assertIsNotDisplayed()
            ui.mainClock.advanceTimeBy(400)
            ui.onNodeWithTag("conversation-header").assertIsDisplayed()
            ui.onNodeWithTag("conversation-footer").assertIsDisplayed()
            ui.onNodeWithTag("main-header").assertDoesNotExist()
            ui.runOnUiThread { state.value = state.value.copy(selected = null) }
            ui.mainClock.advanceTimeBy(400)
            ui.onNodeWithTag("conversation-header").assertDoesNotExist()
            ui.onNodeWithTag("conversation-footer").assertDoesNotExist()
            ui.onNodeWithTag("main-header").assertIsDisplayed()
            ui.onNodeWithTag("main-navigation").assertIsDisplayed()
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
        assertTrue("record_start" in commands)
        ui.runOnIdle { state.value = state.value.copy(voice = VoiceState("Recording", "peer", 3, listOf(.1f, .5f, .8f, .3f))) }
        ui.onNodeWithContentDescription("Stop recording").performClick()
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
    @Test fun closingAttachmentsReversesThePanelHeight() {
        show { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", chats = listOf(chat), selected = "peer"), { _, _ -> }) }
        val closed = ui.onNodeWithTag("composer-panel").fetchSemanticsNode().boundsInWindow.height
        ui.onNodeWithContentDescription("Attachments").performClick()
        val expanded = ui.onNodeWithTag("composer-panel").fetchSemanticsNode().boundsInWindow.height
        assertTrue(expanded > closed)
        ui.mainClock.autoAdvance = false
        ui.onNodeWithContentDescription("Close attachment panel").performClick()
        ui.mainClock.advanceTimeBy(96)
        val closing = ui.onNodeWithTag("composer-panel").fetchSemanticsNode().boundsInWindow.height
        assertTrue("Closing jumped to its final position", closing < expanded && closing > closed)
        ui.onNodeWithContentDescription("Photos").assertExists()
        ui.mainClock.autoAdvance = true
        ui.waitForIdle()
        assertEquals(closed, ui.onNodeWithTag("composer-panel").fetchSemanticsNode().boundsInWindow.height)
        ui.onNodeWithContentDescription("Photos").assertDoesNotExist()
    }
    @Test fun createFormsAnimateOutBeforeRemoval() {
        show { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", chats = listOf(chat), selected = "peer"), { _, _ -> }) }
        ui.onNodeWithContentDescription("Attachments").performClick()
        ui.onNodeWithContentDescription("Create").performClick()
        for (kind in listOf("Note", "Checklist", "Poll", "Reminder", "Task", "Timer")) {
            ui.onNodeWithContentDescription("Create page 1").performClick()
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
        val expanded = ui.onNodeWithTag("composer-panel").fetchSemanticsNode().boundsInWindow.height
        ui.onNodeWithContentDescription("Format").performClick()
        ui.onNodeWithContentDescription("Continue writing").performClick()
        ui.waitUntil(5000) { imeHeight() > 0 }
        Thread.sleep(600); ui.waitForIdle()
        ui.runOnUiThread { androidx.core.view.WindowCompat.getInsetsController(ui.activity.window, view).hide(WindowInsetsCompat.Type.ime()) }
        ui.waitUntil(5000) { imeHeight() == 0 }
        Thread.sleep(300); ui.waitForIdle()
        ui.onNodeWithContentDescription("Bold").assertIsDisplayed()
        ui.onNodeWithContentDescription("Back to attachments").performClick()
        assertEquals(expanded, ui.onNodeWithTag("composer-panel").fetchSemanticsNode().boundsInWindow.height, 3f)
        ui.onNodeWithContentDescription("Create").assertIsDisplayed()
    }
    @Test fun scannerStaysBelowInstructionsAndCancelRemainsReachable() {
        val instrument = InstrumentationRegistry.getInstrumentation()
        instrument.uiAutomation.grantRuntimePermission(instrument.targetContext.packageName, android.Manifest.permission.CAMERA)
        val flow = JSONObject().put("stage", "scan_offer")
        val commands=mutableListOf<String>()
        show { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(), { _, _ -> }, overlay = { DeviceLinkDialog(flow, false, null) { action, _ -> commands+=action } }) }
        val finder = ui.onNodeWithTag("link-viewfinder").fetchSemanticsNode().boundsInWindow
        val instructions=ui.onNodeWithText("Scan the code shown by your new device. Keep both devices with you throughout setup.").fetchSemanticsNode().boundsInWindow
        val cancel=ui.onNodeWithText("Cancel").assertIsDisplayed().fetchSemanticsNode().boundsInWindow
        assertTrue(finder.top>=instructions.bottom)
        assertTrue(cancel.bottom<=finder.top)
        ui.onNodeWithText("Cancel").performClick()
        assertEquals(listOf("cancel"),commands)
    }
    @Test fun tabDirectionFollowsPositionAndSubpagesKeepTheHeader() {
        show { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", chats = listOf(chat)), { _, _ -> }) }
        val header = ui.onNodeWithTag("main-header").fetchSemanticsNode().id
        for ((label, page, fromRight) in listOf(Triple("Calls", "calls", true), Triple("Settings", "settings", true), Triple("Calls", "calls", false), Triple("Messages", "inbox", false))) {
            ui.mainClock.autoAdvance = false
            ui.onNodeWithContentDescription(label).performClick()
            ui.mainClock.advanceTimeBy(80)
            val moving = ui.onNodeWithTag("main-page-$page").getUnclippedBoundsInRoot().left.value
            assertTrue("$label entered from the wrong side: $moving", if (fromRight) moving > 0f else moving < 0f)
            assertEquals(header, ui.onNodeWithTag("main-header").fetchSemanticsNode().id)
            ui.mainClock.autoAdvance = true; ui.waitForIdle()
        }
        ui.onNodeWithContentDescription("Settings").performClick()
        ui.onNodeWithText("Theme, typography, and layout").performScrollTo().performClick()
        assertEquals(header, ui.onNodeWithTag("main-header").fetchSemanticsNode().id)
        ui.onNodeWithTag("main-navigation").assertDoesNotExist()
        ui.onNodeWithContentDescription("Back").performClick()
        assertEquals(header, ui.onNodeWithTag("main-header").fetchSemanticsNode().id)
        ui.onNodeWithTag("main-navigation").assertIsDisplayed()
    }
    @Test fun settledConversationChromeDoesNotRestartOnSync() {
        val state = mutableStateOf(MessengerState(phase = "connected", selected = "peer", chats = listOf(chat),
            messages = listOf(ChatMessage("motion", "sam", "A letter arriving from below", false, "9:33am", "", false, emptyList(), emptyList(), null, true))))
        show { SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { _, _ -> }) }
        val header = ui.onNodeWithTag("conversation-header").fetchSemanticsNode()
        val footer = ui.onNodeWithTag("conversation-footer").fetchSemanticsNode()
        for (busy in listOf(true, false)) {
            ui.mainClock.autoAdvance = false
            ui.runOnIdle { state.value = state.value.copy(busy = busy, messages = state.value.messages.map { it.copy(delivery = "Delivered") }) }
            ui.mainClock.advanceTimeBy(64)
            val current = ui.onNodeWithTag("conversation-header").fetchSemanticsNode()
            assertEquals(header.id, current.id)
            assertEquals(header.boundsInWindow, current.boundsInWindow)
            assertEquals(footer.boundsInWindow, ui.onNodeWithTag("conversation-footer").fetchSemanticsNode().boundsInWindow)
            ui.onNodeWithText("A letter arriving from below").assertIsDisplayed()
            ui.mainClock.autoAdvance = true
            ui.waitForIdle()
        }
    }
    @Test fun devicesHideFingerprintsAndOfferRemovalAndRenaming() {
        val fingerprint = "abcd".repeat(16)
        val state = MessengerState(phase = "connected", devices = listOf(AccountDevice("old-device", false, "Travel phone", false, fingerprint = fingerprint)))
        val commands = mutableListOf<Pair<String, Map<String, Any?>>>()
        show { SigilApp(NativeCore::palette, NativeCore::analyze, state, { name, fields -> commands += name to fields }) }
        ui.onNodeWithContentDescription("Settings").performClick()
        ui.onNodeWithText("Linked devices and verification").performScrollTo().performClick()
        ui.onNodeWithText("Travel phone").assertIsDisplayed()
        ui.onNodeWithText(fingerprint.chunked(4).joinToString(" ")).assertDoesNotExist()
        ui.onNodeWithContentDescription("Rename Travel phone").performClick()
        ui.onNode(hasSetTextAction()).performTextReplacement("Old Android")
        ui.onNodeWithText("Save").performClick()
        assertTrue(commands.any { it.first == "organize" })
        ui.onNodeWithText("Remove device").performClick()
        ui.onNodeWithText("Sign out device").performClick()
        assertTrue(commands.any { it.first == "revoke_device" && it.second["device"] == "old-device" })
    }
    @Test fun accentColorsOutgoingBubblesButtonsAndDarkSelection() {
        val state = mutableStateOf(MessengerState(phase = "connected", chats = listOf(chat), selected = "peer", messages = listOf(
            ChatMessage("outgoing", "self", "A green letter", true, "9:33am", "Sent", false, emptyList(), emptyList(), null, true)
        ), ui = mapOf("appearance" to "Newsreader|Dark|336644|false")))
        var accent = androidx.compose.ui.graphics.Color.Unspecified
        var handle = androidx.compose.ui.graphics.Color.Unspecified
        show { SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { _, _ -> }, overlay = {
            accent = androidx.compose.material3.MaterialTheme.colorScheme.primary
            handle = androidx.compose.foundation.text.selection.LocalTextSelectionColors.current.handleColor
        }) }
        assertEquals(accent, handle)
        assertNotEquals(androidx.compose.ui.graphics.Color.Black, handle)
        fun countAccent(node: SemanticsNodeInteraction): Int {
            val pixels = node.captureToImage().toPixelMap()
            return (0 until pixels.height).sumOf { y -> (0 until pixels.width).count { x -> pixels[x,y] == accent } }
        }
        assertTrue("Outgoing bubble does not use accent", countAccent(ui.onNodeWithTag("timeline")) > 100)
        assertTrue("Voice button does not use accent", countAccent(ui.onNodeWithContentDescription("Voice message")) > 100)
        ui.onNodeWithTag("composer").performClick().performTextInput("Visible caret")
        // Sample across a blink cycle; text fields keep the accent caret in dark mode.
        var caretPixels = 0
        repeat(4) { ui.mainClock.advanceTimeBy(150); caretPixels = maxOf(caretPixels, countAccent(ui.onNodeWithTag("composer"))) }
        assertTrue("Dark composer cursor has no accent pixels", caretPixels > 2)
    }
}
