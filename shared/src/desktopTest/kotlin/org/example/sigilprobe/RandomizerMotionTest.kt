package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class RandomizerMotionTest {
    @get:Rule val ui=createComposeRule()
    @Test fun stored_results_replay_deterministically_and_obey_motion_and_lifecycle_preferences() {
        val examples=listOf(
            RandomizerMotion("dice",listOf(DieFace(4,3),DieFace(6,5),DieFace(8,7),DieFace(10,9),DieFace(12,11),DieFace(20,17))),
            RandomizerMotion("dice",listOf(DieFace(100,87))),
            RandomizerMotion("coin",frames=listOf("Heads","Tails"),selected=0,result="Heads"),
            RandomizerMotion("coin",frames=listOf("Heads","Tails"),selected=1,result="Tails"),
            RandomizerMotion("choice",frames=listOf("Pizza","Tacos","Pasta"),selected=1,result="Tacos"),
            RandomizerMotion("number",frames=listOf("8","12","1","9"),result="7"))
        var value by mutableStateOf(examples.first())
        var reduced by mutableStateOf(false)
        var full by mutableStateOf(false)
        var visible by mutableStateOf(true)
        val clock=TextPlayback()
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalMotion provides MotionPolicy(reduced),LocalMotionVisible provides visible) {
            MessageMotion("random",clock,false) {Box(Modifier.width(280.dp).testTag("stage")) {RandomizerStage(value,full)}}
        }}}
        fun pixels():List<androidx.compose.ui.graphics.Color> {
            val pixels=ui.onNodeWithTag("stage").captureToImage().toPixelMap()
            return buildList {repeat(pixels.height) {y->repeat(pixels.width) {x->add(pixels[x,y])}}}
        }
        for(example in examples) {
            ui.runOnIdle {value=example;clock.elapsed=350f}
            val moving=pixels();val bounds=ui.onNodeWithTag("stage").fetchSemanticsNode().boundsInRoot.size
            ui.runOnIdle {clock.elapsed=2000f}
            val settled=pixels()
            assertNotEquals(moving,settled,example.kind)
            assertEquals(bounds,ui.onNodeWithTag("stage").fetchSemanticsNode().boundsInRoot.size)
            ui.runOnIdle {clock.replay();clock.elapsed=350f}
            assertEquals(moving,pixels(),"Replay ${example.kind}")
            ui.runOnIdle {reduced=true}
            assertEquals(settled,pixels(),"Reduced motion ${example.kind}")
            ui.runOnIdle {reduced=false;full=true;clock.elapsed=350f}
            assertEquals(settled,pixels(),"Expanded ${example.kind}")
            ui.runOnIdle {full=false;clock.elapsed=350f;visible=false}
            ui.runOnIdle {visible=true}
            assertEquals(settled,pixels(),"Resume must show the stored result")
            assertEquals(example,value)
        }
    }
}
