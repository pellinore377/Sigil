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

class UtilityPresentationTest {
    @get:Rule val ui=createComposeRule()

    private fun message(vararg parts:MessagePart)=ChatMessage("card","author","",false,"9:41","read",false,emptyList(),emptyList(),null,true,peer="conversation",parts=parts.toList())
    private fun render(message:ChatMessage,command:Command?=null) {ui.setContent {MaterialTheme {Box(Modifier.width(400.dp)) {MessageCards(message,{""},command)}}}}
    private fun utility(value:UtilityContent,command:Command?=null)=render(message(MessagePart("u","utility","",utility=value)),command)
    private fun bounds(matcher:SemanticsMatcher,unmerged:Boolean=false)=ui.onNode(matcher,unmerged).fetchSemanticsNode().boundsInRoot

    @Test fun a_code_block_labels_its_language_at_the_foot_and_offers_no_expanded_view() {
        val body="let total = 4;"
        render(message(MessagePart("c","text",body,rich=RichText(body,blocks=listOf(RichBlock(0,body.length,"code",language="rust"))))))
        ui.onNodeWithText("Open code").assertDoesNotExist()
        ui.onNodeWithText("Code",substring=false).assertDoesNotExist()
        ui.onNodeWithContentDescription("Copy code").assertDoesNotExist()
        val code=bounds(hasText(body))
        val language=bounds(hasText("rust"))
        assertTrue(code.bottom<=language.top,"The language label sits under the code, not over it")
        assertTrue(language.right>=code.right-8f,"The language label is pinned to the trailing edge")
        ui.onNode(hasText(body)).assertHasNoClickAction()
    }

    @Test fun a_quote_pairs_its_marks_and_leads_the_attribution_with_an_em_dash() {
        utility(UtilityContent("quote",rich=RichText("Simple is better."),secondary=RichText("Sam Example"),details=listOf(RichText("Field notes"))))
        val quotation=bounds(hasText("Simple is better."))
        val opening=bounds(hasText("“"))
        val closing=bounds(hasText("”"))
        assertTrue(opening.right<=quotation.left,"The opening mark leads the quotation")
        assertTrue(closing.left>=quotation.right,"The closing mark trails the quotation")
        val dash=bounds(hasText("—"))
        val author=bounds(hasText("Sam Example"))
        assertTrue(dash.right<=author.left,"An em dash introduces the attribution")
        assertTrue(dash.top>=quotation.top,"The attribution follows the quotation")
        ui.onNodeWithText("Field notes").assertExists()
        ui.onNodeWithText("Show all 1").assertDoesNotExist()
    }

    @Test fun a_shortcut_joins_text_height_keycaps_with_a_plus() {
        utility(UtilityContent("keys",details=listOf(RichText("Ctrl"),RichText("Shift"),RichText("P"))))
        assertEquals(2,ui.onAllNodesWithText("+").fetchSemanticsNodes().size,"Every gap between keys carries one joiner")
        val first=bounds(hasText("Ctrl"))
        val last=bounds(hasText("P"))
        assertTrue(first.right<=last.left,"The keys run left to right on one line")
        assertTrue(last.top<first.bottom,"A shortcut is one row, not a stack")
        assertTrue(first.height<48f,"A keycap stays close to the height of the text beside it")
    }

    @Test fun a_swatch_is_a_text_height_rectangle_beside_its_value() {
        utility(UtilityContent("swatch",display="#ff5733",copy="#ff5733",rgba=0xff5733ffL))
        val chip=bounds(hasContentDescription("Color sample #ff5733"))
        val value=bounds(hasText("#ff5733"))
        assertEquals(1,ui.onAllNodesWithText("#ff5733").fetchSemanticsNodes().size,"The value is written once")
        assertTrue(chip.right<=value.left,"The rectangle leads the value")
        assertTrue(chip.height<40f && chip.width>chip.height,"The rectangle is a text-height bar, not a slab")
        assertTrue(abs(chip.center.y-value.center.y)<6f,"The rectangle is centred on the line of text")
    }

    @Test fun a_qr_code_snaps_its_tile_and_carries_its_payload_underneath() {
        val cells=(0 until 29*29).map {if(it/29 in 4..24 && it%29 in 4..24 && it%2==0)'1' else '0'}.joinToString("")
        utility(UtilityContent("qr",rich=RichText("Synthetic network"),qr=QrContent("wifi",29,cells,"WIFI:T:WPA;S:Synthetic;P:synthetic-secret;;",RichText("synthetic-secret"),false)))
        val tile=bounds(hasContentDescription("Scannable QR code"))
        val payload=bounds(hasText("Synthetic network"))
        assertEquals(tile.width,tile.height,"The tile is square")
        assertEquals(0f,tile.width%29f,"The tile is a whole number of modules, so no white margin is left over")
        assertTrue(tile.bottom<=payload.top,"The payload reads under the code, not as a heading over it")
        ui.onNodeWithText("Open QR code").assertDoesNotExist()
        ui.onNodeWithText("Copy Wi-Fi details").assertExists()
    }

    @Test fun a_contact_reads_as_avatar_name_handle_and_message() {
        val contact=ContactContent("@sam:example.test",RichText("Sam Example"),"01".repeat(32),null)
        render(message(MessagePart("card","contact","Shared contact",contact=contact)),{_,_->})
        ui.onNodeWithText("Open contact").assertDoesNotExist()
        val avatar=bounds(hasContentDescription("Sam Example"))
        val name=bounds(hasText("Sam Example"))
        val handle=bounds(hasText("@sam:example.test"))
        val send=bounds(hasContentDescription("Message shared contact"))
        assertTrue(avatar.right<=name.left,"The avatar leads the row")
        assertTrue(handle.top>=name.bottom-2f && abs(handle.left-name.left)<2f,"The handle sits directly under the display name")
        assertTrue(send.left>=name.right,"Message sits to the right of the name")
    }

    @Test fun a_vcard_lists_each_field_with_a_type_label_and_a_quiet_copy() {
        ui.setContent {MaterialTheme {Box(Modifier.width(400.dp)) {
            VcardCard("Ada Placeholder",listOf(VcardField(vcardGlyph("TEL"),"+1 555 0100","cell"),VcardField(vcardGlyph("EMAIL"),"someone@example.test","home")))
        }}}
        val value=bounds(hasText("+1 555 0100"))
        val label=bounds(hasText("cell"))
        val copy=bounds(hasContentDescription("Copy cell"))
        assertTrue(label.top>=value.bottom-2f,"The type label sits beneath its value")
        assertTrue(copy.left>=value.right,"The copy affordance closes the row")
        assertTrue(bounds(hasContentDescription("Ada Placeholder")).bottom<=value.top,"The avatar and name head the card")
        ui.onNodeWithContentDescription("Copy home").assertExists()
    }
}
