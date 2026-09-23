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
import kotlin.math.abs
import kotlin.test.*

class ShareEncodeCardsTest {
    @get:Rule val ui=createComposeRule()

    private fun message(vararg parts:MessagePart)=ChatMessage("card","author","",false,"9:41","read",false,emptyList(),emptyList(),null,true,peer="conversation",parts=parts.toList())
    private fun render(message:ChatMessage,width:Int=400,command:Command?=null) {ui.setContent {MaterialTheme {Box(Modifier.width(width.dp)) {MessageCards(message,{""},command)}}}}
    private fun bounds(matcher:SemanticsMatcher)=ui.onNode(matcher,true).fetchSemanticsNode().boundsInRoot
    private val cells=(0 until 29*29).map {if(it/29 in 4..24 && it%29 in 4..24 && it%2==0)'1' else '0'}.joinToString("")

    private fun contactPart(name:RichText)=MessagePart("c1","card","Shared contact",contact=ContactContent("@sam:example.test",name,"01".repeat(32),null))

    @Test fun a_contact_reads_as_one_named_row_then_one_message_action() {
        val sent=mutableListOf<Pair<String,Map<String,Any?>>>()
        render(message(contactPart(RichText("Sam Example"))),command={name,fields->sent+=name to fields})
        ui.onNodeWithText("Contact",substring=false,useUnmergedTree=true).assertDoesNotExist()
        val row=bounds(hasContentDescription("Contact, Sam Example, @sam:example.test",substring=false))
        val send=bounds(hasContentDescription("Message Sam Example"))
        assertTrue(send.top>=row.bottom && send.height>=48f,"Message is a full-height action under the identity")
        ui.onNodeWithContentDescription("Message Sam Example").performClick()
        ui.runOnIdle {assertEquals(listOf("contact_open" to mapOf<String,Any?>("peer" to "conversation","author" to "author","message" to "card","card" to "c1")),sent)}
    }

    @Test fun an_outgoing_contact_carries_no_action() {
        ui.setContent {MaterialTheme {Box(Modifier.width(400.dp)) {MessageCards(message(contactPart(RichText("Sam Example"))).copy(mine=true),{""},{_,_->})}}}
        ui.onNodeWithText("Message").assertDoesNotExist()
    }

    @Test fun a_concealed_contact_name_never_reaches_the_monogram_or_the_label() {
        render(message(contactPart(RichText("Private name",listOf(RichSpan(0,12,reveal="spoiler"))))),command={_,_->})
        ui.onNodeWithContentDescription("Contact, hidden name, @sam:example.test",substring=false).assertExists()
        ui.onNodeWithContentDescription("Private name",substring=true).assertDoesNotExist()
        ui.onNodeWithContentDescription("Message this contact").assertExists()
        assertEquals("AC",contactInitials("Ari  Chen Lee"))
        assertEquals("PP",contactInitials("\uD83C\uDF89 Party Planner"),"An emoji word is skipped, never split")
        assertEquals("",contactInitials("\uD83C\uDF89"),"No letters falls back to the person glyph")
    }

    @Test fun a_long_name_at_large_text_stops_at_three_lines() {
        val name="Alexandria Montgomery-Featherstonehaugh of the Very Long Name Society and Associated Clubs of Everywhere"
        ui.setContent {MaterialTheme {androidx.compose.runtime.CompositionLocalProvider(androidx.compose.ui.platform.LocalDensity provides androidx.compose.ui.unit.Density(1f,1.3f)) {Box(Modifier.width(360.dp)) {
            MessageCards(message(contactPart(RichText(name))),{""},{_,_->})
        }}}}
        val row=bounds(hasContentDescription("Contact, ",substring=true))
        val type=androidx.compose.material3.Typography()
        val cap=(type.titleMedium.lineHeight.value*3+4+type.labelMedium.lineHeight.value*2)*1.3f
        assertTrue(row.height<=cap+2f,"Three name lines and the address at most, got ${row.height} of $cap")
        ui.onNodeWithContentDescription("Message $name").assertExists()
    }

    @Test fun a_link_qr_shows_the_url_under_a_snapped_tile_and_opens_it() {
        render(message(MessagePart("u","utility","",utility=UtilityContent("qr",rich=RichText("https://example.org"),qr=QrContent("url",29,cells,"https://example.org")))))
        val tile=bounds(hasContentDescription("QR code for https://example.org"))
        assertEquals(tile.width,tile.height,"The tile is square")
        assertEquals(0f,tile.width%29f,"A whole number of modules")
        assertTrue(bounds(hasText("https://example.org")).top>=tile.bottom,"The payload reads under the code")
        ui.onNodeWithText("Open link").assertExists()
        ui.onNodeWithText("Copy QR contents").assertDoesNotExist()
        ui.onNode(hasContentDescription("QR code for https://example.org")).assertHasClickAction()
    }

    @Test fun a_wifi_qr_names_the_network_hides_the_password_and_copies_only_the_password() {
        var copied=""
        ui.setContent {MaterialTheme {androidx.compose.runtime.CompositionLocalProvider(LocalSensitiveCopy provides {copied=it}) {Box(Modifier.width(400.dp)) {
            MessageCards(message(MessagePart("u","utility","",utility=UtilityContent("qr",rich=RichText("Synthetic network"),qr=QrContent("wifi",29,cells,"WIFI:T:WPA;S:Synthetic;P:synthetic-secret;;",RichText("synthetic-secret"),false)))),{""},null)
        }}}}
        ui.onNodeWithText("Synthetic network",useUnmergedTree=true).assertExists()
        ui.onNodeWithText("Wi-Fi network · password hidden").assertExists()
        ui.onNodeWithText("synthetic-secret").assertDoesNotExist()
        ui.onNodeWithText("Copy password").performClick()
        ui.runOnIdle {assertEquals("synthetic-secret",copied)}
    }

    @Test fun a_concealed_text_qr_waits_for_reveal() {
        render(message(MessagePart("u","utility","",utility=UtilityContent("qr",rich=RichText("Text QR code"),qr=QrContent("text",29,cells,"secret words",concealed=true)))))
        ui.onNodeWithContentDescription("QR code").assertDoesNotExist()
        ui.onNodeWithText("secret words").assertDoesNotExist()
        ui.onNodeWithText("Reveal QR code").performClick()
        ui.onNodeWithContentDescription("QR code").assertExists()
        ui.onNodeWithText("secret words").assertExists()
    }

    @Test fun an_incoming_contact_qr_messages_the_account() {
        val sent=mutableListOf<String>()
        val qr=MessagePart("q1","utility","",utility=UtilityContent("qr",rich=RichText("@ari:studio.example"),qr=QrContent("contact",29,cells,"sigil:contact:@ari:studio.example:"+"01".repeat(32))))
        render(message(qr),command={name,fields->sent+=name+":"+fields["card"]})
        ui.onNodeWithContentDescription("Message @ari:studio.example").performClick()
        ui.runOnIdle {assertEquals(listOf("contact_open:q1"),sent)}
        assertEquals("@ari:studio.example","sigil:contact:@ari:studio.example:ab".contactQrAddress())
    }

    @Test fun an_outgoing_wifi_qr_offers_no_copy() {
        val wifi=MessagePart("u","utility","",utility=UtilityContent("qr",rich=RichText("Synthetic network"),qr=QrContent("wifi",29,cells,"WIFI:T:WPA;S:Synthetic;P:synthetic-secret;;",RichText("synthetic-secret"),false)))
        ui.setContent {MaterialTheme {Box(Modifier.width(400.dp)) {MessageCards(message(wifi).copy(mine=true),{""},{_,_->})}}}
        ui.onNodeWithText("Copy password").assertDoesNotExist()
        ui.onNodeWithText("Wi-Fi network · password hidden").assertExists()
    }

    @Test fun a_qr_reply_quote_names_what_the_code_holds() {
        fun quote(qr:QrContent,label:String)=cardQuote(listOf(MessagePart("u","utility","",utility=UtilityContent("qr",rich=RichText(label),qr=qr))))?.detail
        assertEquals("Take the scenic route.",quote(QrContent("text",29,cells,"Take the scenic route."),"Text QR code"))
        assertEquals("https://example.org",quote(QrContent("url",29,cells,"https://example.org"),"https://example.org"))
        assertEquals("•••",quote(QrContent("text",29,cells,"secret words",concealed=true),"Text QR code"))
    }

    @Test fun ascii_art_never_wraps_or_scrolls_and_wide_art_enlarges() {
        val art="+"+"-".repeat(58)+"+\n|"+" ".repeat(58)+"|\n+"+"-".repeat(58)+"+"
        render(message(MessagePart("u","utility","",utility=UtilityContent("art",display=art))),width=240)
        val node=ui.onNodeWithContentDescription("ASCII art, 3 lines",substring=true).fetchSemanticsNode()
        assertTrue(node.boundsInRoot.width<=240f,"Wide art fits the bubble instead of overflowing it")
        ui.onAllNodes(SemanticsMatcher.keyIsDefined(androidx.compose.ui.semantics.SemanticsProperties.HorizontalScrollAxisRange)).assertCountEquals(0)
        assertEquals(1f,artScale(100f,200f),"Art that fits keeps its size")
        assertEquals(.8f,artScale(250f,200f),"Wider art scales to fit")
        assertEquals(.2f,artScale(1000f,200f),"Very wide art still fits; the enlarged view holds it 1:1")
        ui.onNodeWithContentDescription("ASCII art, 3 lines, shrunk to fit").performClick()
        ui.onNodeWithContentDescription("Close ASCII art").assertExists()
    }

    @Test fun small_art_is_not_a_button() {
        render(message(MessagePart("u","utility","",utility=UtilityContent("art",display="/\\_/\\\n( o.o )"))),width=320)
        ui.onNodeWithContentDescription("ASCII art, 2 lines",substring=false).assertHasNoClickAction()
    }
}
