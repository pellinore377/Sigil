package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.runtime.*
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*

class ContactCardTest {
    @get:Rule val ui=createAndroidComposeRule<ComponentActivity>()
    @Test fun a_received_contact_reads_as_one_row_and_messages_without_trust_changes() {
        var contact by mutableStateOf(ContactContent("@sam:example.test",RichText("Sam Example"),"01".repeat(32),null))
        val commands=mutableListOf<Pair<String,Map<String,Any?>>>()
        val chat=ChatSummary("self","@viewer:example.test","","",true,emptyList())
        ui.runOnUiThread { ui.activity.setSigilContent {
            val message=ChatMessage("contact","sam","Shared contact",false,"9:33","read",false,emptyList(),emptyList(),null,true,timestamp=1000,parts=listOf(MessagePart("card","contact","Shared contact",contact=contact)))
            SigilApp(NativeCore::palette,NativeCore::analyze,MessengerState(phase="connected",chats=listOf(chat),selected="self",messages=listOf(message)),{name,fields->commands+=name to fields})
        } }
        ui.onNodeWithContentDescription("Contact, Sam Example, @sam:example.test").assertIsDisplayed()
        ui.onNodeWithContentDescription("Message Sam Example").performClick()
        ui.runOnIdle {
            val opens=commands.filter { it.first=="contact_open" }
            assertEquals(1,opens.size)
            assertEquals("card",opens[0].second["card"])
            assertTrue(commands.none { it.first in listOf("confirm","contact_request","contact_qr","post") })
        }
        ui.runOnIdle { contact=contact.copy(name=RichText("Private name",listOf(RichSpan(0,12,reveal="spoiler")))) }
        ui.onNodeWithContentDescription("Contact, hidden name, @sam:example.test").assertExists()
        ui.onNodeWithContentDescription("Private name",substring=true).assertDoesNotExist()
        ui.onNodeWithContentDescription("Message this contact").assertExists()
    }
}
