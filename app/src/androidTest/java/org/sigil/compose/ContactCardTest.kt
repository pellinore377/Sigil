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
    @Test fun contact_expansion_copies_explicitly_without_lookup_or_trust_changes() {
        var contact by mutableStateOf(ContactContent("@sam:example.test",RichText("Sam Example"),"01".repeat(32),"BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Sam Example\r\nEND:VCARD\r\n"))
        val commands=mutableListOf<String>()
        val chat=ChatSummary("self","@viewer:example.test","","",true,emptyList())
        ui.runOnUiThread { ui.activity.setSigilContent {
            val message=ChatMessage("contact","sam","Shared contact",true,"9:33","sent",false,emptyList(),emptyList(),null,true,timestamp=1000,parts=listOf(MessagePart("card","contact","Shared contact",contact=contact)))
            SigilApp(NativeCore::palette,NativeCore::analyze,MessengerState(phase="connected",chats=listOf(chat),selected="self",messages=listOf(message)),{name,_->commands+=name})
        } }
        ui.onNodeWithText("Open contact").performClick()
        ui.onNode(hasText("Sam Example") and hasAnyAncestor(isDialog())).assertIsDisplayed()
        ui.onNodeWithText("Copy address").performClick()
        val clipboard=ui.activity.getSystemService(android.content.ClipboardManager::class.java)
        ui.runOnIdle { assertEquals("@sam:example.test",clipboard.primaryClip!!.getItemAt(0).text.toString()) }
        ui.onNodeWithText("More details").performScrollTo().performClick()
        ui.onNodeWithText("Copy vCard").performScrollTo().performClick()
        ui.runOnIdle { assertEquals(contact.vcard,clipboard.primaryClip!!.getItemAt(0).text.toString()) }
        ui.onNodeWithContentDescription("Close contact").performScrollTo().performClick()
        ui.runOnIdle { contact=contact.copy(name=RichText("Private name",listOf(RichSpan(0,12,reveal="spoiler"))),vcard=null) }
        ui.onNodeWithText("Private name").assertDoesNotExist()
        ui.onNodeWithText("Open contact").performClick()
        ui.onNodeWithText("Private name").assertDoesNotExist()
        ui.onNodeWithText("Copy vCard").assertDoesNotExist()
        ui.runOnIdle { assertTrue(commands.none { it in listOf("find","confirm","contact_open","contact_request","contact_qr","post") }) }
        ui.onNodeWithContentDescription("Close contact").performScrollTo().performClick()
        ui.onNodeWithContentDescription("Message shared contact").performClick()
        ui.runOnIdle { assertEquals(1,commands.count { it=="contact_open" });assertTrue(commands.none { it in listOf("confirm","contact_request","contact_qr","post") }) }
    }
}
