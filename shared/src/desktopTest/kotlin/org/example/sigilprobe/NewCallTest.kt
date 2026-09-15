package org.sigil

import androidx.compose.runtime.*
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class NewCallTest {
    @get:Rule val ui = createComposeRule()
    private val maya = ChatSummary("maya-id", "@maya:example.com", "", "", true, emptyList(), displayName = "Maya")
    private fun state() = MessengerState(phase = "connected", chats = listOf(maya, maya.copy(id = "self", displayName = "Note to Self"), maya.copy(id = "unverified", displayName = "Unverified", verified = false), maya.copy(id = "hidden", displayName = "Hidden", hidden = true)))
    @Test fun selectingContactDoesNotCallAndExplicitVideoUsesItsRealTarget() {
        val calls = mutableListOf<Pair<String, Map<String, Any?>>>()
        var closed = 0
        ui.setContent { SigilTheme(Appearance(), palette = NativeCore::palette) { NewCallDialog(state(), { name, fields -> calls += name to fields }, { closed++ }) } }
        ui.onNodeWithText("Audio call").assertIsNotEnabled()
        ui.onNodeWithText("Note to Self").assertDoesNotExist()
        ui.onNodeWithText("Unverified", substring = false).assertDoesNotExist()
        ui.onNodeWithText("Hidden", substring = false).assertDoesNotExist()
        ui.onNodeWithText("Maya", substring = false).performClick()
        ui.runOnIdle { assertTrue(calls.isEmpty()); assertEquals(0, closed) }
        ui.onNodeWithText("Video call").performClick()
        ui.runOnIdle { assertEquals(listOf("call_start" to mapOf<String, Any?>("peer" to "maya-id", "video" to true)), calls); assertEquals(1, closed) }
    }
    @Test fun unavailableVideoAndChangingContactEligibilityCannotStartACall() {
        val current = mutableStateOf(state())
        val calls = mutableListOf<Pair<String, Map<String, Any?>>>()
        ui.setContent { SigilTheme(Appearance(), palette = NativeCore::palette) { CompositionLocalProvider(LocalClientFeatures provides ClientFeatures(videoCalls = false)) { NewCallDialog(current.value, { name, fields -> calls += name to fields }, {}) } } }
        ui.onNodeWithText("Video call").assertDoesNotExist()
        ui.onNodeWithText("Maya", substring = false).performClick()
        ui.runOnIdle { current.value = current.value.copy(busy = true) }
        ui.onNodeWithText("Audio call").assertIsNotEnabled()
        ui.runOnIdle { current.value = current.value.copy(busy = false, chats = listOf(maya.copy(verified = false))) }
        ui.onNodeWithText("Audio call").assertIsNotEnabled()
        ui.runOnIdle { assertTrue(calls.isEmpty()) }
    }
}
