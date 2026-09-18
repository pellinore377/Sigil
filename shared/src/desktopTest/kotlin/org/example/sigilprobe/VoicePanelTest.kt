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

class VoicePanelTest {
    @get:Rule val ui=createComposeRule()
    @Test fun opening_voice_waits_for_a_tap_before_recording() {
        val commands=mutableListOf<String>()
        ui.setContent {MaterialTheme {Box(Modifier.width(412.dp).height(760.dp)) {
            ComposerPanel(TextFieldState(),{""},true,false,{action,_->commands+=action},"peer",VoiceState(),0,null){_,_,_->}
        }}}
        ui.onNodeWithContentDescription("Voice message").performClick()
        ui.onNodeWithContentDescription("Tap to record your voice").assertIsDisplayed()
        ui.onNodeWithContentDescription("Attach").assertIsNotEnabled()
        ui.runOnIdle {assertTrue(commands.none {it.startsWith("record_")})}
        ui.onNodeWithContentDescription("Tap to record your voice").performClick()
        ui.runOnIdle {assertEquals(listOf("record_start"),commands.filter {it.startsWith("record_")})}
        ui.onNodeWithContentDescription("Cancel").performClick()
        ui.runOnIdle {assertEquals("record_cancel",commands.last {it.startsWith("record_")})}
        ui.onNodeWithContentDescription("Tap to record your voice").assertDoesNotExist()
    }
    @Test fun recording_offers_restart_stop_and_attach() {
        val commands=mutableListOf<String>()
        var voice by mutableStateOf(VoiceState())
        ui.setContent {MaterialTheme {Box(Modifier.width(412.dp).height(760.dp)) {
            ComposerPanel(TextFieldState(),{""},true,false,{action,_->commands+=action},"peer",voice,0,null){_,_,_->}
        }}}
        ui.onNodeWithContentDescription("Voice message").performClick()
        ui.onNodeWithContentDescription("Record").performClick()
        ui.runOnIdle {assertEquals(listOf("record_start"),commands.filter {it.startsWith("record_")});voice=VoiceState("Recording","peer",4,List(48){.5f})}
        ui.onNodeWithText("00:04").assertIsDisplayed()
        ui.onNodeWithContentDescription("Cancel").assertDoesNotExist()
        ui.onNodeWithContentDescription("Stop").performClick()
        ui.runOnIdle {assertEquals("record_stop",commands.last())}
        ui.onNodeWithContentDescription("Restart").performClick()
        ui.runOnIdle {assertEquals(listOf("record_cancel","record_start"),commands.takeLast(2))}
        ui.onNodeWithContentDescription("Attach").assertIsEnabled().performClick()
        ui.runOnIdle {assertEquals("record_stop",commands.last());voice=VoiceState("Ready","peer",4,List(48){.5f},duration=4000)}
        ui.onNodeWithContentDescription("Play voice preview").assertIsDisplayed()
        ui.onNodeWithTag("audio-time").assertTextEquals("00:04")
        ui.onNodeWithText("Add text").assertIsDisplayed()
        ui.onNodeWithContentDescription("Send voice message").assertIsEnabled()
        ui.onNodeWithContentDescription("Discard voice message").performClick()
        ui.runOnIdle {assertEquals("record_cancel",commands.last())}
    }
}
