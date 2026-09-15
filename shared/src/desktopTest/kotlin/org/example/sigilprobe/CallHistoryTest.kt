package org.sigil

import androidx.compose.runtime.*
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class CallHistoryTest {
    @get:Rule val ui = createComposeRule()
    private fun call(id: String, name: String, missed: Boolean = false) = CallSummary(id, "ended", true, 100, listOf(CallParticipant("member-$id", "peer-$id", name, false, true, false, false, false, address = "@$id:example.com")), name = name, time = "9:41", day = "Today", missed = missed)
    @Test fun missedFilterUsesRecordedOutcomeInsteadOfGuessingFromIncomingEnded() {
        val state = MessengerState(phase = "connected", calls = listOf(call("a", "Alice", true), call("b", "Bob")))
        ui.setContent { SigilTheme(Appearance(), palette = NativeCore::palette) { CallHistoryPage(state, { _, _ -> }) } }
        ui.onNodeWithText("Today").assertExists()
        ui.onNodeWithText("Bob").assertIsDisplayed()
        ui.onNode(hasText("Missed") and isSelectable()).performClick()
        ui.onNodeWithText("Alice").assertIsDisplayed()
        ui.onNodeWithText("Bob").assertDoesNotExist()
        ui.onNodeWithText("All", substring = false).performClick()
        ui.onNodeWithText("Bob").assertIsDisplayed()
    }
    @Test fun detailActionsUseHistoryForRedialAndResolvedContactForMessage() {
        val record = call("maya", "Maya").copy(duration = 74, video = true)
        val contact = ChatSummary("actual-contact", "@maya:example.com", "", "", true, emptyList(), displayName = "Maya")
        val state = MessengerState(phase = "connected", calls = listOf(record), chats = listOf(contact))
        val selected = mutableStateOf<String?>(null)
        val commands = mutableListOf<Pair<String, Map<String, Any?>>>()
        ui.setContent { SigilTheme(Appearance(), palette = NativeCore::palette) { CallHistoryPage(state, { name, fields -> commands += name to fields }, selected.value) { selected.value = it } } }
        ui.onNodeWithText("Maya").performClick()
        ui.runOnIdle { assertTrue(commands.isEmpty()); assertEquals("maya", selected.value) }
        ui.onNodeWithText("Incoming · 1:14").assertExists()
        ui.onNodeWithContentDescription("Video call").performClick()
        ui.runOnIdle { assertEquals("call_redial" to mapOf("call" to "maya", "video" to true, "name" to "Maya"), commands.single()) }
        ui.onNodeWithContentDescription("Message").performClick()
        ui.runOnIdle { assertEquals("open" to mapOf("peer" to "actual-contact"), commands.last()); assertNull(selected.value) }
    }
    @Test fun legacyIncomingIsNotCalledMissedAndUnknownDurationIsNotZero() {
        val legacy = call("legacy", "Legacy")
        assertEquals("Incoming", callStatus(legacy))
        assertNull(legacy.duration)
        assertNull(legacy.video)
        assertNull(callContact(legacy.copy(direct = false), listOf(ChatSummary("peer-legacy", "", "", "", true, emptyList()))))
    }
}
