package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.runtime.*
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*

class TextMotionTest {
    @get:Rule val ui=createAndroidComposeRule<ComponentActivity>()
    @Test fun all_text_effects_draw_without_changing_layout_or_accessibility_text() {
        val chat=ChatSummary("self","@sam:example.test","","",true,emptyList(),displayName="Sam")
        var state by mutableStateOf(MessengerState(phase="connected",chats=listOf(chat),selected="self",timelineLoaded=true))
        ui.mainClock.autoAdvance=false
        ui.runOnUiThread {ui.activity.setSigilContent {SigilApp(NativeCore::palette,NativeCore::analyze,state,{_,_->})}}
        ui.mainClock.advanceTimeBy(600)
        val kinds=listOf("shake","wave","pulse","glow","typewriter","sparkle","glitch","scatter","flip","barrel")
        for((index,kind) in kinds.withIndex()) {
            val text="$kind letter office العربية 👩🏽‍💻"
            val prefix=text.indexOf('ا')
            val units=if(kind=="barrel")listOf(0 to text.length) else (0 until prefix).map {it to it+1}+listOf(prefix to text.length)
            val rich=RichText(text,motion=listOf(TextMotion(kind,1200,1,if(kind=="barrel")600 else 140,if(kind=="flip")180 else 360,1080,45,12,units)))
            val message=ChatMessage((index+1).toString(16).padStart(64,'0'),"sam",text,true,"9:33","Sent",false,emptyList(),emptyList(),null,true,
                timestamp=1800000000L+index,parts=listOf(MessagePart("","text",text,rich=rich)))
            ui.runOnUiThread {state=state.copy(messages=listOf(message)+state.messages.take(1))}
            ui.mainClock.advanceTimeBy(420)
            val node=ui.onNodeWithText(text,useUnmergedTree=true)
            node.assertIsDisplayed()
            val bounds=node.fetchSemanticsNode().boundsInRoot.size
            val moving=node.captureToImage().asAndroidBitmap()
            if(androidx.test.platform.app.InstrumentationRegistry.getArguments().getString("capture_motion")=="true") {
                java.io.File(ui.activity.cacheDir,"motion-$kind.png").outputStream().use {moving.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)}
                if(kind in listOf("glow","barrel")) {
                    val full=ui.onRoot().captureToImage().asAndroidBitmap()
                    java.io.File(ui.activity.cacheDir,"motion-$kind-full.png").outputStream().use {full.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)}
                }
            }
            ui.mainClock.advanceTimeBy(1800)
            assertEquals(bounds,node.fetchSemanticsNode().boundsInRoot.size)
            node.assertTextEquals(text)
            val settled=node.captureToImage().asAndroidBitmap()
            if(kind=="flip" && androidx.test.platform.app.InstrumentationRegistry.getArguments().getString("capture_motion")=="true") {
                java.io.File(ui.activity.cacheDir,"motion-flip-settled.png").outputStream().use {settled.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)}
            }
            assertFalse("$kind did not change its pixels",moving.sameAs(settled))
        }
        val text=state.messages.first().text
        ui.onNodeWithText(text).performTouchInput {longClick()}
        ui.mainClock.advanceTimeBy(500)
        ui.onNodeWithText("Replay animation").assertIsDisplayed().performClick()
        ui.mainClock.advanceTimeBy(500)
        ui.onNodeWithText("Replay animation").assertDoesNotExist()
        ui.onNodeWithText(text,useUnmergedTree=true).assertIsDisplayed()
    }
}
