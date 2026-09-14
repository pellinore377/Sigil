package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.runtime.*
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*

class ComposerTrayTest {
    @get:Rule val ui=createAndroidComposeRule<ComponentActivity>()
    private val chat=ChatSummary("self","@sam:example.test","","",true,emptyList())
    private fun setup(voice:VoiceState=VoiceState(),command:(String,Map<String,Any?>)->Unit={_,_->}) {
        ui.runOnUiThread {ui.activity.setSigilContent {SigilApp(NativeCore::palette,NativeCore::analyze,MessengerState(phase="connected",chats=listOf(chat),selected="self",timelineLoaded=true,voice=voice),command)}}
    }
    private fun open(tool:String) {
        ui.onNodeWithContentDescription("Attachments").performClick();ui.onNodeWithContentDescription("Create").performClick()
        ui.onNodeWithText("Search tools").performTextInput(tool)
        ui.onNodeWithContentDescription(tool).performClick()
    }
    @Test fun structured_content_stages_above_a_separate_caption_and_sends_once() {
        val posts=mutableListOf<Map<String,Any?>>()
        setup {name,fields->if(name=="post")posts+=fields}
        open("Progress")
        ui.waitUntil(5000){ui.onAllNodes(hasText("Add to message") and isEnabled()).fetchSemanticsNodes().isNotEmpty()}
        ui.onNodeWithText("Add to message").assertIsEnabled().performClick()
        assertTrue(posts.isEmpty())
        ui.onNodeWithTag("composer").performClick().performTextInput("Almost there")
        ui.onNodeWithContentDescription("Send message").performClick()
        ui.waitForIdle()
        assertEquals(1,posts.size);assertEquals("progress::0;\n\nAlmost there",posts.single()["text"])
    }
    @Test fun keyboard_does_not_cover_the_builder_and_panel_is_above_the_composer() {
        setup();open("Recipe")
        ui.onNodeWithText("Title").performClick().performTextInput("Pasta")
        fun keyboardHeight()=androidx.core.view.WindowInsetsCompat.toWindowInsetsCompat(ui.activity.window.decorView.rootWindowInsets).getInsets(androidx.core.view.WindowInsetsCompat.Type.ime()).bottom
        ui.waitUntil(5000){keyboardHeight()>0}
        ui.waitForIdle()
        val panel=ui.onNodeWithTag("composer-panel").fetchSemanticsNode().boundsInWindow
        val input=ui.onNodeWithTag("composer").fetchSemanticsNode().boundsInWindow
        assertTrue("Panel must be above caption: $panel $input",panel.bottom<=input.top+1)
        ui.onNodeWithText("Title").assertIsDisplayed()
        val keyboard=keyboardHeight()
        assertTrue(input.bottom<=ui.activity.window.decorView.height-keyboard+2)
        androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()?.let {bitmap->
            java.io.File(ui.activity.cacheDir,"recipe-tray.png").outputStream().use {bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)}
            bitmap.recycle()
        }
    }
    @Test fun voice_draft_leaves_caption_field_available_until_explicit_send() {
        val commands=mutableListOf<Pair<String,Map<String,Any?>>>()
        setup(VoiceState(phase="Ready",peer="self",seconds=4,levels=List(20){.3f},duration=4000)){name,fields->commands+=name to fields}
        ui.onNodeWithTag("composer").performClick().performTextInput("Listen to this")
        assertFalse(commands.any {it.first=="record_send"})
        ui.onNodeWithContentDescription("Send voice message").performClick()
        assertEquals("Listen to this",commands.single {it.first=="record_send"}.second["caption"])
    }
}
