package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*

class HelpTest {
    @get:Rule val ui=createAndroidComposeRule<ComponentActivity>()
    @Test fun bare_help_opens_locally_without_sending_or_requesting_contact() {
        val commands=mutableListOf<String>()
        val chat=ChatSummary("self","@sam:example.test","","",false,emptyList())
        ui.runOnUiThread {ui.activity.setSigilContent {
            SigilApp(NativeCore::palette,NativeCore::analyze,MessengerState(phase="connected",chats=listOf(chat),selected="self"),{name,_->commands+=name})
        }}
        ui.onNodeWithTag("composer").performClick().performTextInput("help::wa")
        ui.onNodeWithText("Search SigilText").assertIsDisplayed().assertIsFocused()
        ui.onNodeWithContentDescription("Open help").assertIsEnabled()
        ui.onNodeWithText("wave").performClick()
        ui.onNodeWithText("Send cheat sheet").performScrollTo().assertIsNotEnabled()
        ui.onNodeWithContentDescription("Back to help").performClick()
        ui.onNodeWithContentDescription("Close help").performClick()
        ui.onNodeWithText("Search SigilText").assertDoesNotExist()
        assertFalse(commands.any {it in listOf("post","contact_request")})
    }
    @Test fun composer_reference_search_copy_and_share_are_separate_actions() {
        val commands=mutableListOf<Pair<String,Map<String,Any?>>>()
        val chat=ChatSummary("self","@sam:example.test","","",true,emptyList())
        ui.runOnUiThread {ui.activity.setSigilContent {
            SigilApp(NativeCore::palette,NativeCore::analyze,MessengerState(phase="connected",chats=listOf(chat),selected="self"),{name,fields->commands+=name to fields})
        }}
        ui.onNodeWithContentDescription("Attachments").performClick()
        ui.onNodeWithContentDescription("Create").performClick()
        ui.onNodeWithContentDescription("Help").performClick()
        ui.onNodeWithText("Search SigilText").performTextInput("WAVE")
        ui.onNodeWithText("wave").performClick()
        ui.onNodeWithText("Copy example").performScrollTo().performClick()
        ui.runOnIdle {assertEquals("wave::Hello;",ui.activity.getSystemService(android.content.ClipboardManager::class.java).primaryClip!!.getItemAt(0).text.toString())}
        assertTrue(commands.none {it.first=="post"})
        ui.onNodeWithText("Send cheat sheet").performScrollTo().performClick()
        val sent=commands.single {it.first=="post"}.second
        assertEquals("help::wave;",sent["text"])
        assertEquals(true,sent["rich"])
        ui.onNodeWithContentDescription("Back to help").performClick()
        ui.onNodeWithContentDescription("Back to create").performClick()
        ui.onNodeWithContentDescription("Back to attachments").assertIsDisplayed()
        assertEquals(1,commands.count {it.first=="post"})
    }
}
