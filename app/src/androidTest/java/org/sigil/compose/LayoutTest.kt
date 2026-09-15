package org.sigil.compose

import android.graphics.Bitmap
import androidx.activity.ComponentActivity
import androidx.compose.runtime.*
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.Density
import androidx.core.view.WindowCompat
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*
import java.io.File

class LayoutTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    private val contact = ChatSummary("peer", "@sam:example.com", "See you tomorrow.", "9:33am", true, emptyList(), displayName = "Sam", unread = 3, pinned = true, presence = "active")
    private fun message(id: String, mine: Boolean, text: String) = ChatMessage(id, if (mine) "me" else "sam", text, mine, "9:33am", "Read", false, emptyList(), emptyList(), null, true, timestamp = 1000, separator = "Today, 9:33am", readers = if (mine) listOf("sam") else emptyList())
    private fun capture(name: String) {
        ui.mainClock.advanceTimeBy(300)
        ui.waitForIdle()
        Thread.sleep(300)
        val bitmap = androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()
        File(ui.activity.cacheDir, "layout-$name.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        bitmap.recycle()
    }
    @Test fun accountAppearanceUpdatesAndroidBarsAndKeepsLargeTextControlsReachable() {
        val state = mutableStateOf(MessengerState(phase = "connected", chats = listOf(contact), profileName = "Sam", address = "@sam:example.com", ui = mapOf("appearance" to "Newsreader|Light|555555|false")))
        ui.runOnUiThread { ui.activity.setSigilContent {
            val density = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(density.density, 2f)) {
                SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { _, _ -> })
            }
        } }
        ui.onNodeWithContentDescription("Settings").performClick()
        ui.onNodeWithText("Media, downloads, and cache").assertExists()
        capture("large-settings")
        ui.onNodeWithText("Theme, typography, and layout").performScrollTo().performClick()
        ui.onNodeWithText("Colors & backgrounds").performClick()
        ui.onNodeWithText("Custom color").performScrollTo().performClick()
        ui.onNodeWithText("Apply color").assertIsDisplayed()
        capture("large-color")
        ui.onNodeWithText("Cancel").performClick()
        ui.runOnIdle {
            val bars = WindowCompat.getInsetsController(ui.activity.window, ui.activity.window.decorView)
            assertTrue(bars.isAppearanceLightStatusBars)
            assertTrue(bars.isAppearanceLightNavigationBars)
            state.value = state.value.copy(ui = mapOf("appearance" to "Google Sans Flex|Dark|555555|false"))
        }
        ui.waitForIdle()
        ui.runOnIdle {
            val bars = WindowCompat.getInsetsController(ui.activity.window, ui.activity.window.decorView)
            assertFalse(bars.isAppearanceLightStatusBars)
            assertFalse(bars.isAppearanceLightNavigationBars)
        }
        ui.onNode(hasContentDescription("Back") and hasAnyAncestor(hasTestTag("main-header"))).performClick()
        ui.onNodeWithText("Reset app appearance").performScrollTo().assertIsDisplayed()
        capture("large-appearance-dark")
    }
    @Test fun timelineAndCallControlsRemainReachableWithLargeText() {
        val state = mutableStateOf(MessengerState(phase = "connected", chats = listOf(contact), selected = "peer", messages = listOf(message("out", true, "A letter for tomorrow."), message("in", false, "Shall we meet at the library?"))))
        val commands = mutableListOf<String>()
        ui.runOnUiThread { ui.activity.setSigilContent {
            val density = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(density.density, 2f)) { SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { name, _ -> commands += name }) }
        } }
        ui.onNodeWithContentDescription("Start audio call").assertIsDisplayed()
        ui.onNodeWithContentDescription("Start video call").assertIsDisplayed()
        ui.onNodeWithTag("composer").assertIsDisplayed()
        capture("large-timeline")
        ui.onNodeWithContentDescription("Voice message").performClick()
        ui.onNodeWithContentDescription("Discard recording").assertIsDisplayed()
        ui.onNodeWithText("0:00", substring = false).assertIsDisplayed()
        val layouts = mutableListOf<androidx.compose.ui.text.TextLayoutResult>()
        ui.onNodeWithText("0:00", substring = false).performSemanticsAction(androidx.compose.ui.semantics.SemanticsActions.GetTextLayoutResult) { it(layouts) }
        capture("large-voice")
        val layout = layouts.single()
        assertEquals(1, layout.lineCount)
        assertTrue(layout.multiParagraph.height <= layout.size.height + 1f)
        assertTrue(layout.getLineRight(0) <= layout.size.width + 1f)
        val participants = listOf(CallParticipant("me", "self", "You", true, true, true, false, false), CallParticipant("sam", "peer", "Sam", false, true, true, false, false))
        ui.runOnIdle { state.value = state.value.copy(call = ActiveCall(CallSummary("call", "active", true, 1000, participants), "Sam", "connected", 123)) }
        ui.onNodeWithText("Mute").assertIsDisplayed()
        ui.onNodeWithText("Speaker").assertIsDisplayed()
        ui.onNodeWithText("End", substring = false).assertIsDisplayed()
        capture("large-call")
        ui.onNodeWithText("End", substring = false).performClick()
        ui.runOnIdle { assertTrue("call_end" in commands) }
    }
    @Test fun mockupSurfacesKeepTheirContentAndActionsAcrossAppearanceChanges() {
        val contacts = listOf(contact, contact.copy(id = "lee", address = "@lee:example.com", displayName = "Lee", presence = "away"), contact.copy(id = "group", address = "", displayName = "Library club", group = true, unread = 0))
        val state = mutableStateOf(MessengerState(phase = "connected", chats = contacts, collectionsEnabled = true, collections = listOf(CollectionItem("friends", "Friends", "group"), CollectionItem("work", "Work", "work")), profileName = "Alex", address = "@alex:example.com", searchHits = listOf(SearchHit("peer", "note", "sam", "Bring a notebook to the library.", "9:33", noted = true)), calls = listOf(CallSummary("history", "ended", true, 3000, listOf(CallParticipant("sam", "peer", "Sam", false, true, true, false, false)), name = "Sam", outgoing = true, time = "9:41", day = "Today", duration = 245, video = false), CallSummary("missed", "declined", true, 2000, emptyList(), name = "Lee", time = "8:20", day = "Today", missed = true), CallSummary("video", "ended", true, 1000, emptyList(), name = "Library club", outgoing = true, time = "17:30", day = "Yesterday", duration = 724, video = true))))
        ui.runOnUiThread { ui.activity.setSigilContent { SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { _, _ -> }) } }
        ui.onNodeWithContentDescription("New conversation").assertIsDisplayed()
        capture("inbox")
        for (mode in listOf("Light", "Dark")) {
            ui.runOnIdle { state.value = state.value.copy(ui = mapOf("appearance" to "Newsreader|$mode|555555|false")) }
            ui.onNodeWithContentDescription("Messages").performClick()
            capture("inbox-${mode.lowercase()}")
            ui.onNodeWithText("Sam", substring = false).performTouchInput { longClick() }
            ui.onNodeWithText("1 selected", substring = false).assertIsDisplayed()
            capture("selection-${mode.lowercase()}")
            ui.onNodeWithContentDescription("More conversation actions").performClick()
            capture("selection-menu-${mode.lowercase()}")
            androidx.test.espresso.Espresso.pressBack()
            ui.onNodeWithContentDescription("Cancel selection").performClick()
            ui.onNodeWithContentDescription("Calls").performClick()
            ui.onNodeWithContentDescription("New call").assertIsDisplayed().performClick()
            ui.onNodeWithText("New call").assertIsDisplayed()
            ui.onNodeWithText("Cancel").performClick()
            capture("calls-${mode.lowercase()}")
            ui.onNode(hasText("Missed") and isSelectable()).performClick()
            ui.onNodeWithText("Sam", substring = false).assertDoesNotExist()
            capture("missed-${mode.lowercase()}")
            ui.onNodeWithText("All", substring = false).performClick()
            ui.onNodeWithText("Sam", substring = false).performClick()
            ui.onNodeWithContentDescription("Audio call").assertIsDisplayed()
            ui.onNodeWithContentDescription("Message", substring = false).assertIsDisplayed()
            capture("call-detail-${mode.lowercase()}")
            ui.onNode(hasContentDescription("Back to calls")).performClick()
            ui.onNodeWithContentDescription("Notes").performClick()
            ui.onNodeWithContentDescription("Notes").assertIsSelected()
            capture("notes-${mode.lowercase()}")
            ui.onNodeWithContentDescription("Settings").performClick()
            capture("settings-${mode.lowercase()}")
            ui.onNodeWithText("Privacy", substring = false).performScrollTo().performClick()
            capture("privacy-${mode.lowercase()}")
            ui.onNode(hasContentDescription("Back") and hasAnyAncestor(hasTestTag("main-header"))).performClick()
        }
        ui.onNodeWithContentDescription("Settings").performClick()
        capture("settings")
        ui.onNodeWithText("Theme, typography, and layout").performScrollTo().performClick()
        capture("appearance")
        val messages = listOf(message("3", true, "I'll bring my notebook.").copy(reactions = listOf("❤️")), message("2", false, "Perfect. See you tomorrow!").copy(pinned = true), message("1", false, "Meet by the library at nine?"))
        for ((font, mode) in listOf("Newsreader" to "Light", "Newsreader" to "Dark", "Google Sans Flex" to "Light", "Google Sans Flex" to "Dark")) {
            ui.runOnIdle { state.value = state.value.copy(selected = "peer", messages = messages, ui = mapOf("appearance" to "$font|$mode|555555|false")) }
            ui.onNodeWithText("I'll bring my notebook.").assertIsDisplayed()
            ui.onNodeWithContentDescription("Encrypted message").assertDoesNotExist()
            capture("timeline-${font.replace(' ', '-')}-${mode.lowercase()}")
        }
        ui.runOnIdle { state.value = state.value.copy(selected = "group", typing = listOf("sam", "lee"), people = mapOf("sam" to "Sam", "lee" to "Lee"), messages = messages.map { if (it.mine) it.copy(readers = listOf("sam", "lee")) else it }, ui = mapOf("appearance" to "Newsreader|Light|555555|false")) }
        ui.onNodeWithContentDescription("Library club is typing").assertExists()
        capture("group-timeline")
        val people = listOf(CallParticipant("me", "self", "You", true, true, true, false, false), CallParticipant("sam", "peer", "Sam", false, true, true, false, false), CallParticipant("lee", "lee", "Lee", false, true, false, false, false), CallParticipant("jo", "jo", "Jo", false, true, true, false, false))
        for ((direct, camera) in listOf(true to false, true to true, false to false, false to true)) {
            val members = (if (direct) people.take(2) else people).map { it.copy(camera = camera) }
            ui.runOnIdle { state.value = state.value.copy(call = ActiveCall(CallSummary("call", "active", direct, 1000, members, canInvite = !direct), if (direct) "Sam" else "Library club", "connected", 756, camera = camera, levels = mapOf("sam" to .55f, "self" to .1f))) }
            ui.onNodeWithText(if (direct) "End" else "Leave", substring = false).assertIsDisplayed()
            ui.onNodeWithText(if (direct) "12:36" else "4 in call · 12:36").assertIsDisplayed()
            capture("${if (direct) "direct" else "group"}-${if (camera) "video" else "audio"}")
        }
    }
    @Test fun composerKeepsOneSurfaceAcrossPanelsAndKeyboard() {
        val state = mutableStateOf(MessengerState(phase = "connected", selected = "self", chats = listOf(contact.copy(id = "self", displayName = "Note to Self")), ui = mapOf("appearance" to "Newsreader|Light|555555|false")))
        ui.runOnUiThread { ui.activity.setSigilContent { SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { _, _ -> }) } }
        capture("composer-closed-light")
        ui.onNodeWithContentDescription("Attachments").performClick()
        ui.onNodeWithContentDescription("One-time location").assertIsDisplayed()
        capture("composer-attachments-light")
        ui.onNodeWithContentDescription("Close attachment panel").performClick()
        ui.onNodeWithTag("composer").performClick().performTextInput("A synthetic caption")
        capture("composer-keyboard-light")
        androidx.test.espresso.Espresso.pressBack()
        ui.runOnIdle { state.value = state.value.copy(voice = VoiceState(phase = "Ready", peer = "self", seconds = 4, levels = List(20) { .3f }, duration = 4000)) }
        ui.onNodeWithContentDescription("Send voice message").assertIsDisplayed()
        capture("composer-voice-light")
        ui.runOnIdle { state.value = state.value.copy(ui = mapOf("appearance" to "Newsreader|Dark|555555|false")) }
        capture("composer-voice-dark")
    }
    @Test fun mainChromeFrostOverScrolledContent() {
        val contacts = (1..24).map { contact.copy(id = "peer-$it", displayName = "Library contact $it", pinned = false) }
        val calls = (1..24).map { CallSummary("call-$it", "ended", true, 1000L - it, emptyList(), name = "Library contact $it", outgoing = true, time = "9:41", day = "Today", duration = 74) }
        val state = mutableStateOf(MessengerState(phase = "connected", chats = contacts, calls = calls, profileName = "Alex", address = "@alex:example.com"))
        ui.runOnUiThread { ui.activity.setSigilContent { SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { _, _ -> }) } }
        for (mode in listOf("Light", "Dark")) {
            ui.runOnIdle { state.value = state.value.copy(ui = mapOf("appearance" to "Newsreader|$mode|555555|false")) }
            for (tab in listOf("Messages", "Calls", "Settings")) {
                ui.onNodeWithContentDescription(tab).performClick()
                ui.onRoot().performTouchInput { swipeUp(startY = height * .75f, endY = height * .3f, durationMillis = 400) }
                capture("frost-${tab.lowercase()}-${mode.lowercase()}")
            }
        }
    }
    @Test fun glassChromeHasNoRectangularBandsBehindItsContent() {
        ui.runOnUiThread { ui.activity.setSigilContent {
            SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", profileName = "Alex", ui = mapOf("appearance" to "Newsreader|Light|555555|false")), { _, _ -> })
        } }
        ui.onNodeWithContentDescription("Settings").performClick()
        capture("glass-regression")
        val screenshot = androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()
        try {
            for ((tag, x) in listOf("main-header" to .8f, "main-navigation" to .08f)) {
                val bounds = ui.onNodeWithTag(tag).fetchSemanticsNode().boundsInWindow
                val samples = listOf(.3f, .5f, .7f).map { y -> screenshot.getPixel((bounds.left + bounds.width * x).toInt(), (bounds.top + bounds.height * y).toInt()) }
                for (channel in listOf<(Int) -> Int>(android.graphics.Color::red, android.graphics.Color::green, android.graphics.Color::blue)) {
                    assertTrue("Rectangular band in $tag: $samples", samples.maxOf(channel) - samples.minOf(channel) <= 4)
                }
            }
        } finally { screenshot.recycle() }
    }
}
