@file:OptIn(kotlin.time.ExperimentalTime::class)
package org.sigil

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class ChecklistCardTest {
    @get:Rule val ui=createComposeRule()
    private val sent=mutableListOf<Map<String,Any?>>()
    private val command:Command={name,fields->if(name=="card_action")sent+=fields}
    private fun message(vararg parts:MessagePart)=ChatMessage("card","author","",false,"9:41","read",false,emptyList(),emptyList(),null,true,peer="conversation",parts=parts.toList())
    private fun render(part:MessagePart,live:Boolean=true) {ui.setContent {MaterialTheme {Box(Modifier.width(400.dp)) {MessageCards(message(part),{""},if(live)command else null)}}}}
    private fun render(part:androidx.compose.runtime.State<MessagePart>) {ui.setContent {MaterialTheme {Box(Modifier.width(400.dp)) {MessageCards(message(part.value),{""},command)}}}}
    private val now get()=kotlin.time.Clock.System.now().epochSeconds

    @Test fun a_tick_shows_at_once_and_sends_the_new_state() {
        render(MessagePart("c","checklist","Shopping",items=listOf(CardItem("a","Milk",false,true),CardItem("b","Bread",true,true))))
        ui.onNodeWithText("1/2").assertExists()
        ui.onNodeWithText("Checklist").assertDoesNotExist()
        ui.onNodeWithContentDescription("Checklist. Shopping").assertExists()
        ui.onNodeWithText("Milk").performClick()
        assertEquals(true,sent.single()["checked"])
        assertEquals("a",sent.single()["item"])
        // The pending tick completes the list before any echo arrives.
        ui.onNodeWithText("All done").assertExists()
        ui.onAllNodes(SemanticsMatcher.expectValue(SemanticsProperties.ToggleableState,ToggleableState.On)).assertCountEquals(2)
    }

    @Test fun a_long_list_shows_five_rows_until_asked_for_all() {
        render(MessagePart("c","checklist","Long",items=(1..40).map {CardItem("i$it","Item $it",it<=3,true)}))
        ui.onNodeWithText("Item 5").assertExists()
        ui.onNodeWithText("Item 6").assertDoesNotExist()
        ui.onNodeWithText("3/40").assertExists()
        ui.onNodeWithText("Show all 40").performClick()
        ui.onNodeWithText("Item 40").assertExists()
        ui.onNodeWithText("Show fewer").assertExists()
    }

    @Test fun a_recurring_list_reads_its_reset_and_marks_kept_rows() {
        val items=listOf(CardItem("a","Water the plants",true,false,persistent=true),CardItem("b","Buy a notebook",false,true))
        render(MessagePart("r","recurring","Weekly",items=items,date="Weekly · resets Today, 12:01 AM"))
        ui.onNodeWithText("Weekly · resets Today, 12:01 AM").assertExists()
        ui.onAllNodesWithContentDescription("Kept through resets").assertCountEquals(1)
        // Only the reset unchecks a recurring row.
        ui.onNodeWithText("Water the plants").performClick()
        assertTrue(sent.isEmpty())
    }

    @Test fun a_task_confirms_before_completing_and_names_who_did_it() {
        val items=listOf(CardItem("a","Book the room",false,true),CardItem("b","Send the agenda",true,true,doneAt=now-5,doneMine=true,undoUntil=now+25,done="Today, 9:41 AM"),
            CardItem("c","Print badges",true,false,doneAt=now-600,done="Today, 9:31 AM"))
        render(MessagePart("t","task","Launch",items=items))
        ui.onNodeWithText("Confirm to complete · 30s undo").assertExists()
        ui.onNodeWithText("Completed by you · Today, 9:41 AM").assertExists()
        ui.onNodeWithText("Completed · Today, 9:31 AM").assertExists()
        ui.onAllNodesWithText("Undo").assertCountEquals(1)
        ui.onNodeWithText("Undo").performClick()
        ui.onNodeWithText("Book the room").performClick()
        assertEquals(1,sent.size)
        ui.onNodeWithText("Complete").performClick()
        assertEquals(listOf("b" to false,"a" to true),sent.map {it["item"] to it["checked"]})
    }

    @Test fun undo_leaves_when_its_window_closes() {
        render(MessagePart("t","task","Launch",items=listOf(CardItem("b","Send the agenda",true,true,doneAt=now-29,doneMine=true,undoUntil=now+1,done="Today"))))
        ui.onNodeWithText("Undo").assertExists()
        ui.waitUntil(4000) {ui.onAllNodesWithText("Undo").fetchSemanticsNodes().isEmpty()}
    }

    @Test fun a_reminder_reads_the_viewer_time_then_the_span() {
        render(MessagePart("m","reminder","Call the vet",at=now+50400L,date="Tomorrow, 9:30 AM"),live=false)
        ui.onNode(SemanticsMatcher.expectValue(SemanticsProperties.ContentDescription,listOf("Reminder. Call the vet")) and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Upcoming") and SemanticsMatcher.keyIsDefined(SemanticsProperties.Heading)).assertExists()
        ui.onNodeWithText("Call the vet",substring=false,ignoreCase=false,useUnmergedTree=true).assertExists()
        ui.onNodeWithText("Tomorrow, 9:30 AM").assertExists()
        ui.onNodeWithText("In 14 hours").assertExists()
        assertEquals("Rang 2 days ago",reminderSpan(0L,172800L))
        // One unit past a day, whole minutes within the hour.
        assertEquals("Rang 21 days ago",reminderSpan(0L,21*86400L+20*3600L))
        assertEquals("In 42 minutes",reminderSpan(42*60L+17L,0L))
        assertEquals("In 3 hours 12 minutes",reminderSpan(3*3600L+12*60L+40L,0L))
        assertEquals("Rang just now",reminderSpan(0L,20L))
    }

    @Test fun a_rang_reminder_says_so() {
        render(MessagePart("m","reminder","",at=now-7200L,date="Today, 7:00 AM"),live=false)
        ui.onNode(SemanticsMatcher.expectValue(SemanticsProperties.ContentDescription,listOf("Reminder")) and SemanticsMatcher.expectValue(SemanticsProperties.StateDescription,"Rang")).assertExists()
        ui.onNodeWithText("Rang 2 hours ago").assertExists()
    }

    @Test fun a_recurring_reset_is_labelled_by_interval_in_the_viewer_clock() {
        assertEquals("Weekly · resets Thursday, 12:01 AM",resetLabel("Weekly","October 29, 12:01 AM",4,"Thursday 12:01 AM","America/Chicago","America/Chicago"))
        assertEquals("Weekly · resets Tomorrow, 7:01 AM · Saturday 12:01 AM in Chicago",resetLabel("Weekly","Tomorrow, 7:01 AM",6,"Saturday 12:01 AM","America/Chicago","Europe/Paris"))
        assertEquals("Weekly · resets Thursday, 7:01 AM · Thursday 12:01 AM in Chicago",resetLabel("Weekly","October 29, 7:01 AM",4,"Thursday 12:01 AM","America/Chicago","Europe/Paris"))
        assertEquals("Monthly · resets November 22, 12:01 AM",resetLabel("Monthly","November 22, 12:01 AM",7,"Nov 22, 12:01 AM","UTC","UTC"))
        assertEquals("Yearly · resets January 1, 2027, 12:01 AM · Jan 1 2027, 12:01 AM in New York",resetLabel("Yearly","January 1, 2027, 12:01 AM",5,"Jan 1 2027, 12:01 AM","America/New_York","UTC"))
        val raw="""{"id":"r","kind":"recurring","text":"Chores","date":"Weekly · resets Thursday 12:01 AM","at":1700000000,"interval":"Weekly","timezone":"${viewerZone()}","items":[{"id":"a","text":"Bins","checked":false,"enabled":true,"persistent":true}]}"""
        val day=listOf("Monday","Tuesday","Wednesday","Thursday","Friday","Saturday","Sunday")[viewerWeekday(1700000000L)-1]
        assertEquals("Weekly · resets $day, 7:01 PM",ContentDecoder.messagePart(raw) {"November 14, 7:01 PM"}.date)
        val task="""{"id":"t","kind":"task","text":"Move","items":[{"id":"a","text":"Keys","checked":true,"enabled":true,"done_at":1700000000,"done_mine":false,"done_by":"ab12","undo_until":0}]}"""
        val item=ContentDecoder.messagePart(task) {"local:$it"}.items.single()
        assertEquals("ab12",item.doneBy)
        assertEquals("Completed by Ari · local:1700000000",taskDone(item,"Ari"))
        assertEquals("Completed by you · local:1700000000",taskDone(item.copy(doneMine=true),"Ari"))
    }

    @Test fun others_completions_are_named_and_offer_no_undo() {
        // Earliest completion is theirs; our later one cannot make the task incomplete, so Undo is not offered.
        render(MessagePart("t","task","Launch",items=listOf(CardItem("c","Print badges",true,true,doneAt=now-10,doneMine=false,undoUntil=now+25,done="Today, 9:31 AM",doneBy="Ari"))))
        ui.onNodeWithText("Completed by Ari · Today, 9:31 AM").assertExists()
        ui.onAllNodesWithText("Undo").assertCountEquals(0)
    }

    @Test fun undo_waits_for_the_completion_to_be_dated() {
        render(MessagePart("t","task","Launch",items=listOf(CardItem("a","Book the room",false,true))))
        ui.onNodeWithText("Book the room").performClick()
        ui.onNodeWithText("Complete").performClick()
        ui.onNode(SemanticsMatcher.expectValue(SemanticsProperties.ToggleableState,ToggleableState.On)).assertExists()
        ui.onAllNodesWithText("Undo").assertCountEquals(0)
    }

    @Test fun undo_reads_a_fresh_clock_after_the_card_sat_open() {
        val part=androidx.compose.runtime.mutableStateOf(MessagePart("t","task","Launch",items=listOf(CardItem("b","Send the agenda",false,true))))
        render(part)
        ui.waitForIdle()
        Thread.sleep(3000)
        part.value=part.value.copy(items=listOf(CardItem("b","Send the agenda",true,true,doneAt=now,doneMine=true,undoUntil=now+1,done="Today")))
        ui.onNodeWithText("Undo").assertExists()
        ui.waitUntil(2500) {ui.onAllNodesWithText("Undo").fetchSemanticsNodes().isEmpty()}
    }

    @Test fun a_pending_recurring_tick_cannot_be_reversed() {
        render(MessagePart("r","recurring","Weekly",items=listOf(CardItem("a","Water the plants",false,true,persistent=true)),date="Weekly · resets Friday, 12:01 AM",at=now+86400L))
        ui.onNodeWithText("Water the plants").performClick()
        ui.onNodeWithText("Water the plants").performClick()
        assertEquals(listOf(true),sent.map {it["checked"]})
    }

    @Test fun a_recurring_list_turns_over_at_its_reset_while_open() {
        val items=listOf(CardItem("a","Water the plants",true,false,persistent=true),CardItem("b","Buy a notebook",true,false))
        render(MessagePart("r","recurring","Weekly",items=items,date="Weekly · resets Today, 12:01 AM",at=now+1))
        ui.onNodeWithText("Buy a notebook").assertExists()
        ui.waitUntil(3000) {ui.onAllNodesWithText("Buy a notebook").fetchSemanticsNodes().isEmpty()}
        ui.onNodeWithText("Weekly · reset Today, 12:01 AM").assertExists()
        ui.onNodeWithText("0/1").assertExists()
        ui.onNodeWithText("Water the plants").performClick()
        assertEquals(listOf(true),sent.map {it["checked"]})
    }

    @Test fun a_long_rich_title_ellipsizes_and_opens_on_tap() {
        val long=(1..40).joinToString(" ") {"word$it"}
        render(MessagePart("c","checklist",long,items=listOf(CardItem("a","Milk",false,true)),rich=RichText(long)))
        val title=ui.onNodeWithContentDescription("Checklist. $long")
        val short=title.fetchSemanticsNode().size.height
        title.assertHasClickAction().performClick()
        assertTrue(title.fetchSemanticsNode().size.height>short)
    }
}
