package org.sigil

import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import org.junit.Rule
import org.junit.Test
import kotlin.test.assertEquals

class MessengerTest {
    @get:Rule val ui = createComposeRule()
    private fun chat(verified: Boolean) = ChatSummary("peer", "@sam:example.com", "", "", verified,
        listOf(ChatDevice("peer", "ab".repeat(32), verified, false, false)))

    @Test fun unverified_device_cannot_send_even_with_a_draft() {
        val commands = mutableListOf<String>()
        ui.setContent { SigilApp(NativeCore::palette, NativeCore::analyze,
            MessengerState(phase = "connected", chats = listOf(chat(false)), selected = "peer"), { name, _ -> commands += name }) }
        ui.onNodeWithTag("composer").performTextInput("hello")
        ui.onNodeWithContentDescription("Send message").assertIsNotEnabled()
        ui.runOnIdle { assertEquals(emptyList(), commands) }
    }
    @Test fun send_preserves_draft_on_failure_and_clears_after_durable_success() {
        val state = mutableStateOf(MessengerState(phase = "connected", chats = listOf(chat(true)), selected = "peer"))
        val commands = mutableListOf<Pair<String, Map<String, Any?>>>()
        ui.setContent { SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { name, fields -> commands += name to fields }) }
        ui.onNodeWithTag("composer").performTextInput("hello")
        ui.onNodeWithContentDescription("Send message").performClick()
        ui.runOnIdle { assertEquals("hello", commands.single().second["text"]); state.value = state.value.copy(issue = "Synthetic storage failure") }
        ui.onNodeWithTag("composer").assertTextContains("hello")
        ui.runOnIdle { state.value = state.value.copy(sent = 1, issue = null) }
        ui.waitForIdle()
        ui.onNodeWithTag("composer").assert(SemanticsMatcher.expectValue(androidx.compose.ui.semantics.SemanticsProperties.EditableText, androidx.compose.ui.text.AnnotatedString("")))
    }
    @Test fun empty_account_has_no_sample_conversations() {
        ui.setContent { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected"), { _, _ -> }) }
        ui.onNodeWithText("Your correspondence starts here.").assertExists()
        ui.onNodeWithText("Alex Morgan").assertDoesNotExist()
        ui.onNodeWithText("Send locally").assertDoesNotExist()
    }
}
