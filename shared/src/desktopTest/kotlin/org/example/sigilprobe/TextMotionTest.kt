package org.sigil

import androidx.compose.foundation.layout.Box
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
    @Test fun preparation_holds_playback_and_optional_replay_obeys_visibility_and_reduced_motion() {
        val clock=TextPlayback(true)
        val token=Any();clock.preparing[token]=Unit
        var visible by mutableStateOf(true)
        var reduced by mutableStateOf(false)
        ui.mainClock.autoAdvance=false
        ui.setContent {CompositionLocalProvider(LocalAppearance provides Appearance(replaySeconds=10),LocalMotion provides MotionPolicy(reduced)) {
            MessageMotion("sample",clock,visible,500) {}
        }}
        ui.mainClock.advanceTimeBy(1000);assertEquals(0f,clock.elapsed)
        clock.materialDuration=3000;clock.preparing.remove(token)
        ui.mainClock.advanceTimeBy(1000);assertTrue(clock.elapsed in 900f..1100f)
        ui.mainClock.advanceTimeBy(2200);assertEquals(12000f,clock.elapsed)
        ui.mainClock.advanceTimeBy(9000);assertEquals(0,clock.generation)
        ui.mainClock.advanceTimeBy(1200);assertEquals(1,clock.generation)
        visible=false;ui.mainClock.advanceTimeBy(30000);assertEquals(1,clock.generation)
        visible=true;reduced=true;ui.mainClock.advanceTimeBy(30000)
        assertEquals(1,clock.generation);assertEquals(12000f,clock.elapsed)
    }
    @Test fun history_paging_and_recomposition_do_not_restart_new_message_playback() {
        val ledger=MotionLedger()
        ledger.update(emptyList(),false,true)
        ledger.update(listOf("old"),true,true)
        assertEquals(12000f,ledger.state("old").elapsed)
        ledger.update(listOf("new","old","older"),true,true)
        val clock=ledger.state("new")
        assertEquals(0f,clock.elapsed)
        assertEquals(12000f,ledger.state("older").elapsed)
        clock.elapsed=800f
        ledger.update(listOf("new","old","older","oldest"),true,true)
        assertSame(clock,ledger.state("new"));assertEquals(800f,clock.elapsed)
        assertEquals(12000f,ledger.state("oldest").elapsed)
        ledger.state("old").replay()
        assertEquals(0f,ledger.state("old").elapsed);assertEquals(800f,clock.elapsed)
        val empty=MotionLedger()
        empty.update(emptyList(),true,true)
        empty.update(listOf("first"),true,true,emptySet())
        empty.update(listOf("first"),true,true,setOf("first"))
        assertEquals(12000f,empty.state("first").elapsed)
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
        val rich=RichText("office العربية 👩🏽‍💻",motion=listOf(TextMotion("wave",1300,"ribbon",15f,28,0,false,"","",listOf(0 to 6,7 to 14,15 to 22))))
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
        assertEquals(12000f,clock.elapsed)
        reduced=false;visible=true
        ui.mainClock.advanceTimeBy(500)
        assertEquals(12000f,clock.elapsed)
    }
    @Test fun concealed_content_offsets_and_slices_keep_only_visible_motion_ranges() {
        val rich=RichText("secret wave",listOf(RichSpan(0,6,reveal="spoiler")),motion=listOf(TextMotion("shake",1550,"echo",5f,0,0,true,"","",listOf(7 to 11))))
        assertEquals(7,motionOffsets(rich,emptySet(),7))
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
        assertEquals(12000f,clock.elapsed)
    }
    private fun pixels():List<androidx.compose.ui.graphics.Color> {
        val pixels=ui.onNodeWithTag("effect").captureToImage().toPixelMap()
        return buildList {repeat(pixels.height) {y->repeat(pixels.width) {x->add(pixels[x,y])}}}
    }
    @Test fun glyph_blur_and_the_portable_fallback_both_leave_the_final_text_unchanged() {
        val clock=TextPlayback(true)
        var native by mutableStateOf(true)
        val rich=RichText("Light",motion=listOf(TextMotion("glow",1300,"travel",24f,48,0,false,"","",listOf(0 to 5))))
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalMotionBlur provides native) {
            MessageMotion("synthetic",clock,false) {RichMessageText(rich,Modifier.testTag("effect"))}
        }}}
        val initial=pixels()
        ui.runOnIdle {clock.elapsed=400f}
        assertNotEquals(initial,pixels())
        ui.runOnIdle {native=false}
        assertNotEquals(initial,pixels())
        ui.runOnIdle {clock.elapsed=12000f}
        assertEquals(initial,pixels())
    }
    @Test fun brushing_only_clears_grid_cells_inside_the_concealed_rectangles() {
        val cells=inkCells(listOf(androidx.compose.ui.geometry.Rect(0f,0f,32f,16f)),8f)
        assertEquals(8,cells.size)
        assertTrue(brushedCells(androidx.compose.ui.geometry.Offset(4f,4f),6f,8f,cells).isNotEmpty())
        assertTrue(brushedCells(androidx.compose.ui.geometry.Offset(200f,200f),6f,8f,cells).isEmpty())
        assertTrue(brushedCells(androidx.compose.ui.geometry.Offset(4f,4f),400f,8f,cells).size<=cells.size)
    }
    @Test fun invisible_ink_and_frosted_glass_do_not_conceal_alike() {
        var kind by mutableStateOf("scratch")
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalMotion provides MotionPolicy(true)) {
            Box(Modifier.testTag("effect")) {RichMessageText(RichText("A secret end",listOf(RichSpan(2,8,reveal=kind))))}
        }}}
        val grain=pixels().toSet()
        ui.runOnIdle {kind="spoiler"}
        val frost=pixels().toSet()
        assertTrue(grain.size>24 && frost.size>24 && grain!=frost,"grain ${grain.size} tones, frost ${frost.size} tones")
    }
    @Test fun concealment_survives_reduced_motion_and_disabled_effects() {
        var concealed by mutableStateOf(true)
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalMotion provides MotionPolicy(true),LocalAppearance provides Appearance(messageEffects=false)) {
            Box(Modifier.testTag("effect")) {RichMessageText(if(concealed)RichText("A secret end",listOf(RichSpan(2,8,reveal="spoiler"))) else RichText("A Hidden text end"))}
        }}}
        val veiled=pixels()
        ui.runOnIdle {concealed=false}
        assertNotEquals(veiled,pixels())
    }
    @Test fun assemble_begins_dispersed_and_settles_into_the_original_text() {
        val clock=TextPlayback()
        val rich=RichText("abc",motion=listOf(TextMotion("assemble",1670,"sort",74f,35,0,false,"","",listOf(0 to 1,1 to 2,2 to 3))))
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalTextMotionSeeds provides {List(192) {"458752"}.joinToString(",")}) {
            MessageMotion("synthetic",clock,false) {RichMessageText(rich,Modifier.testTag("effect"))}
        }}}
        val settled=pixels()
        ui.runOnIdle {clock.elapsed=0f}
        assertNotEquals(settled,pixels())
        ui.runOnIdle {clock.elapsed=12000f}
        assertEquals(settled,pixels())
        ui.onNodeWithText("abc").assertExists()
    }
    @Test fun sparkle_takes_its_colour_from_the_span_it_decorates() {
        val clock=TextPlayback()
        var painted by mutableStateOf(true)
        val motion=TextMotion("sparkle",1900,"constellation",11f,0,9,false,"","",listOf(0 to 1,1 to 2,2 to 3))
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalTextMotionSeeds provides {List(192) {"458752"}.joinToString(",")}) {
            MessageMotion("synthetic",clock,false) {
                RichMessageText(RichText("abc",if(painted)listOf(RichSpan(0,3,colors=listOf("red1","blue3"))) else emptyList(),motion=listOf(motion)),Modifier.testTag("effect"))
            }
        }}}
        ui.runOnIdle {clock.elapsed=400f}
        val coloured=pixels().toSet()
        ui.runOnIdle {painted=false}
        assertNotEquals(coloured,pixels().toSet())
    }
    @Test fun upside_down_text_is_static_reversed_and_keeps_the_original_readable() {
        var flipped by mutableStateOf(true)
        var reduced by mutableStateOf(false)
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalMotion provides MotionPolicy(reduced)) {
            RichMessageText(RichText("abc",if(flipped)listOf(RichSpan(0,3,flags=setOf("flip"))) else emptyList(),
                motion=if(flipped)listOf(TextMotion("flip",0,"plain",0f,0,0,false,"","",listOf(0 to 1,1 to 2,2 to 3))) else emptyList()),Modifier.testTag("effect"))
        }}}
        val upsideDown=pixels()
        ui.runOnIdle {reduced=true}
        assertEquals(upsideDown,pixels())
        ui.onNodeWithText("abc").assertExists()
        ui.runOnIdle {flipped=false}
        assertNotEquals(upsideDown,pixels())
        ui.onNodeWithText("abc").assertExists()
    }
}
