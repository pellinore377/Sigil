@file:OptIn(kotlin.time.ExperimentalTime::class)
package org.sigil

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class TimeCardsTest {
    @get:Rule val ui=createComposeRule()

    private fun message(vararg parts:MessagePart)=ChatMessage("card","author","",false,"9:41","read",false,emptyList(),emptyList(),null,true,peer="conversation",parts=parts.toList())
    private fun render(message:ChatMessage,insert:((String)->Unit)?=null) {
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalComposerInsert provides insert) {Box(Modifier.width(400.dp)) {MessageCards(message,{""},null)}}}}
    }
    private val now get()=kotlin.time.Clock.System.now().epochSeconds

    // The sender's calendar day decides Today, not 24 hours from the stored 09:00.
    @Test fun countdowns_count_the_senders_calendar_days_and_never_go_negative() {
        val day=1_800_000_000L.floorDiv(86400L)*86400L
        assertEquals(0L,distanceDays(day+32400L,day,day+82800L))
        assertEquals(-1L,distanceDays(day+32400L,day,day-1L))
        assertEquals(2L,distanceDays(day+32400L,day,day+2*86400L+5L))
        assertEquals(1L,distanceDays(0L,0L,90061L))
        assertEquals("Today" to "the date is here",distanceFigure(0L))
        assertEquals("1" to "day to go",distanceFigure(-1L))
        assertEquals("290" to "days ago",distanceFigure(290L))
        assertEquals("1" to "year to go",distanceFigure(-400L))
        // Elapsed time stops at days: hours and minutes are noise against a date.
        assertEquals("6" to "years ago",elapsedFigure(1551421800L.floorDiv(86400L),1755093600L.floorDiv(86400L)))
        assertEquals("5 months · 12 days",elapsedBreakdown(1551421800L.floorDiv(86400L),1755093600L.floorDiv(86400L)))
        assertNull(elapsedFigure(5L,5L))
    }

    // Civil dates in the sender's calendar, not UTC days from the stored 09:00.
    private fun elapsed(iso:String,dayStart:Long,now:Long):Pair<Pair<String,String>?,String> {
        val from=senderDay(iso,dayStart,dayStart+32400L)
        val to=from+distanceDays(dayStart+32400L,dayStart,now)
        return elapsedFigure(from,to) to elapsedBreakdown(from,to)
    }
    @Test fun elapsed_time_counts_the_senders_civil_days_at_any_hour() {
        // UTC-5 sender read at 00:30 local on 23 October 2026.
        val now=1792733400L
        assertEquals(("22" to "days ago") to "",elapsed("2026-10-01",1790830800L,now))
        assertEquals(("7" to "years ago") to "7 months · 9 days",elapsed("2019-03-14",1552539600L,now))
        // UTC+10 sender at a month boundary: 1 October to 1 November is one month, though UTC still says 30 September.
        assertEquals(("1" to "month ago") to "",elapsed("2026-10-01",1790776800L,1793457000L))
        assertEquals(isoDay("2026-10-01"),senderDay("",1790776800L,1790809200L))
        assertNull(isoDay("2026-02-30"))
    }

    @Test fun a_timed_moment_reads_in_hours_then_minutes_within_a_day() {
        val at=1_800_000_000L
        assertEquals("2" to "hours to go",nearFigure(at,at-7230L))
        assertEquals("10" to "minutes to go",nearFigure(at,at-570L))
        assertEquals("1" to "minute ago",nearFigure(at,at+59L))
        assertEquals("5" to "hours ago",nearFigure(at,at+5*3600L+100L))
        assertNull(nearFigure(at,at+86400L))
        val day=now.floorDiv(86400L)*86400L
        render(message(MessagePart("t","countdown","Soon",at=now+3*3600L+60L,date="23 October 2026, 11:50 PM CDT",dayStart=day,timed=true),
            MessagePart("u","countdown","Earlier",at=now-2*3600L,date="23 October 2026, 1:00 AM CDT",dayStart=day,timed=true)))
        ui.onNodeWithContentDescription("3 hours to go").assertExists()
        ui.onNodeWithContentDescription("2 hours ago").assertExists()
        ui.onNodeWithText("23 October 2026, 11:50 PM CDT").assertExists()
        ui.onAllNodesWithContentDescription("Today",substring=true).assertCountEquals(0)
    }

    @Test fun a_passed_countdown_reads_as_elapsed_time_under_its_date() {
        val start=now.floorDiv(86400L)*86400L-3*86400L
        render(message(MessagePart("d","countdown","Launch",at=start+32400L,date="5 July 2027",dayStart=start)))
        ui.onNodeWithContentDescription("3 days ago").assertExists()
        ui.onNodeWithText("5 July 2027").assertExists()
        ui.onAllNodesWithText("-",substring=true).assertCountEquals(0)
    }

    @Test fun elapsed_time_dated_ahead_counts_down_and_dated_today_is_today() {
        val today=now.floorDiv(86400L)*86400L
        render(message(MessagePart("a","ago","Opening",at=today+10*86400L+32400L,date="Later",dayStart=today+10*86400L),
            MessagePart("b","ago","Started",at=today+32400L,date="Today's date",dayStart=today)))
        ui.onNodeWithContentDescription("10 days to go").assertExists()
        ui.onNodeWithContentDescription("Today the date is here").assertExists()
        ui.onAllNodesWithText("Since",substring=true).assertCountEquals(0)
    }

    @Test fun an_ended_timer_says_when_it_ended_in_place_of_its_track() {
        render(message(MessagePart("z","timer","Timer",at=now-300L,startedAt=now-330L,date="Today, 9:41 AM")))
        ui.onNodeWithContentDescription("Timer ended").assertExists()
        ui.onNodeWithText("Time's up").assertExists()
        ui.onNodeWithText("Ended 9:41 AM · 5 minutes ago").assertExists()
    }

    // A label decoded yesterday still says Today; the card reads the end from absolute time instead.
    @Test fun an_ended_timer_from_yesterday_never_claims_today() {
        render(message(MessagePart("z","timer","Timer",at=now-30*3600L,startedAt=now-30*3600L-60L,date="Today, 9:41 AM")))
        ui.onNodeWithText("Ended 1 day ago").assertExists()
        ui.onAllNodesWithText("Today",substring=true).assertCountEquals(0)
        assertEquals("Ended just now",timerEnded(100L,130L,""))
        assertEquals("Ended 3:00 PM · 2 hours ago",timerEnded(100L,100L+7300L,"21 October, 2026, 3:00 PM"))
    }

    @Test fun timer_and_countdown_quotes_name_the_span_and_match_the_card_glyph() {
        assertEquals(CardQuote("Timer","timer","5 minutes · ends today, 12:13 AM"),cardQuote(listOf(MessagePart("z","timer","Timer",at=1300L,startedAt=1000L,date="Today, 12:13 AM")),now=1100L))
        assertEquals(CardQuote("Timer","timer","5 minutes · ended yesterday, 12:13 AM"),cardQuote(listOf(MessagePart("z","timer","Timer",at=1300L,startedAt=1000L,date="Yesterday, 12:13 AM")),now=1400L))
        assertEquals("calendar_month",cardQuote(listOf(MessagePart("c","countdown","Launch",at=1300L,date="5 July 2027")))?.glyph)
    }

    @Test fun a_running_timer_keeps_its_end_time_to_itself() {
        render(message(MessagePart("z","timer","Timer",at=now+300L,startedAt=now-10L,date="Today, 9:41 AM")))
        ui.onNodeWithText("Remaining").assertExists()
        ui.onNodeWithText("Today, 9:41 AM").assertDoesNotExist()
    }

    private val sheet=ContentDecoder.part("""{"kind":"card","text":"","utility":{"kind":"help","display":"Time","alternate":"Type help:: in the composer to browse without sending.",
        "secondary":{"text":"Reminders, timers, countdowns and elapsed time.","spans":[],"blocks":[]},"entries":[{"label":"Timer","code":"timer::5m;"},{"label":"Countdown","code":"countdown::2027-01-01::New year;"}]}}""")

    @Test fun a_help_sheet_decodes_its_entries_and_prints_no_type_word() {
        assertEquals(listOf(HelpEntry("Timer","timer::5m;"),HelpEntry("Countdown","countdown::2027-01-01::New year;")),sheet.utility?.entries)
        render(message(sheet))
        ui.onNodeWithText("Time").assertExists()
        ui.onNodeWithText("timer::5m;").assertExists()
        ui.onNodeWithText("Type help:: in the composer to browse without sending.").assertExists()
        ui.onNodeWithContentDescription("SigilText help. Time").assertExists().assert(SemanticsMatcher.keyIsDefined(androidx.compose.ui.semantics.SemanticsProperties.Heading))
        ui.onAllNodesWithContentDescription("SigilText help",substring=true).assertCountEquals(1)
        ui.onNodeWithText("Help").assertDoesNotExist()
        // Without a composer there is nothing to try an example in, so rows take no tap.
        ui.onNodeWithText("timer::5m;").assertHasNoClickAction()
    }

    @Test fun tapping_a_help_row_tries_its_example_in_the_composer() {
        val inserted=mutableListOf<String>()
        render(message(sheet)) {inserted+=it}
        ui.onNode(hasText("Countdown") and hasClickAction()).performClick()
        assertEquals(listOf("countdown::2027-01-01::New year;"),inserted)
        assertEquals(CardQuote("SigilText help","help","Time"),cardQuote(listOf(sheet)))
    }
}
