package org.sigil

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.graphics.toAwtImage
import androidx.compose.ui.unit.dp
import kotlinx.serialization.json.*
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class RatingRequestTest {
    @get:Rule val ui=createComposeRule()
    // The core's timeline shape for `rate::5::Dinner?;` after three ratings, one of them the viewer's 4.
    private fun timeline(scale:Int=5,counts:List<Int> =listOf(0,0,1,1,1),own:Int=4,closed:Boolean=false)="""{"messages":[{"id":"m1","author":"a1","text":"","mine":true,"timestamp":1,"read_by_me":true,"attachment":null,
        "parts":[{"id":"c1","kind":"poll","text":"Dinner?","scale":$scale,"multiple":false,"closed":$closed,"can_close":${!closed},"voters":${counts.sum()},
        "items":[${(1..scale).joinToString(",") {"""{"id":"s$it","text":"$it","checked":${it==own},"enabled":${!closed},"count":${counts.getOrElse(it-1){0}}}"""}}]}]}]}"""
    private fun decoded(json:String=timeline())=StateDecoder.messages(Json.parseToJsonElement(json).jsonObject,"group:g",{it.toString()}).single()
    private fun cards(message:ChatMessage,sent:MutableList<Pair<String,Map<String,Any?>>>) =
        ui.setContent {MaterialTheme {Box(Modifier.width(400.dp)) {MessageCards(message,{""},{name,fields->sent+=name to fields})}}}

    @Test fun the_core_shape_decodes_to_a_rating_with_its_average() {
        val part=decoded().parts.single()
        assertEquals(5,part.scale)
        assertEquals("4",ratingAverage(part))
        assertEquals("You rated 4 · 4 average · 3 ratings",ratingFoot(part,4))
        assertEquals("4.3",ratingAverage(part.copy(items=part.items.mapIndexed {i,it->it.copy(count=listOf(0L,0L,0L,2L,1L)[i])})))
        assertEquals("Tap to rate",ratingFoot(part.copy(voters=0,items=part.items.map {it.copy(count=0)}),0))
        assertEquals("Results after you rate",ratingFoot(part.copy(voters=null),0))
        assertEquals("Rating closed · No ratings",ratingFoot(part.copy(closed=true,voters=0),0))
        // Counts still arriving or hidden are not "No ratings".
        assertEquals("Rating closed",ratingFoot(part.copy(closed=true,voters=null),0))
        assertEquals("Rating closed · You rated 4",ratingFoot(part.copy(closed=true,voters=null),4))
        assertEquals(CardQuote("Rating","star","Dinner?").line,cardQuote(listOf(part))?.line)
    }

    @Test fun tapping_a_star_sends_that_score_and_shows_it_at_once() {
        val sent=mutableListOf<Pair<String,Map<String,Any?>>>()
        cards(decoded(),sent)
        ui.onNodeWithContentDescription("Rating. Dinner?").assertExists()
        ui.onNodeWithContentDescription("4 out of 5 stars").assertIsSelected()
        ui.onNodeWithContentDescription("2 out of 5 stars").assertIsNotSelected().performClick()
        ui.onNodeWithContentDescription("2 out of 5 stars").assertIsSelected()
        ui.onNodeWithText("You rated 2",substring=true).assertExists()
        // The score already held is not sent again.
        ui.onNodeWithContentDescription("2 out of 5 stars").performClick()
        ui.runOnIdle {
            val (name,fields)=sent.single()
            assertEquals("card_action",name)
            assertEquals(mapOf("peer" to "group:g","author" to "a1","message" to "m1","card" to "c1","choices" to listOf("s2")),fields)
        }
    }

    @Test fun a_ten_point_scale_offers_numbered_scores() {
        val sent=mutableListOf<Pair<String,Map<String,Any?>>>()
        cards(decoded(timeline(10,listOf(0,0,0,0,0,0,1,1,0,0),own=0)),sent)
        ui.onNodeWithText("7.5 average · 2 ratings").assertExists()
        ui.onAllNodes(SemanticsMatcher.expectValue(SemanticsProperties.Role,androidx.compose.ui.semantics.Role.RadioButton)).assertCountEquals(10)
        ui.onNodeWithContentDescription("10 out of 10").performClick()
        ui.runOnIdle {assertEquals(listOf("s10"),sent.single().second["choices"])}
    }

    @Test fun a_closed_rating_shows_the_final_average_instead_of_controls() {
        val sent=mutableListOf<Pair<String,Map<String,Any?>>>()
        cards(decoded(timeline(counts=listOf(0,0,1,2,2),closed=true)),sent)
        ui.onNodeWithText("Rating closed · You rated 4 · 4.2 average · 5 ratings").assertExists()
        ui.onNodeWithContentDescription("Rating closed, 4.2 out of 5 average").assertExists().performClick()
        ui.onAllNodes(SemanticsMatcher.expectValue(SemanticsProperties.Role,androidx.compose.ui.semantics.Role.RadioButton)).assertCountEquals(0)
        ui.runOnIdle {assertTrue(sent.isEmpty())}
    }

    @Test fun a_closed_rating_without_a_known_average_keeps_disabled_controls() {
        val sent=mutableListOf<Pair<String,Map<String,Any?>>>()
        cards(decoded(timeline(counts=listOf(0,0,0,0,0),own=0,closed=true)),sent)
        ui.onNodeWithText("Rating closed · No ratings").assertExists()
        ui.onNodeWithContentDescription("1 out of 5 star").assertIsNotEnabled().performClick()
        ui.runOnIdle {assertTrue(sent.isEmpty())}
    }

    @Test fun ten_scores_wrap_to_two_rows_of_wide_targets_on_a_phone() {
        cards(decoded(timeline(10,listOf(0,0,0,0,0,0,1,1,0,0),own=0)),mutableListOf())
        for(n in 1..10) {
            val bounds=ui.onNodeWithContentDescription("$n out of 10").getBoundsInRoot()
            assertTrue(bounds.right-bounds.left>=48.dp && bounds.bottom-bounds.top>=48.dp,"cell $n is $bounds")
        }
        val one=ui.onNodeWithContentDescription("1 out of 10").getBoundsInRoot()
        val six=ui.onNodeWithContentDescription("6 out of 10").getBoundsInRoot()
        assertEquals(one.left,six.left)
        assertTrue(six.top>one.top)
    }

    // Phone-density renders for review; skipped unless a directory is given.
    @Test fun renders() {
        val dir=System.getenv("SIGIL_RATING_RENDERS")?.let {java.io.File(it).apply {mkdirs()}} ?: return
        for(dark in listOf(false,true)) {
            ui.setContent {
                androidx.compose.runtime.CompositionLocalProvider(androidx.compose.ui.platform.LocalDensity provides androidx.compose.ui.unit.Density(2f)) {
                    SigilTheme(Appearance(mode=if(dark)"Dark" else "Light"),palette=NativeCore::palette) {
                        androidx.compose.material3.Surface {Box(Modifier.size(360.dp,380.dp)) {FormBuilder("Rating",true,{}) {_,_->}}}
                    }
                }
            }
            javax.imageio.ImageIO.write(ui.onRoot().captureToImage().toAwtImage(),"png",java.io.File(dir,"form-${if(dark)"dark" else "light"}.png"))
        }
    }

    @Test fun the_author_ends_a_rating_from_the_message_menu() {
        val sent=mutableListOf<Pair<String,Map<String,Any?>>>()
        val chat=ChatSummary("group:g","#dinner:example.test","","",true,emptyList(),displayName="Dinner",group=true)
        val state=MessengerState(phase="connected",chats=listOf(chat),selected=chat.id,messages=listOf(decoded()),timelineLoaded=true)
        ui.setContent {MaterialTheme {Box(Modifier.size(420.dp,800.dp)) {ConversationPage(chat,state,TextFieldState(),{""},{name,fields->sent+=name to fields},"",false,null,{})}}}
        ui.onNodeWithContentDescription("1 out of 5 star").performMouseInput {rightClick()}
        ui.onNodeWithText("End rating").performClick()
        ui.onNodeWithText("End rating?").assertExists()
        ui.onNodeWithText("End rating").performClick()
        ui.runOnIdle {assertEquals("poll_close" to mapOf<String,Any?>("peer" to "group:g","author" to "a1","message" to "m1","card" to "c1"),sent.single {it.first=="poll_close"})}
    }

    @Test fun the_rating_form_asks_by_default() {
        ui.setContent {MaterialTheme {Box(Modifier.size(420.dp,800.dp)) {FormBuilder("Rating",true,{}) {_,_->}}}}
        ui.onNodeWithText("Question").assertExists()
        ui.onNodeWithText("Out of 5").assertIsSelected()
        ui.onNodeWithText("Out of 10").performClick().assertIsSelected()
        ui.onNodeWithText("Show").performClick()
        ui.onNodeWithText("Question").assertDoesNotExist()
        ui.onNodeWithText("Out of").assertExists()
    }
}
