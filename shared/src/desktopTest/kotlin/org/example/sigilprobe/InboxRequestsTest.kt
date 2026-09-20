package org.sigil

import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import org.junit.Rule
import org.junit.Test
import kotlin.test.assertEquals

class InboxRequestsTest {
    @get:Rule val ui = createComposeRule()
    private val request = ChatSummary("peer", "@sam:example.com", "", "", false, emptyList(), request = "incoming", hidden = true, contactOnly = true)

    @Test fun requests_survive_conversation_filters_and_open_acceptance() {
        val selected = mutableStateOf(false)
        val chats = mutableStateOf(listOf(request))
        val decisions = mutableListOf<Map<String, Any?>>()
        ui.setContent { SigilTheme(Appearance(), palette = NativeCore::palette) {
            if (selected.value) ContactRequestPanel(chats.value.single(), false) { name, fields ->
                if (name == "contact_request") decisions += fields
            } else Inbox(MessengerState(phase = "connected", chats = chats.value, collectionsEnabled = true), "work", {}, emptySet(), {},
                { assertEquals("peer", it); selected.value = true }, { null })
        } }
        ui.onNodeWithText("Message requests (1)").assertExists()
        ui.onNodeWithText("No conversations in this collection").assertDoesNotExist()
        ui.onAllNodesWithText("sam").assertCountEquals(1)
        ui.onNodeWithText("Wants to connect").assertExists()
        ui.onNodeWithText("sam").performClick()
        ui.onNodeWithText("Decline").assertExists()
        ui.onNodeWithText("Accept").performClick()
        ui.runOnIdle {
            assertEquals("peer", decisions.single()["peer"])
            assertEquals("accept", decisions.single()["action"])
            chats.value = listOf(request.copy(request = "accepted"))
            selected.value = false
        }
        ui.onNodeWithText("Message requests (1)").assertDoesNotExist()
        ui.onNodeWithText("No conversations in this collection").assertExists()
    }

    @Test fun outgoing_request_is_visible_before_the_first_message() {
        ui.setContent { SigilTheme(Appearance(), palette = NativeCore::palette) {
            Inbox(MessengerState(phase = "connected", chats = listOf(request.copy(request = "pending", hidden = false))), "", {}, emptySet(), {}, {}, { null })
        } }
        ui.onNodeWithText("sam").assertExists()
        ui.onNodeWithText("Pending").assertExists()
        ui.onNodeWithText("No conversations yet").assertDoesNotExist()
    }
}
