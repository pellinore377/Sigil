package org.sigil

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import kotlinx.serialization.json.*
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class PollTimelineTest {
    @get:Rule val ui=createComposeRule()
    // The shape the core's timeline command gives an open poll that one member has voted in.
    private val timeline="""{"messages":[{"id":"m1","author":"a1","text":"Lunch?","mine":true,"timestamp":1,"read_by_me":true,"attachment":null,
        "parts":[{"id":"c1","kind":"poll","text":"Lunch?","multiple":false,"closed":false,"can_close":true,"voters":1,
        "items":[{"id":"o1","text":"Soup","checked":false,"enabled":true,"count":0},{"id":"o2","text":"Salad","checked":false,"enabled":true,"count":1}]}]}]}"""
    private fun decoded()=StateDecoder.messages(Json.parseToJsonElement(timeline).jsonObject,"group:g",{it.toString()}).single()

    @Test fun a_browser_timeline_keeps_the_poll_live() {
        val poll=decoded().parts.single()
        assertEquals("c1",poll.id)
        assertEquals(listOf(true,true),poll.items.map {it.enabled})
        assertEquals(listOf(0L,1L),poll.items.map {it.count})
        assertEquals(1L,poll.voters)
        assertTrue(poll.canClose)
        assertEquals("1 vote",pollFoot(poll))
    }

    @Test fun choosing_an_option_sends_a_vote_for_that_card() {
        val sent=mutableListOf<Pair<String,Map<String,Any?>>>()
        ui.setContent {MaterialTheme {Box(Modifier.width(400.dp)) {MessageCards(decoded(),{""},{name,fields->sent+=name to fields})}}}
        ui.onNodeWithText("Salad").performClick()
        ui.runOnIdle {
            val (name,fields)=sent.single()
            assertEquals("card_action",name)
            assertEquals(mapOf("peer" to "group:g","author" to "a1","message" to "m1","card" to "c1","choices" to listOf("o2")),fields)
        }
    }

    @Test fun ending_a_poll_asks_first() {
        var ended=0
        var dismissed=0
        ui.setContent {MaterialTheme {EndPollDialog({dismissed++}) {ended++}}}
        ui.onNodeWithText("End poll?").assertExists()
        ui.onNodeWithText("Keep open").performClick()
        ui.runOnIdle {assertEquals(0,ended);assertEquals(1,dismissed)}
        ui.onNodeWithText("End poll").performClick()
        ui.runOnIdle {assertEquals(1,ended)}
    }

    private fun page(message:ChatMessage,sent:MutableList<Pair<String,Map<String,Any?>>>) {
        val chat=ChatSummary("group:g","#lunch:example.test","","",true,emptyList(),displayName="Lunch",group=true)
        val state=MessengerState(phase="connected",chats=listOf(chat),selected=chat.id,messages=listOf(message),timelineLoaded=true)
        ui.setContent {MaterialTheme {Box(Modifier.size(420.dp,800.dp)) {ConversationPage(chat,state,TextFieldState(),{""},{name,fields->sent+=name to fields},"",false,null,{})}}}
    }

    @Test fun the_author_ends_a_poll_from_a_right_click_on_it() {
        val sent=mutableListOf<Pair<String,Map<String,Any?>>>()
        page(decoded(),sent)
        ui.onNodeWithText("Salad").performMouseInput {rightClick()}
        ui.onNodeWithText("End poll").performClick()
        ui.onNodeWithText("End poll?").assertExists()
        ui.onNodeWithText("End poll").performClick()
        ui.runOnIdle {
            assertTrue(sent.none {it.first=="card_action"})
            assertEquals("poll_close" to mapOf<String,Any?>("peer" to "group:g","author" to "a1","message" to "m1","card" to "c1"),sent.single {it.first=="poll_close"})
        }
    }

    @Test fun a_right_click_on_a_text_bubble_opens_the_message_actions() {
        val sent=mutableListOf<Pair<String,Map<String,Any?>>>()
        page(decoded().copy(text="Plain words",parts=emptyList()),sent)
        ui.onNodeWithText("Plain words",useUnmergedTree=true).performMouseInput {rightClick()}
        ui.onNodeWithText("Reply").assertExists()
    }

    @Test fun a_long_press_on_an_option_opens_the_menu_without_voting() {
        val sent=mutableListOf<Pair<String,Map<String,Any?>>>()
        page(decoded(),sent)
        ui.onNodeWithText("Soup").performTouchInput {longClick()}
        ui.onNodeWithText("End poll").assertExists()
        ui.runOnIdle {assertTrue(sent.none {it.first=="card_action"})}
    }

    @Test fun a_member_who_did_not_ask_cannot_end_the_poll() {
        val sent=mutableListOf<Pair<String,Map<String,Any?>>>()
        val message=decoded().let {m->m.copy(mine=false,parts=m.parts.map {it.copy(canClose=false)})}
        page(message,sent)
        ui.onNodeWithText("Salad").performMouseInput {rightClick()}
        ui.onNodeWithText("Reply").assertExists()
        ui.onNodeWithText("End poll").assertDoesNotExist()
    }

    @Test fun each_poll_in_a_message_has_its_own_end() {
        val sent=mutableListOf<Pair<String,Map<String,Any?>>>()
        val message=decoded().let {m->m.copy(parts=m.parts+m.parts.single().copy(id="c2",text="Dinner?",items=listOf(CardItem("o3","Pasta",false,true,0))))}
        page(message,sent)
        ui.onNodeWithText("Pasta").performMouseInput {rightClick()}
        ui.onNodeWithText("End poll: Dinner?").performClick()
        ui.onNodeWithText("Dinner?",substring=true).assertExists()
        ui.onNodeWithText("End poll").performClick()
        ui.runOnIdle {assertEquals("c2",sent.single {it.first=="poll_close"}.second["card"])}
    }
}
