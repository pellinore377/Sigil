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
        ui.mainClock.autoAdvance = false
        ui.onNodeWithText("Theme, typography, and layout").performScrollTo().performClick()
        ui.mainClock.advanceTimeBy(80)
        capture("appearance-opening")
        ui.mainClock.advanceTimeBy(80)
        capture("appearance-opening-160")
        ui.mainClock.autoAdvance = true; ui.waitForIdle()
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
    @Test fun conversationMovesContentInsideStationarySurfaces() {
        val state = mutableStateOf(MessengerState(phase = "connected", ui = mapOf("appearance" to "Newsreader|Dark|555555|false"), chats = listOf(chat.copy(ui = mapOf("chat_theme" to "287C54|false")))))
        show { SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { name, _ -> if (name == "close") state.value = state.value.copy(selected = null) }) }
        val header = ui.onNodeWithTag("main-header").fetchSemanticsNode().id
        val footer = ui.onNodeWithTag("footer-surface").fetchSemanticsNode().id
        val initialHeaderHeight = ui.onNodeWithTag("main-header").getUnclippedBoundsInRoot().let { it.bottom - it.top }
        val footerBottom = ui.onNodeWithTag("footer-surface").getUnclippedBoundsInRoot().bottom
        val initialHeight = ui.onNodeWithTag("footer-surface").getUnclippedBoundsInRoot().let { it.bottom - it.top }
        fun headerColor(): androidx.compose.ui.graphics.Color {
            val pixels = ui.onNodeWithTag("main-header").captureToImage().toPixelMap()
            return pixels[pixels.width / 2, pixels.height - 4]
        }
        val background = headerColor()
        ui.mainClock.autoAdvance = false
        ui.runOnIdle { state.value = state.value.copy(selected = "peer", messages = listOf(ChatMessage("motion", "sam", "A letter arriving from below", false, "9:33am", "", false, emptyList(), emptyList(), null, true))) }
        ui.mainClock.advanceTimeBy(64)
        val first = ui.onNodeWithTag("timeline-body").getUnclippedBoundsInRoot().top
        assertTrue("Timeline did not enter from below", first.value > 100f)
        assertEquals(header, ui.onNodeWithTag("main-header").fetchSemanticsNode().id)
        assertEquals(initialHeaderHeight, ui.onNodeWithTag("main-header").getUnclippedBoundsInRoot().let { it.bottom - it.top })
        assertTrue("Main header elements did not slide away", ui.onNodeWithText("Sigil").getUnclippedBoundsInRoot().left.value < 0f)
        assertEquals(footer, ui.onNodeWithTag("footer-surface").fetchSemanticsNode().id)
        assertEquals(footerBottom, ui.onNodeWithTag("footer-surface").getUnclippedBoundsInRoot().bottom)
        val growingHeight = ui.onNodeWithTag("footer-surface").getUnclippedBoundsInRoot().let { it.bottom - it.top }
        assertTrue("Footer height did not start growing", growingHeight > initialHeight)
        assertEquals("Header color changed before the movement finished", background, headerColor())
        capture("opening-64")
        ui.mainClock.advanceTimeBy(80)
        assertTrue(ui.onNodeWithTag("timeline-body").getUnclippedBoundsInRoot().top < first)
        assertEquals(background, headerColor())
        ui.mainClock.advanceTimeBy(64)
        assertTrue("Floating button stopped above the gesture area", ui.onNodeWithContentDescription("New conversation").getUnclippedBoundsInRoot().top > footerBottom)
        capture("opening-208")
        ui.mainClock.advanceTimeBy(96)
        val earlyTint = headerColor()
        val growingHeaderHeight = ui.onNodeWithTag("main-header").getUnclippedBoundsInRoot().let { it.bottom - it.top }
        assertTrue("Header did not grow with its tint", growingHeaderHeight > initialHeaderHeight)
        ui.runOnIdle { state.value = state.value.copy(busy = true) }
        ui.mainClock.advanceTimeBy(48)
        val laterTint = headerColor()
        val label = ui.onNodeWithText("Sam").captureToImage().toPixelMap()
        assertTrue("Header foreground became dark during its tint", (0 until label.height).sumOf { y -> (0 until label.width).count { x -> label[x, y].red > .6f && label[x, y].green > .6f && label[x, y].blue > .6f } } > 20)
        capture("opening-352")
        ui.mainClock.autoAdvance = true; ui.waitForIdle()
        assertTrue("Header jumped to its final height", growingHeaderHeight < ui.onNodeWithTag("main-header").getUnclippedBoundsInRoot().let { it.bottom - it.top })
        assertTrue("Footer height jumped to its final size", growingHeight < ui.onNodeWithTag("footer-surface").getUnclippedBoundsInRoot().let { it.bottom - it.top })
        val settled = headerColor()
        assertNotEquals("Header tint did not animate", earlyTint, laterTint)
        ui.runOnIdle { state.value = state.value.copy(busy = false, messages = state.value.messages.map { it.copy(delivery = "Delivered") }) }
        ui.mainClock.advanceTimeBy(500)
        assertEquals(header, ui.onNodeWithTag("main-header").fetchSemanticsNode().id)
        assertEquals("Header changed after settling", settled, headerColor())
        ui.onNodeWithText("A letter arriving from below").assertIsDisplayed()
        ui.onNodeWithContentDescription("Back").performClick()
        assertEquals(header, ui.onNodeWithTag("main-header").fetchSemanticsNode().id)
        assertEquals(footer, ui.onNodeWithTag("footer-surface").fetchSemanticsNode().id)
        ui.onNodeWithTag("main-navigation").assertIsDisplayed()
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
