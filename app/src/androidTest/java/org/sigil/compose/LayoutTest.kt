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
        ui.onNodeWithText("Record").performScrollTo().assertIsDisplayed()
        ui.onNodeWithText("Cancel", substring = false).performScrollTo().assertIsDisplayed()
        val layouts = mutableListOf<androidx.compose.ui.text.TextLayoutResult>()
        ui.onNodeWithText("Cancel", substring = false).performSemanticsAction(androidx.compose.ui.semantics.SemanticsActions.GetTextLayoutResult) { it(layouts) }
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
        val state = mutableStateOf(MessengerState(phase = "connected", chats = contacts, collectionsEnabled = true, collections = listOf(CollectionItem("friends", "Friends", "group"), CollectionItem("work", "Work", "work")), profileName = "Alex", address = "@alex:example.com"))
        ui.runOnUiThread { ui.activity.setSigilContent { SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { _, _ -> }) } }
        ui.onNodeWithContentDescription("New conversation").assertIsDisplayed()
        capture("inbox")
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
}
