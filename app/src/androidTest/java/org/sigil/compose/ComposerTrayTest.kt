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
    private fun setup(voice:VoiceState=VoiceState(),messages:List<ChatMessage> = emptyList(),command:(String,Map<String,Any?>)->Unit={_,_->}) {
        ui.runOnUiThread {ui.activity.setSigilContent {SigilApp(NativeCore::palette,NativeCore::analyze,MessengerState(phase="connected",chats=listOf(chat),selected="self",timelineLoaded=true,voice=voice,messages=messages),command)}}
    }
    private fun open(tool:String) {
        ui.onNodeWithContentDescription("Attachments").performClick();ui.onNodeWithContentDescription("Create").performClick()
        if(tool!="Recipe")ui.onNodeWithContentDescription("Create page 2").performClick()
        ui.onNodeWithContentDescription(tool).assertIsDisplayed().performClick()
    }
    @Test fun structured_content_stages_above_a_separate_caption_and_sends_once() {
        val posts=mutableListOf<Map<String,Any?>>()
        setup {name,fields->if(name=="post")posts+=fields}
        open("Progress")
        ui.waitUntil(5000){ui.onAllNodes(hasContentDescription("Attach") and isEnabled()).fetchSemanticsNodes().isNotEmpty()}
        ui.onNodeWithContentDescription("Attach").assertIsEnabled().performClick()
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
    @Test fun latest_message_stays_above_the_floating_composer_when_keyboard_opens() {
        setup(messages=listOf(ChatMessage("latest","self","Latest synthetic message",true,"9:41","sent",false,emptyList(),emptyList(),null,true,peer="self")))
        ui.onNodeWithTag("composer").performClick()
        fun keyboardHeight()=androidx.core.view.WindowInsetsCompat.toWindowInsetsCompat(ui.activity.window.decorView.rootWindowInsets).getInsets(androidx.core.view.WindowInsetsCompat.Type.ime()).bottom
        ui.waitUntil(5000){keyboardHeight()>0}
        ui.waitForIdle()
        val bubble=ui.onNodeWithText("Latest synthetic message").assertIsDisplayed().fetchSemanticsNode().boundsInWindow
        val composer=ui.onNodeWithTag("composer").fetchSemanticsNode().boundsInWindow
        assertTrue("Latest message overlaps composer: $bubble $composer",bubble.bottom<=composer.top)
        assertTrue("Composer overlaps keyboard",composer.bottom<=ui.activity.window.decorView.height-keyboardHeight()+2)
    }
    @Test fun voice_draft_leaves_caption_field_available_until_explicit_send() {
        val commands=mutableListOf<Pair<String,Map<String,Any?>>>()
        setup(VoiceState(phase="Ready",peer="self",seconds=4,levels=List(20){.3f},duration=4000)){name,fields->commands+=name to fields}
        ui.onNodeWithTag("composer").performClick().performTextInput("Listen to this")
        assertFalse(commands.any {it.first=="record_send"})
        ui.onNodeWithContentDescription("Send voice message").performClick()
        assertEquals("Listen to this",commands.single {it.first=="record_send"}.second["caption"])
    }
    @Test fun typed_randomizers_preview_above_input_and_inline_motion_stays_an_indicator() {
        val posts=mutableListOf<Map<String,Any?>>()
        setup {name,fields->if(name=="post")posts+=fields}
        val input=ui.onNodeWithTag("composer")
        for ((kind,source) in listOf("dice" to "roll::2d6;","coin" to "pick::flip;","cards" to "pick::Museum, Park, Library;")) {
            input.performClick().performTextReplacement(source)
            val expected=nativeStructuredPreview(source)!!.text
            ui.waitUntil(5000){ui.onAllNodesWithText(expected).fetchSemanticsNodes().isNotEmpty()}
            fun textures(view:android.view.View):List<android.view.TextureView> = when(view) {
                is android.view.TextureView->listOf(view)
                is android.view.ViewGroup->(0 until view.childCount).flatMap {textures(view.getChildAt(it))}
                else->emptyList()
            }
            ui.waitUntil(10000){
                var drawn=false
                ui.runOnUiThread {
                    val views=textures(ui.activity.window.decorView)
                    drawn=views.size==(if(kind=="dice")2 else 1) && views.all {view->
                        val bitmap=view.bitmap
                        val visible=bitmap!=null && (1..7).any {x->(1..7).any {y->android.graphics.Color.alpha(bitmap.getPixel(bitmap.width*x/8,bitmap.height*y/8))>0}}
                        bitmap?.recycle();visible
                    }
                }
                drawn
            }
            ui.waitForIdle()
            val preview=ui.onNodeWithTag("typed-sigil-preview").assertIsDisplayed().fetchSemanticsNode().boundsInWindow
            assertTrue("Preview covers input",preview.bottom<=input.fetchSemanticsNode().boundsInWindow.top+1)
            assertTrue(posts.isEmpty())
            androidx.test.platform.app.InstrumentationRegistry.getInstrumentation().uiAutomation.takeScreenshot()?.let {bitmap->
                java.io.File(ui.activity.cacheDir,"typed-$kind.png").outputStream().use {bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)}
                bitmap.recycle()
            }
        }
        input.performTextReplacement("wave::Hello;")
        ui.waitUntil(5000){ui.onAllNodesWithContentDescription("Animated text: wave").fetchSemanticsNodes().isNotEmpty()}
        input.performTextReplacement("Plain text")
        ui.waitUntil(5000){ui.onAllNodesWithTag("typed-sigil-preview").fetchSemanticsNodes().isEmpty()}
        input.performTextReplacement("roll::2d6;")
        ui.onNodeWithContentDescription("Send message").performClick()
        assertEquals("roll::2d6;",posts.single()["text"])
        assertEquals(true,posts.single()["rich"])
    }
}
