package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import kotlinx.serialization.json.*
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class ReplyPresentationTest {
    @get:Rule val ui=createComposeRule()
    private fun message(reply:String?,author:String?,mine:Boolean)=ChatMessage("m","b","I'm good",true,"now","Sent",false,emptyList(),emptyList(),reply,true,replyAuthor=author,replyMine=mine)
    @Test fun quote_names_the_answered_author_and_the_chip_names_the_target() {
        var closed=0
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalMediaSender provides {if(it.author=="a")"Maya Chen" else if(it.mine)"You" else it.author}) {Column(Modifier.width(360.dp)) {
            MessageBubble(message("Hello how are you?","a",false),false,false,{""})
            ContextChip("Maya Chen","Hello how are you?") {closed++}
        }}}}
        ui.onNodeWithTag("reply-quote").assertIsDisplayed()
        ui.onAllNodesWithText("Maya Chen").assertCountEquals(2)
        ui.onAllNodesWithText("Hello how are you?").assertCountEquals(2)
        ui.onNodeWithContentDescription("Cancel reply or edit").performClick()
        ui.runOnIdle {assertEquals(1,closed)}
    }
    @Test fun an_older_core_without_the_author_still_quotes() {
        ui.setContent {MaterialTheme {Column(Modifier.width(360.dp)) {MessageBubble(message("Earlier message",null,false),false,false,{""})}}}
        ui.onNodeWithTag("reply-quote").assertIsDisplayed()
        ui.onNodeWithText("Earlier message").assertIsDisplayed()
    }
    @Test fun the_wire_carries_the_reply_author() {
        val messages=StateDecoder.messages(Json.parseToJsonElement("""{"messages":[{"id":"m","author":"a","text":"Reply","mine":false,"timestamp":1,"delivery":"Sent","pinned":false,"reactions":[],"my_reactions":[],"reply":"Earlier","reply_author":"b","reply_mine":true,"read_by_me":true,"readers":[],"noted":false,"attachment":null,"parts":[]}]}""").jsonObject,"peer",{"t"})
        assertEquals("b",messages.single().replyAuthor)
        assertTrue(messages.single().replyMine)
    }
    @Test fun attachment_quotes_show_a_preview_and_type_and_a_memo_shows_a_microphone() {
        val video=AttachmentDetails("clip.mp4","video/mp4",1200)
        val memo=AttachmentDetails("Voice message.aac","audio/aac",800)
        ui.setContent {MaterialTheme {Column(Modifier.width(360.dp)) {
            MessageBubble(message("Video","a",false).copy(replyAttachment=video),false,false,{""})
            ContextChip("You",null,memo,null) {}
        }}}
        ui.onNodeWithText("Video").assertIsDisplayed()
        ui.onNodeWithText("Voice message").assertIsDisplayed()
        assertEquals("Photo",attachmentLabel(AttachmentDetails("a.jpg","image/jpeg",1)))
        assertEquals("GIF",attachmentLabel(AttachmentDetails("a.gif","image/gif",1)))
        assertEquals("Pdf",attachmentLabel(AttachmentDetails("a.pdf","application/pdf",1)))
    }
    @Test fun card_quotes_carry_the_create_glyph_and_a_result_where_there_is_one() {
        assertEquals("Poll" to "ballot",cardQuote(listOf(MessagePart("p","poll","Lunch?"))))
        assertEquals("4/5" to "star",cardQuote(listOf(MessagePart("u","card","",utility=UtilityContent("rating",display="4/5")))))
        assertEquals("Rating" to "star",cardQuote(listOf(MessagePart("u","card","",utility=UtilityContent("rating")))))
        assertEquals("Heads" to "toll",cardQuote(listOf(MessagePart("u","card","",utility=UtilityContent("pick",motion=RandomizerMotion("coin",frames=listOf("Heads","Tails"),selected=0))))))
        assertEquals("89" to "casino",cardQuote(listOf(MessagePart("u","card","",utility=UtilityContent("dice",motion=RandomizerMotion("dice",result="89"))))))
        assertNull(cardQuote(listOf(MessagePart("t","text","hi"))))
    }
}
