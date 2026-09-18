@file:OptIn(kotlin.time.ExperimentalTime::class)
package org.sigil

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class TemporalCardTest {
    @get:Rule val ui=createComposeRule()

    private fun message(vararg parts:MessagePart)=ChatMessage("card","author","",false,"9:41","read",false,emptyList(),emptyList(),null,true,peer="conversation",parts=parts.toList())
    private fun render(message:ChatMessage) {ui.setContent {MaterialTheme {Box(Modifier.width(400.dp)) {MessageCards(message,{""},null)}}}}

    @Test fun elapsed_spans_are_broken_down_against_the_calendar_not_average_months() {
        // 2020-01-31T00:00:00Z .. 2020-03-01T00:00:00Z: one calendar month plus a leap day.
        assertEquals("1 month · 1 day",temporalPhrase(temporalComponents(1580428800L,1583020800L)))
        // 2019-01-31 .. 2019-03-01 borrows a 28-day February.
        assertEquals("1 month · 1 day",temporalPhrase(temporalComponents(1548892800L,1551398400L)))
        // 2019-03-01T06:30 .. 2025-08-13T14:00 reads out largest first, down to the minute.
        assertEquals("6 years · 5 months · 12 days · 7 hours · 30 minutes",temporalPhrase(temporalComponents(1551421800L,1755093600L)))
        assertEquals("2 hours · 5 minutes",temporalPhrase(temporalComponents(0L,7500L)))
        // Under a minute, and a zero span, carry no components at all.
        assertTrue(temporalComponents(0L,59L).isEmpty())
        assertTrue(temporalComponents(100L,100L).isEmpty())
        // The elapsed card leads with the first component and reads out the rest.
        assertEquals("5 months · 12 days · 7 hours · 30 minutes",temporalPhrase(temporalComponents(1551421800L,1755093600L).drop(1)))
    }

    @Test fun a_countdown_counts_whole_days_before_it_counts_years() {
        assertEquals(290L to "days",temporalCalendarScale(290L*86400L))
        assertEquals(1L to "day",temporalCalendarScale(86400L))
        assertEquals(0L to "days",temporalCalendarScale(3600L))
        assertEquals(1L to "year",temporalCalendarScale(366L*86400L))
        assertEquals(7L to "years",temporalCalendarScale(2740L*86400L))
    }

    @Test fun the_flap_keeps_a_fixed_cell_count_inside_each_regime() {
        assertEquals("00:00",temporalDigits(0L))
        assertEquals("09:05",temporalDigits(545L))
        assertEquals("59:59",temporalDigits(3599L))
        assertEquals("01:00:00",temporalDigits(3600L))
        assertEquals("02:03:04",temporalDigits(7384L))
    }

    @Test fun a_task_list_is_numbered_so_it_never_reads_as_a_checklist() {
        val items=listOf(CardItem("a","Book the room",true,false),CardItem("b","Send the agenda",false,true))
        render(message(MessagePart("t","task","Launch",items=items)))
        ui.onNodeWithText("Launch").assertExists()
        ui.onNodeWithText("1/2").assertExists()
        ui.onNodeWithText("2").assertExists()
        ui.onNodeWithText("Confirm to complete · 30s undo").assertExists()
        ui.onNodeWithText("Task").assertDoesNotExist()
    }

    // The wire allows 256 rows; the card renders the reference's eight and counts the rest.
    @Test fun a_long_checklist_is_bounded_at_the_reference_row_cap() {
        val items=(1..40).map {CardItem("i$it","Item $it",it<=3,true)}
        render(message(MessagePart("c","checklist","Long",items=items)))
        ui.onNodeWithText("Item 8").assertExists()
        ui.onNodeWithText("Item 9").assertDoesNotExist()
        ui.onNodeWithText("+32 more").assertExists()
        ui.onNodeWithText("3/40").assertExists()
    }

    @Test fun a_checklist_keeps_its_progress_count_and_prints_no_type_word() {
        val items=listOf(CardItem("a","Milk",true,true),CardItem("b","Bread",false,true))
        render(message(MessagePart("c","checklist","Shopping",items=items)))
        ui.onNodeWithText("Shopping").assertExists()
        ui.onNodeWithText("1/2").assertExists()
        ui.onNodeWithText("Checklist").assertDoesNotExist()
        ui.onNodeWithText("Confirm to complete · 30s undo").assertDoesNotExist()
    }

    @Test fun a_recurring_list_marks_kept_rows_without_a_replay_glyph() {
        val items=listOf(CardItem("a","Water the plants",false,true,persistent=true),CardItem("b","Buy a notebook",false,true))
        render(message(MessagePart("r","recurring","Weekly",items=items,date="Weekly · resets Friday 12:01 AM")))
        ui.onNodeWithText("Weekly · resets Friday 12:01 AM").assertExists()
        ui.onNodeWithText("Water the plants").assertExists()
        ui.onAllNodesWithContentDescription("Kept through resets").assertCountEquals(1)
        ui.onNodeWithText("Recurring").assertDoesNotExist()
    }

    @Test fun the_countdown_titles_beside_the_glyph_and_reads_its_unit_beside_the_figure() {
        val at=kotlin.time.Clock.System.now().epochSeconds+90061L
        render(message(MessagePart("d","countdown","Release day",at=at,date="Mar 3, 9:30 AM")))
        ui.onNodeWithContentDescription("Countdown").assertExists()
        ui.onNodeWithText("Release day").assertExists()
        ui.onNodeWithContentDescription("1 day to go").assertExists()
        ui.onNodeWithText("Countdown",substring=false).assertDoesNotExist()
        ui.onAllNodesWithText("Until",substring=true).assertCountEquals(0)
        ui.onAllNodesWithText("Open",substring=true).assertCountEquals(0)
    }

    @Test fun elapsed_time_leads_with_a_figure_and_follows_with_the_breakdown() {
        val at=kotlin.time.Clock.System.now().epochSeconds-(400L*86400L)-7200L
        render(message(MessagePart("g","ago","The project began",at=at)))
        ui.onNodeWithContentDescription("Elapsed time").assertExists()
        ui.onNodeWithText("The project began").assertExists()
        ui.onNodeWithContentDescription("1 year ago").assertExists()
        ui.onAllNodesWithText("month",substring=true).assertCountEquals(1)
    }

    @Test fun a_reminder_leads_with_its_absolute_time_not_a_span_figure() {
        val at=kotlin.time.Clock.System.now().epochSeconds+50400L
        render(message(MessagePart("m","reminder","Call the vet",at=at,date="Tomorrow, 9:30 AM")))
        ui.onNodeWithContentDescription("Reminder").assertExists()
        ui.onNodeWithText("Call the vet").assertExists()
        // The span moves with the wall clock; only its lead-in is asserted.
        ui.onNodeWithText("Tomorrow, 9:30 AM · in ",substring=true).assertExists()
    }

    @Test fun an_untitled_temporal_card_falls_back_to_its_glyph_not_its_type_word() {
        val now=kotlin.time.Clock.System.now().epochSeconds
        render(message(MessagePart("d","countdown","",at=now+90061L),MessagePart("m","reminder","",at=now+50400L,date="Tomorrow, 9:30 AM"),MessagePart("g","ago","",at=now-864000L)))
        ui.onNodeWithContentDescription("Countdown").assertExists()
        ui.onNodeWithContentDescription("Reminder").assertExists()
        ui.onNodeWithContentDescription("Elapsed time").assertExists()
        ui.onNodeWithText("Countdown").assertDoesNotExist()
        ui.onNodeWithText("Reminder").assertDoesNotExist()
        ui.onNodeWithText("Elapsed time").assertDoesNotExist()
    }

    @Test fun a_note_is_set_apart_from_the_bubble_it_sits_in() {
        render(message(MessagePart("n","note","Gate code is on the fridge")))
        ui.onNodeWithContentDescription("Note").assertExists()
        ui.onNodeWithText("Gate code is on the fridge").assertExists()
        ui.onNodeWithText("Note").assertDoesNotExist()
    }

    @Test fun the_timer_reads_out_as_a_clock_and_carries_no_details_affordance() {
        // The spoken span is a pure function of the remaining seconds; the rendered card is only checked for its shape.
        assertEquals("1 hour 1 minute",temporalSpan(3700L))
        val now=kotlin.time.Clock.System.now().epochSeconds
        render(message(MessagePart("z","timer","Timer",at=now+3700L,startedAt=now-100L)))
        ui.onNodeWithContentDescription(" remaining",substring=true).assertExists()
        ui.onNodeWithText("Remaining").assertExists()
        ui.onNodeWithText("Timer").assertDoesNotExist()
        ui.onAllNodesWithText("Details",substring=true).assertCountEquals(0)
    }

    // The reference rings the whole card, not the digit row: the cue must change pixels outside the flap.
    @Test fun the_expiry_cue_rings_around_the_whole_card() {
        ui.mainClock.autoAdvance=false
        val now=kotlin.time.Clock.System.now().epochSeconds
        ui.setContent {MaterialTheme {Box(Modifier.width(400.dp).padding(24.dp)) {MessageCards(message(MessagePart("z","timer","Timer",at=now+2L,startedAt=now-8L)),{""},null)}}}
        ui.mainClock.advanceTimeByFrame()
        // Wait out the real seconds the card is counting, then step to the frame the cue starts on.
        Thread.sleep(2400)
        var guard=0
        while(ui.onAllNodesWithContentDescription("Timer ended").fetchSemanticsNodes().isEmpty() && guard++<300)ui.mainClock.advanceTimeBy(16)
        val flap=ui.onNodeWithContentDescription("Timer ended").fetchSemanticsNode().boundsInRoot
        ui.mainClock.advanceTimeBy(200)
        val early=pixels()
        ui.mainClock.advanceTimeBy(400)
        val late=pixels()
        val (width,changed)=diff(early,late)
        assertTrue(changed.isNotEmpty(),"The expiry cue must animate something")
        assertTrue(changed.any {it/width<flap.top-1f},"The cue must reach above the digit row, not hug it")
        assertTrue(changed.any {it%width<flap.left-1f},"The cue must reach left of the digit row, not hug it")
    }

    // The ring and the fade that follows it belong to a timer seen expiring, never to one scrolled into view long after.
    @Test fun a_timer_that_was_never_seen_running_shows_its_ended_face_without_animating() {
        ui.mainClock.autoAdvance=false
        render(message(MessagePart("z","timer","Timer",at=1600000000L,startedAt=1599999970L)))
        ui.mainClock.advanceTimeByFrame()
        val settled=pixels()
        ui.mainClock.advanceTimeBy(MotionRing+MotionSettle+240L)
        assertEquals(settled,pixels(),"An already-ended timer must not replay the ring or the fade")
        ui.onNodeWithContentDescription("Timer ended").assertExists()
    }

    // Indices of the pixels that differ between two captures, with the row width they index into.
    private fun diff(before:List<androidx.compose.ui.graphics.Color>,after:List<androidx.compose.ui.graphics.Color>):Pair<Int,List<Int>> {
        val width=ui.onRoot().captureToImage().toPixelMap().width
        return width to before.indices.filter {before[it]!=after[it]}
    }

    private fun pixels():List<androidx.compose.ui.graphics.Color> {
        val map=ui.onRoot().captureToImage().toPixelMap()
        return buildList {repeat(map.height) {y->repeat(map.width) {x->add(map[x,y])}}}
    }
}
