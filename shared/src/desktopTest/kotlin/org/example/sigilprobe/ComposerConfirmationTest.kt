package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class ComposerConfirmationTest {
    @get:Rule val ui=createComposeRule()
    @Test fun form_confirmation_stages_before_explicit_send_and_back_clears_the_action() {
        val sent=mutableListOf<String>()
        val draft=TextFieldState("Caption")
        ui.setContent {MaterialTheme {Box(Modifier.width(412.dp).height(760.dp)) {
            ComposerPanel(draft,{""},true,false,{_,_->},"peer",VoiceState(),0,null){text,_,_->sent+=text}
        }}}
        ui.onNodeWithContentDescription("Attachments").performClick()
        listOf("Contact","Poll","Checklist","Recipe").forEach {ui.onNodeWithContentDescription(it).assertDoesNotExist()}
        ui.onNodeWithContentDescription("Create").performClick()
        ui.onNodeWithContentDescription("Note").performClick()
        ui.onNodeWithContentDescription("Attach").assertIsNotEnabled()
        ui.onNodeWithText("Your note").performTextInput("Remember the ferry")
        ui.onNodeWithContentDescription("Attach").assertIsEnabled().performClick()
        ui.onNodeWithText("Attach").assertDoesNotExist()
        ui.runOnIdle {assertTrue(sent.isEmpty())}
        ui.onNodeWithContentDescription("Send message").performClick()
        ui.runOnIdle {assertEquals(listOf("note::Remember the ferry;\n\nCaption"),sent)}
        ui.onNodeWithContentDescription("Edit Note").performClick()
        ui.onNodeWithContentDescription("Back to create").performClick()
        ui.onNodeWithContentDescription("Attach").assertDoesNotExist()
    }
    @Test fun formatting_remains_open_while_typing_and_applies_to_selection() {
        val draft=TextFieldState("Text")
        ui.setContent {MaterialTheme {Box(Modifier.width(412.dp).height(760.dp)) {
            ComposerPanel(draft,{""},true,false,{_,_->},"peer",VoiceState(),0,null){_,_,_->}
        }}}
        ui.onNodeWithContentDescription("Attachments").performClick()
        ui.onNodeWithContentDescription("Format").performClick()
        ui.onNodeWithTag("composer").performClick()
        ui.onNodeWithContentDescription("Bold").assertExists().performClick()
        ui.onNodeWithContentDescription("Bold").assertExists()
        ui.runOnIdle {assertTrue(draft.text.contains("bold::"))}
        val bounds=ui.onNodeWithTag("composer-panel").fetchSemanticsNode().boundsInRoot
        assertTrue(bounds.height<130f)
    }
    @Test fun location_completion_preserves_text_written_after_submission() {
        val draft=TextFieldState("Meet at the gate")
        var completed:(()->Unit)?=null
        var submitted:Map<String,Any?>?=null
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalPlacePanel provides {target,_,done->
            BuilderConfirm("Share place") {submitted=target;completed=done}
        }) {Box(Modifier.width(412.dp).height(700.dp)) {
            ComposerPanel(draft,{""},true,false,{_,_->},"peer",VoiceState(),0,null,attachmentTarget=mapOf("peer" to "peer","thread_message" to "root")){_,_,_->}
        }}}}
        ui.onNodeWithContentDescription("Attachments").performClick()
        ui.onNodeWithContentDescription("Drop a pin").performClick()
        ui.onNodeWithContentDescription("Share place").performClick()
        ui.runOnIdle {
            assertEquals("Meet at the gate",submitted?.get("location_caption"))
            assertEquals("root",submitted?.get("thread_message"))
            draft.edit {replace(0,length,"Bring a notebook")}
            completed!!()
            assertEquals("Bring a notebook",draft.text.toString())
        }
    }

}
