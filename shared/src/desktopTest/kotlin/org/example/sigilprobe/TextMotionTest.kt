package org.sigil

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.layout.width
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
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

    @Test fun size_steps_follow_the_reference_and_only_lines_holding_enlarged_text_grow() {
        assertEquals(listOf(.7f,.8f,.9f,1f,1.15f,1.35f,1.6f),(-3..3).map(::sizeStep))
        val style=androidx.compose.ui.text.TextStyle(fontSize=16.sp,lineHeight=23.sp)
        val text=androidx.compose.ui.text.buildAnnotatedString {append("quiet\nHello");addStyle(androidx.compose.ui.text.SpanStyle(fontSize=1.6f.em),6,11)}
        val (sized,sizedStyle)=sizedParagraphs(text,style)
        val plain=sized.paragraphStyles.single()
        assertEquals(0 to 5,plain.start to plain.end)
        assertEquals(23f,plain.item.lineHeight.value,.01f)
        assertTrue(sizedStyle.lineHeight==androidx.compose.ui.unit.TextUnit.Unspecified)
        assertEquals(style,sizedParagraphs(androidx.compose.ui.text.AnnotatedString("plain"),style).second)
        // A wrapped big3 run must not stretch the body-only line that follows it.
        val long=androidx.compose.ui.text.buildAnnotatedString {
            append("Before Hello there, this is a long run that wraps; and after it, quietly, all the way to the very end of the paragraph.")
            addStyle(androidx.compose.ui.text.SpanStyle(fontSize=1.6f.em),7,50)
        }
        var layout:androidx.compose.ui.text.TextLayoutResult?=null
        ui.setContent {val (t,s2)=sizedParagraphs(long,style);androidx.compose.material3.Text(t,Modifier.width(220.dp),style=s2,onTextLayout={layout=it})}
        ui.runOnIdle {
            val l=checkNotNull(layout);val last=l.lineCount-1
            assertTrue(l.getLineStart(last)>50 && l.lineCount>=3)
            val body=with(ui.density) {23.sp.toPx()}
            assertTrue(l.getLineBottom(last)-l.getLineTop(last)<=body*1.15f,"body line ${l.getLineBottom(last)-l.getLineTop(last)} vs $body")
        }
    }
    @OptIn(ExperimentalTestApi::class) @Test fun a_hidden_spoiler_keeps_its_size_and_weight_so_revealing_never_reflows() {
        val rich=RichText("A secret that is big and bold here",listOf(RichSpan(2,26,flags=setOf("bold"),size=3,reveal="spoiler")))
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalMotion provides MotionPolicy(true)) {
            Box(Modifier.width(240.dp).testTag("effect")) {RichMessageText(rich)}
        }}}
        val before=ui.onNodeWithTag("effect").fetchSemanticsNode().size
        ui.onNode(hasText("[Spoiler, hidden]",substring=true)).performCustomAccessibilityActionWithLabel("Reveal")
        ui.onNodeWithText("A secret that is big and bold here").assertExists()
        assertEquals(before,ui.onNodeWithTag("effect").fetchSemanticsNode().size)
    }
    @OptIn(ExperimentalTestApi::class) @Test fun screen_readers_hear_neither_redactions_nor_hidden_text_and_can_reveal_it() {
        val rich=RichText("Pin [REDACTED] then spoil and scratch",listOf(RichSpan(4,14,redaction=8),RichSpan(20,25,reveal="spoiler"),RichSpan(30,37,reveal="scratch")))
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalMotion provides MotionPolicy(true)) {RichMessageText(rich)}}}
        ui.onNodeWithText("Pin [REDACTED] then [Spoiler, hidden] and [Scratch to reveal, hidden]").assertExists()
        ui.onAllNodes(hasText("xxxx",substring=true)).assertCountEquals(0)
        ui.onAllNodes(hasText("spoil",substring=true,ignoreCase=false).and(hasText("Spoiler",substring=true).not())).assertCountEquals(0)
        ui.onNode(hasText("[Scratch to reveal, hidden]",substring=true)).performCustomAccessibilityActionWithLabel("Reveal 2")
        ui.onNodeWithText("Pin [REDACTED] then [Spoiler, hidden] and scratch").assertExists()
        ui.onNode(hasText("[Spoiler, hidden]",substring=true)).performCustomAccessibilityActionWithLabel("Reveal")
        ui.onNodeWithText("Pin [REDACTED] then spoil and scratch").assertExists()
    }
    @Test fun a_redaction_bar_parts_from_every_ground_at_three_to_one() {
        val white=androidx.compose.ui.graphics.Color.White
        listOf(androidx.compose.ui.graphics.Color(0xff202020),androidx.compose.ui.graphics.Color(0xff4f4f4f),androidx.compose.ui.graphics.Color(0xff2b2b2b)).forEach {ground->
            assertTrue(contrastRatio(redactionRing(ground,white),ground)>=3f)
        }
        assertTrue(contrastRatio(androidx.compose.ui.graphics.Color(0xff1b1b1b),androidx.compose.ui.graphics.Color(0xffececec))>=3f)
    }
    @Test fun a_spoiler_veil_reads_on_dark_and_light_bubbles() {
        var dark by mutableStateOf(true)
        ui.setContent {MaterialTheme {
            val ground=if(dark)androidx.compose.ui.graphics.Color(0xff2b2b2b) else androidx.compose.ui.graphics.Color(0xffececec)
            val ink=if(dark)androidx.compose.ui.graphics.Color.White else androidx.compose.ui.graphics.Color(0xff1b1b1b)
            CompositionLocalProvider(LocalMotion provides MotionPolicy(true),LocalMessageSurface provides ground,androidx.compose.material3.LocalContentColor provides ink) {
                Box(Modifier.testTag("effect").background(ground)) {RichMessageText(RichText("hidden words",listOf(RichSpan(0,12,reveal="spoiler"))))}
            }
        }}
        repeat(2) {
            val ground=if(dark)androidx.compose.ui.graphics.Color(0xff2b2b2b) else androidx.compose.ui.graphics.Color(0xffececec)
            val marked=pixels().count {kotlin.math.abs(it.luminance()-ground.luminance())>.04f}
            assertTrue(marked>pixels().size/20,"veil marks only $marked pixels")
            ui.runOnIdle {dark=false}
        }
    }
}
