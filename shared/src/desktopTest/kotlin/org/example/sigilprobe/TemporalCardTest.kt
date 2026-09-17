@file:OptIn(kotlin.time.ExperimentalTime::class)
package org.sigil

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.Modifier
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
        assertEquals("1 month · 1 day",temporalBreakdown(1580428800L,1583020800L))
        // 2019-01-31 .. 2019-03-01 borrows a 28-day February.
        assertEquals("1 month · 1 day",temporalBreakdown(1548892800L,1551398400L))
        // 2019-03-01T06:30 .. 2025-08-13T14:00 reads out to four components, largest first.
        assertEquals("6 years · 5 months · 12 days · 7 hours",temporalBreakdown(1551421800L,1755093600L))
        assertEquals("2 hours · 5 minutes",temporalBreakdown(0L,7500L))
        assertEquals("Less than a minute",temporalBreakdown(0L,59L))
        assertEquals("",temporalBreakdown(100L,100L))
    }

    @Test fun the_flap_keeps_a_fixed_cell_count_inside_each_regime() {
        assertEquals("0:00",temporalDigits(0L))
        assertEquals("9:05",temporalDigits(545L))
        assertEquals("59:59",temporalDigits(3599L))
        assertEquals("1:00:00",temporalDigits(3600L))
        assertEquals("2:03:04",temporalDigits(7384L))
    }

    @Test fun a_task_list_is_recognisable_without_reading_it() {
        val items=listOf(CardItem("a","Book the room",true,false),CardItem("b","Send the agenda",false,true))
        render(message(MessagePart("t","task","Launch",items=items)))
        ui.onNodeWithText("Task").assertExists()
        ui.onNodeWithText("Confirm to complete · 30 seconds to undo").assertExists()
        ui.onNodeWithText("Completed · final").assertExists()
        ui.onNodeWithText("1 of 2 complete").assertDoesNotExist()
    }

    @Test fun a_checklist_keeps_its_progress_meter_and_its_own_glyph() {
        val items=listOf(CardItem("a","Milk",true,true),CardItem("b","Bread",false,true))
        render(message(MessagePart("c","checklist","Shopping",items=items)))
        ui.onNodeWithText("Checklist").assertExists()
        ui.onNodeWithText("1 of 2 complete").assertExists()
        ui.onNodeWithText("Confirm to complete · 30 seconds to undo").assertDoesNotExist()
    }

    @Test fun a_recurring_list_routes_to_the_checklist_card_without_a_replay_glyph() {
        val items=listOf(CardItem("a","Water the plants",false,true))
        render(message(MessagePart("r","recurring","Weekly",items=items,date="Monday, 12:01 AM")))
        ui.onNodeWithText("Recurring").assertExists()
        ui.onNodeWithText("Resets Monday, 12:01 AM").assertExists()
        ui.onNodeWithText("Water the plants").assertExists()
    }

    @Test fun the_countdown_titles_beside_the_glyph_and_never_opens_a_bigger_version() {
        val at=kotlin.time.Clock.System.now().epochSeconds+90061L
        render(message(MessagePart("d","countdown","Release day",at=at,date="Mar 3, 9:30 AM")))
        ui.onNodeWithContentDescription("Countdown").assertExists()
        ui.onNodeWithText("Release day").assertExists()
        ui.onNodeWithText("Countdown",substring=false).assertDoesNotExist()
        ui.onNodeWithText("Until Mar 3, 9:30 AM").assertExists()
        ui.onAllNodesWithText("Open",substring=true).assertCountEquals(0)
    }

    @Test fun a_reminder_leads_with_its_absolute_time_not_a_span_figure() {
        val at=kotlin.time.Clock.System.now().epochSeconds+50400L
        render(message(MessagePart("m","reminder","Call the vet",at=at,date="Tomorrow, 9:30 AM")))
        ui.onNodeWithContentDescription("Reminder").assertExists()
        ui.onNodeWithText("Call the vet").assertExists()
        ui.onNodeWithText("Tomorrow, 9:30 AM").assertExists()
        ui.onNodeWithText("in 14 hours").assertExists()
    }

    @Test fun a_note_is_set_apart_from_the_bubble_it_sits_in() {
        render(message(MessagePart("n","note","Gate code is on the fridge")))
        ui.onNodeWithText("Note").assertExists()
        ui.onNodeWithText("Gate code is on the fridge").assertExists()
    }

    @Test fun the_timer_reads_out_as_a_clock_and_carries_no_details_affordance() {
        val now=kotlin.time.Clock.System.now().epochSeconds
        render(message(MessagePart("z","timer","Timer",at=now+125L,startedAt=now-55L)))
        ui.onNodeWithContentDescription("2 minutes 5 seconds remaining").assertExists()
        ui.onNodeWithText("remaining").assertExists()
        ui.onAllNodesWithText("Details",substring=true).assertCountEquals(0)
    }
}
