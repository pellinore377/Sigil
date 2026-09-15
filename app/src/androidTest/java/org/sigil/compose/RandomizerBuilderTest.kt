package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*

class RandomizerBuilderTest {
    @get:Rule val ui=createAndroidComposeRule<ComponentActivity>()
    @Test fun create_keeps_randomizers_in_the_composer_and_sends_canonical_content_explicitly() {
        val commands=mutableListOf<Pair<String,Map<String,Any?>>>()
        val chat=ChatSummary("self","@sam:example.test","","",true,emptyList())
        val state=mutableStateOf(MessengerState(phase="connected",chats=listOf(chat),selected="self",timelineLoaded=true))
        ui.runOnUiThread {ui.activity.setSigilContent {
            SigilApp(NativeCore::palette,NativeCore::analyze,state.value,{name,fields->commands+=name to fields})
        }}
        fun open(name:String) {
            ui.onNodeWithContentDescription("Attachments").performClick()
            ui.onNodeWithContentDescription("Create").performClick()
            ui.onNodeWithContentDescription(name).assertIsDisplayed().performClick()
        }
        fun stageAndAcknowledge(name:String,source:String,motion:RandomizerMotion) {
            ui.onNodeWithContentDescription("Attach").performClick()
            ui.waitUntil(5000) {ui.onAllNodesWithTag("randomizer-preview-objects").fetchSemanticsNodes().isNotEmpty()}
            val previewCenter=ui.onNodeWithTag("randomizer-preview-objects").getUnclippedBoundsInRoot().let {(it.top.value+it.bottom.value)/2}
            ui.mainClock.autoAdvance=false
            ui.onNodeWithContentDescription("Send message").performClick()
            assertEquals(source,commands.last {it.first=="post"}.second["text"])
            assertEquals(true,commands.last {it.first=="post"}.second["rich"])
            val id="synthetic-${commands.count {it.first=="post"}}"
            ui.runOnIdle {
                val part=MessagePart(id,"Utility",motion.result,utility=UtilityContent(if(motion.kind=="dice")"dice" else "pick",display=motion.result,motion=motion))
                val message=ChatMessage(id,"sam",source,true,"9:30 AM","sent",false,emptyList(),emptyList(),null,true,peer="self",parts=listOf(part))
                state.value=state.value.copy(sent=state.value.sent+1,sentText=source,sentMessage=id,messages=listOf(message)+state.value.messages)
            }
            ui.mainClock.advanceTimeBy(32)
            if(motion.kind!="choice") {
                ui.onNodeWithContentDescription("Edit $name").assertExists()
                ui.onNodeWithContentDescription("Send message").assertIsNotEnabled()
            }
            val portalTag=if(motion.kind=="choice")"picker-card-3" else "material-flight-recorded"
            var sawPortal=false
            var sawRetainedPortal=false
            ui.waitUntil(20000) {
                ui.mainClock.advanceTimeBy(16)
                val portal=ui.onAllNodesWithTag(portalTag,useUnmergedTree=true).fetchSemanticsNodes().isNotEmpty()
                val retained=ui.onAllNodesWithContentDescription("Edit $name").fetchSemanticsNodes().isNotEmpty()
                if(portal && !sawPortal && motion.kind=="choice") {
                    val cardCenter=ui.onNodeWithTag(portalTag,useUnmergedTree=true).getUnclippedBoundsInRoot().let {(it.top.value+it.bottom.value)/2}
                    assertEquals("Card stays where its preview was",previewCenter,cardCenter,1f)
                }
                sawPortal=sawPortal||portal
                sawRetainedPortal=sawRetainedPortal||(portal&&retained)
                sawPortal && !retained
            }
            assertTrue("Native $name portal rendered",sawPortal)
            if(motion.kind!="choice")assertTrue("Preview retained while $name departs",sawRetainedPortal)
            ui.mainClock.advanceTimeBy(13000)
            ui.mainClock.autoAdvance=true
            ui.onAllNodesWithTag("material-flight-recorded",useUnmergedTree=true).assertCountEquals(0)
            ui.onNodeWithContentDescription("Edit $name").assertDoesNotExist()
            ui.onNodeWithContentDescription(when(motion.kind){"coin"->"Coin: ${motion.result}";"choice"->"Chosen: ${motion.result}";else->"Dice: "+motion.dice.joinToString {"d${it.sides} · ${it.face}"}}).assertExists()
            assertEquals(id,state.value.messages.first().id)
            assertEquals(motion.result,state.value.messages.first().parts.first().utility?.motion?.result)
        }
        open("Dice")
        ui.onNodeWithText("Count 1").performTextReplacement("3")
        ui.onNodeWithText("Sides 1").performTextReplacement("8")
        assertTrue(commands.none {it.first=="post"})
        stageAndAcknowledge("Dice","roll::3d8;",RandomizerMotion("dice",listOf(DieFace(8,2),DieFace(8,4),DieFace(8,7)),result="13"))
        open("Cards")
        ui.onNodeWithText("Choice 1").performTextInput("Fish,\nchips")
        ui.onNodeWithText("Choice 2").performScrollTo().performTextInput("redact::literal;")
        stageAndAcknowledge("Cards",NativeCore.builderSource("Choice\nFish, chips\nredact::literal;"),RandomizerMotion("choice",frames=listOf("Fish, chips","redact::literal;"),selected=0,result="Fish, chips"))
        open("Coin")
        stageAndAcknowledge("Coin","pick::flip;",RandomizerMotion("coin",frames=listOf("Heads","Tails"),selected=0,result="Heads"))
        assertEquals(3,commands.count {it.first=="post"})
    }
}
