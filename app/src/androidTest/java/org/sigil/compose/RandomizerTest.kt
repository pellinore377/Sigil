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

class RandomizerTest {
    @get:Rule val ui=createAndroidComposeRule<ComponentActivity>()
    @Test fun volumetric_dice_and_coins_replay_the_same_stored_results_in_the_timeline() {
        val chat=ChatSummary("self","@sam:example.test","","",true,emptyList())
        var state by mutableStateOf(MessengerState(phase="connected",chats=listOf(chat),selected="self",timelineLoaded=true))
        ui.mainClock.autoAdvance=false
        ui.runOnUiThread {ui.activity.setSigilContent {SigilApp(NativeCore::palette,NativeCore::analyze,state,{_,_->})}}
        ui.mainClock.advanceTimeBy(600)
        val samples=listOf(
            UtilityContent("dice",display="2d6 · 8",motion=RandomizerMotion("dice",listOf(DieFace(6,3),DieFace(6,5)))),
            UtilityContent("dice",display="Mixed dice · 52",motion=RandomizerMotion("dice",listOf(DieFace(4,3),DieFace(6,5),DieFace(8,7),DieFace(10,9),DieFace(12,11),DieFace(20,17)))),
            UtilityContent("pick",display="flip",rich=RichText("Heads"),motion=RandomizerMotion("coin",frames=listOf("Heads","Tails"),selected=0,result="Heads")),
            UtilityContent("pick",display="flip",rich=RichText("Tails"),motion=RandomizerMotion("coin",frames=listOf("Heads","Tails"),selected=1,result="Tails")))
        samples.forEachIndexed {index,value->
            val message=ChatMessage("random-$index","sam","Stored result",true,"9:33","sent",false,emptyList(),emptyList(),null,true,
                timestamp=1000,parts=listOf(MessagePart("card","utility","Stored result",utility=value)))
            ui.runOnUiThread {state=state.copy(messages=listOf(message)+state.messages.take(1))}
            ui.mainClock.advanceTimeBy(350)
            val description=value.motion!!.let {m->if(m.kind=="coin")"Coin: ${m.result}" else "Dice: "+m.dice.joinToString {"d${it.sides} · ${it.face}"}}
            val stage=ui.onNodeWithContentDescription(description)
            val moving=stage.captureToImage().asAndroidBitmap()
            ui.mainClock.advanceTimeBy(1500)
            val settled=stage.captureToImage().asAndroidBitmap()
            assertFalse(moving.sameAs(settled))
            if(androidx.test.platform.app.InstrumentationRegistry.getArguments().getString("capture_motion")=="true") {
                java.io.File(ui.activity.cacheDir,"randomizer-$index-moving.png").outputStream().use {moving.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)}
                java.io.File(ui.activity.cacheDir,"randomizer-$index-settled.png").outputStream().use {settled.compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)}
                if(index==0)java.io.File(ui.activity.cacheDir,"randomizer-screen.png").outputStream().use {ui.onRoot(useUnmergedTree=true).captureToImage().asAndroidBitmap().compress(android.graphics.Bitmap.CompressFormat.PNG,100,it)}
            }
            stage.performTouchInput {longClick()}
            ui.mainClock.advanceTimeBy(500)
            ui.mainClock.autoAdvance=true
            ui.onNodeWithText("Replay animation").performScrollTo()
            ui.mainClock.autoAdvance=false
            ui.onNodeWithText("Replay animation").performClick()
            ui.mainClock.advanceTimeBy(500)
            assertFalse(stage.captureToImage().asAndroidBitmap().sameAs(settled))
            ui.mainClock.advanceTimeBy(1500)
            assertTrue(stage.captureToImage().asAndroidBitmap().sameAs(settled))
            assertEquals(value,state.messages.first().parts.single().utility)
        }
    }
}
