package org.sigil

import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class TextMotionTest {
    @get:Rule val ui=createComposeRule()
    @Test fun history_paging_and_recomposition_do_not_restart_new_message_playback() {
        val ledger=MotionLedger()
        ledger.update(emptyList(),false,true)
        ledger.update(listOf("old"),true,true)
        assertEquals(2000f,ledger.state("old").elapsed)
        ledger.update(listOf("new","old","older"),true,true)
        val clock=ledger.state("new")
        assertEquals(0f,clock.elapsed)
        assertEquals(2000f,ledger.state("older").elapsed)
        clock.elapsed=800f
        ledger.update(listOf("new","old","older","oldest"),true,true)
        assertSame(clock,ledger.state("new"));assertEquals(800f,clock.elapsed)
        assertEquals(2000f,ledger.state("oldest").elapsed)
        ledger.state("old").replay()
        assertEquals(0f,ledger.state("old").elapsed);assertEquals(800f,clock.elapsed)
        val empty=MotionLedger()
        empty.update(emptyList(),true,true)
        empty.update(listOf("first"),true,true,emptySet())
        empty.update(listOf("first"),true,true,setOf("first"))
        assertEquals(2000f,empty.state("first").elapsed)
        val history=(0..1000).map {"history-$it"}
        val archive=MotionLedger()
        archive.update(history,true,false)
        history.forEach {archive.state(it)}
        val replay=archive.state(history.last())
        replay.replay()
        archive.update(history,true,false)
        assertSame(replay,archive.state(history.last()))
        assertEquals(0f,replay.elapsed)
    }
    @Test fun playback_pauses_offscreen_and_reduced_motion_settles_without_restarting() {
        val clock=TextPlayback(true)
        var visible by mutableStateOf(false)
        var reduced by mutableStateOf(false)
        val rich=RichText("office العربية 👩🏽‍💻",motion=listOf(TextMotion("wave",1200,1,140,0,1000,45,0,listOf(0 to 6,7 to 14,15 to 22))))
        ui.mainClock.autoAdvance=false
        ui.setContent { MaterialTheme {
            CompositionLocalProvider(LocalMotion provides MotionPolicy(reduced),LocalTextMotionSeeds provides {List(192) {"12345"}.joinToString(",")}) {
                MessageMotion("synthetic",clock,visible) {RichMessageText(rich,Modifier.testTag("moving-text"))}
            }
        } }
        ui.mainClock.advanceTimeBy(2500)
        assertEquals(0f,clock.elapsed)
        visible=true
        ui.mainClock.advanceTimeBy(300)
        assertTrue(clock.elapsed in 200f..350f)
        val bounds=ui.onNodeWithTag("moving-text").fetchSemanticsNode().boundsInRoot
        ui.onNodeWithText(rich.text).assertExists()
        visible=false
        ui.mainClock.advanceTimeByFrame()
        val paused=clock.elapsed
        ui.mainClock.advanceTimeBy(2500)
        assertEquals(paused,clock.elapsed)
        assertEquals(bounds,ui.onNodeWithTag("moving-text").fetchSemanticsNode().boundsInRoot)
        reduced=true
        ui.mainClock.advanceTimeByFrame()
        assertEquals(2000f,clock.elapsed)
        reduced=false;visible=true
        ui.mainClock.advanceTimeBy(500)
        assertEquals(2000f,clock.elapsed)
    }
    @Test fun concealed_content_offsets_and_slices_keep_only_visible_motion_ranges() {
        val rich=RichText("secret wave",listOf(RichSpan(0,6,reveal="spoiler")),motion=listOf(TextMotion("shake",480,4,80,0,1000,0,0,listOf(7 to 11))))
        assertEquals(12,motionOffsets(rich,emptySet(),7))
        assertEquals(7,motionOffsets(rich,setOf(0),7))
        val slice=richSlice(rich,7,11)
        assertEquals(listOf(0 to 4),slice.motion.single().units)
        assertTrue(richSlice(rich,0,6).motion.isEmpty())
    }
    @Test fun disabling_message_effects_prevents_autoplay() {
        val clock=TextPlayback(true)
        ui.setContent {CompositionLocalProvider(LocalAppearance provides Appearance(messageEffects=false)) {
            MessageMotion("synthetic",clock,true) {}
        }}
        ui.waitForIdle()
        assertEquals(2000f,clock.elapsed)
    }
    private fun pixels():List<androidx.compose.ui.graphics.Color> {
        val pixels=ui.onNodeWithTag("effect").captureToImage().toPixelMap()
        return buildList {repeat(pixels.height) {y->repeat(pixels.width) {x->add(pixels[x,y])}}}
    }
    @Test fun glyph_blur_and_the_portable_fallback_both_leave_the_final_text_unchanged() {
        val clock=TextPlayback(true)
        var native by mutableStateOf(true)
        val rich=RichText("Light",motion=listOf(TextMotion("glow",1200,1,180,0,1000,0,0,listOf(0 to 5))))
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalMotionBlur provides native) {
            MessageMotion("synthetic",clock,false) {RichMessageText(rich,Modifier.testTag("effect"))}
        }}}
        val initial=pixels()
        ui.runOnIdle {clock.elapsed=400f}
        assertNotEquals(initial,pixels())
        ui.runOnIdle {native=false}
        assertNotEquals(initial,pixels())
        ui.runOnIdle {clock.elapsed=2000f}
        assertEquals(initial,pixels())
    }
    @Test fun upside_down_glyphs_are_settled_in_history_and_reduced_motion() {
        var flipped by mutableStateOf(true)
        var reduced by mutableStateOf(false)
        val motion=TextMotion("flip",700,1,0,180,1000,40,0,listOf(0 to 1,1 to 2,2 to 3))
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalMotion provides MotionPolicy(reduced)) {
            RichMessageText(RichText("abc",motion=if(flipped)listOf(motion) else emptyList()),Modifier.testTag("effect"))
        }}}
        val upsideDown=pixels()
        ui.runOnIdle {reduced=true}
        assertEquals(upsideDown,pixels())
        ui.runOnIdle {flipped=false}
        assertNotEquals(upsideDown,pixels())
        ui.onNodeWithText("abc").assertExists()
    }
}
