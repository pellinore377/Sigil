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
    private fun card(u:UtilityContent)=listOf(MessagePart("u","card","",utility=u))
    private fun rich(text:String)=RichText(text)
    @Test fun card_quotes_carry_the_create_glyph_and_what_identifies_the_card() {
        assertEquals("Poll · Lunch?",cardQuote(listOf(MessagePart("p","poll","Lunch?")))?.line)
        assertEquals("ballot",cardQuote(listOf(MessagePart("p","poll","Lunch?")))?.glyph)
        assertEquals("Rating · 4/5",cardQuote(card(UtilityContent("rating",display="4/5")))?.line)
        assertEquals("Rating",cardQuote(card(UtilityContent("rating")))?.line)
        assertEquals("Coin · Heads",cardQuote(card(UtilityContent("pick",motion=RandomizerMotion("coin",frames=listOf("Heads","Tails"),selected=0))))?.line)
        assertEquals("Dice · 4 + 6 = 10",cardQuote(card(UtilityContent("dice",details=listOf(rich("d6 · 4"),rich("d6 · 6")),motion=RandomizerMotion("dice",result="10"))))?.line)
        assertEquals("Dice · 17",cardQuote(card(UtilityContent("dice",details=listOf(rich("d20 · 17")),motion=RandomizerMotion("dice",result="17"))))?.line)
        assertEquals("Dice · 1 + 2 + 3 + 4 + 5 + 6 + … = 28",cardQuote(card(UtilityContent("dice",details=(1..7).map {rich("d6 · $it")},motion=RandomizerMotion("dice",result="28"))))?.line)
        assertEquals("Calculation · 17 × 34 = 578",cardQuote(card(UtilityContent("calculation",display="578",rich=rich("17 * 34"))))?.line)
        assertEquals("Calculation · 2⁸ ÷ 4 = 64",cardQuote(card(UtilityContent("calculation",display="64",rich=rich("2 ^ 8 / 4"))))?.line)
        assertEquals("Conversion · 20 C = 68 F",cardQuote(card(UtilityContent("conversion",display="20 C",alternate="68 F")))?.line)
        assertEquals("Random number · 42 · Between 1 and 100",cardQuote(card(UtilityContent("random",display="42",alternate="Between 1 and 100")))?.line)
        assertEquals("Pick · A quiet café",cardQuote(card(UtilityContent("pick",display="Choice",rich=rich("A quiet café"))))?.line)
        assertEquals("Keyboard shortcut · Ctrl + Shift + P",cardQuote(card(UtilityContent("keys",details=listOf(rich("Ctrl"),rich("Shift"),rich("P")))))?.line)
        assertEquals("Keyboard shortcut · Cmd + Up arrow",cardQuote(card(UtilityContent("keys",details=listOf(rich("cmd"),rich("up")))))?.line)
        assertEquals("Color swatch · #6E84D2",cardQuote(card(UtilityContent("swatch",display="#6e84d280",rgba=0x6e84d280L)))?.line)
        assertEquals("Quote · “Leave room.” · Studio notes",cardQuote(card(UtilityContent("quote",rich=rich("Leave room."),secondary=rich("Studio notes"))))?.line)
        assertEquals("QR code · Synthetic network",cardQuote(card(UtilityContent("qr",rich=rich("Synthetic network"),qr=QrContent("wifi",1,"1","WIFI:S:x;P:secret;;"))))?.line)
        assertNull(cardQuote(listOf(MessagePart("t","text","hi"))))
    }
    @Test fun data_and_structured_quotes_name_their_title() {
        val chart=ChartContent("pie",rich("A little balance"),false,0f,emptyList(),emptyList(),null,listOf(ChartPoint(rich("Making"),0f,0f,"42",null,.42f,"42")))
        val quote=cardQuote(listOf(MessagePart("c","card","",chart=chart)))!!
        assertEquals("Pie chart · A little balance",quote.line)
        assertSame(chart,quote.chart)
        val diagram=DiagramContent("org",rich(""),1f,1f,listOf(DiagramNode(rich("Lead"),"box",0f,0f),DiagramNode(rich("Design"),"box",0f,0f)),emptyList(),emptyList())
        assertEquals("Org chart · Lead → Design",cardQuote(listOf(MessagePart("d","card","",diagram=diagram)))?.line)
        val table=TableContent(listOf(rich("Name"),rich("Role")),listOf(listOf(rich("Ari"),rich("Design"))),emptyList(),emptyList(),null)
        assertEquals("Table · Name, Role · 1 row",cardQuote(listOf(MessagePart("t","card","",table=table)))?.line)
        val items=listOf(CardItem("1","Milk",true,true),CardItem("2","Eggs",false,true))
        assertEquals("Checklist · Groceries · 1 of 2 done",cardQuote(listOf(MessagePart("l","checklist","Groceries",items)))?.line)
        assertEquals("Reminder · Call the studio · Tue 09:00",cardQuote(listOf(MessagePart("r","reminder","Call the studio",date="Tue 09:00")))?.line)
        assertEquals("Location · Dropped pin",cardQuote(listOf(MessagePart("g","location","Dropped pin")))?.line)
        assertEquals("Location · Live location",cardQuote(listOf(MessagePart("g","location","",locationMode="live")))?.line)
        assertEquals("Contact · Ari",cardQuote(listOf(MessagePart("a","card","",contact=ContactContent("@ari:studio.example",rich("Ari"),"x",null))))?.line)
        assertEquals("Recipe · Lemon pasta · serves 2",cardQuote(listOf(MessagePart("f","card","",recipe=RecipeContent(rich("Lemon pasta"),2,2,null,emptyList(),emptyList(),emptyList()))))?.line)
    }
    @Test fun quotes_keep_spoilers_covered() {
        val hidden=RichText("The answer is 42",listOf(RichSpan(14,16,reveal="tap")))
        assertEquals("The answer is •••",hidden.quotePlain())
        assertEquals("The answer is •••",quoteText(listOf(MessagePart("t","text","The answer is 42",rich=hidden)),"The answer is 42"))
        assertEquals("plain",quoteText(listOf(MessagePart("t","text","plain")),"plain"))
        assertEquals("Poll · Pick •••",cardQuote(listOf(MessagePart("p","poll","Pick 7",rich=RichText("Pick 7",listOf(RichSpan(5,6,redaction=1))))))?.line)
    }
    @Test fun a_math_reply_shows_the_formula_not_its_source() {
        val formula=UtilityContent("math",display="\\frac{a}{b}",math=MathTypeset(1000f,1f,.7f,.3f,mapOf(1 to "M0 0L500 0L500 700Z"),listOf(MathRun(1,0f,0f,1f,null)),emptyList()))
        ui.setContent {MaterialTheme {Column(Modifier.width(360.dp)) {
            MessageBubble(message("\\frac{a}{b}","a",false).copy(replyParts=card(formula)),false,false,{""})
            ContextChip("Maya Chen","\\frac{a}{b}",card=cardQuote(card(formula))) {}
        }}}
        ui.onAllNodesWithText("\\frac{a}{b}",substring=true).assertCountEquals(0)
        ui.onAllNodesWithContentDescription("Formula. \\frac{a}{b}").assertCountEquals(2)
    }
    @Test fun a_card_reply_renders_its_line_in_bubble_and_chip_and_threads_decode_their_root() {
        val dice=card(UtilityContent("dice",details=listOf(rich("d6 · 4"),rich("d6 · 6")),motion=RandomizerMotion("dice",result="10")))
        ui.setContent {MaterialTheme {Column(Modifier.width(360.dp)) {
            MessageBubble(message("roll::2d6;","a",false).copy(replyParts=dice),false,false,{""})
            ContextChip("Maya Chen","roll::2d6;",card=cardQuote(dice)) {}
        }}}
        ui.onAllNodesWithText("Dice · 4 + 6 = 10").assertCountEquals(2)
        ui.onAllNodesWithText("roll::2d6;",substring=true).assertCountEquals(0)
        val messages=StateDecoder.messages(Json.parseToJsonElement("""{"messages":[{"id":"m","author":"a","text":"Reply","mine":false,"timestamp":1,"delivery":"Sent","pinned":false,"reactions":[],"my_reactions":[],"reply":null,"read_by_me":true,"readers":[],"noted":false,"attachment":null,"parts":[],"thread_parts":[{"id":"p","kind":"poll","text":"Lunch?","items":[]}]}]}""").jsonObject,"peer",{"t"})
        assertEquals("Poll · Lunch?",cardQuote(messages.single().threadParts)?.line)
    }
}
