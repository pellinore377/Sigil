package org.sigil

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Density
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

    @Test fun a_quote_hangs_one_opening_mark_and_sets_the_attribution_on_its_own_line() {
        utility(UtilityContent("quote",rich=RichText("Simple is better."),secondary=RichText("Sam Example"),details=listOf(RichText("Field notes"))))
        ui.onNodeWithText("Quote",substring=false).assertDoesNotExist()
        val card=ui.onNode(hasContentDescription("Quote")).fetchSemanticsNode()
        val quotation=bounds(hasText("Simple is better."),true)
        val by=bounds(hasContentDescription("— Sam Example, Field notes"),true)
        assertTrue(quotation.left-card.boundsInRoot.left>=40f,"The quotation is indented past a hanging margin for the mark")
        assertEquals(quotation.left,by.left,"Quotation and attribution share one start edge")
        assertTrue(by.top>=quotation.bottom,"The attribution sits on its own line under the quotation")
        ui.onNode(hasText("”"),true).assertDoesNotExist()
        assertTrue(card.config.toString().contains("Simple is better."),"The quotation is read as one merged node")
    }

    @Test fun the_quote_mark_stays_inside_the_card_and_meets_the_cap_line_at_large_text() {
        var scale by mutableFloatStateOf(1f)
        val message=message(MessagePart("u","utility","",utility=UtilityContent("quote",rich=RichText("Simple is better."),secondary=RichText("Sam"))))
        ui.setContent {val d=LocalDensity.current; CompositionLocalProvider(LocalDensity provides Density(d.density,scale)) {SigilTheme(Appearance(),palette=NativeCore::palette) {Box(Modifier.width(400.dp)) {MessageCards(message,{""},null)}}}}
        for (s in listOf(1f,1.3f)) {
            scale=s; ui.waitForIdle()
            val density=ui.density.density
            val card=bounds(hasContentDescription("Quote"))
            val text=bounds(hasText("Simple is better."),true)
            val pixels=ui.onRoot().captureToImage().toPixelMap()
            val ground=pixels[card.left.toInt()+1,card.top.toInt()+1]
            fun inkTop(from:Float,to:Float)=(0 until pixels.height).first {y->(from.toInt() until to.toInt()).any {x->pixels[x,y].let {abs(it.red-ground.red)+abs(it.alpha-ground.alpha)>.2f}}}
            val mark=inkTop(card.left,text.left-2f)
            val cap=inkTop(text.left,text.right)
            assertTrue(mark>=card.top-1f,"At $s the mark stays inside the card's padding")
            assertTrue(abs(mark-cap)<=3*density,"At $s the mark's top meets the first line's cap line ($mark vs $cap)")
        }
    }

    @Test fun a_quote_without_an_author_still_attributes_its_source() {
        utility(UtilityContent("quote",rich=RichText("Less."),details=listOf(RichText("Volume 01"))))
        ui.onNode(hasContentDescription("— Volume 01"),true).assertExists()
    }

    @Test fun a_shortcut_sets_deep_keycaps_with_modifier_glyphs_and_speaks_the_names() {
        utility(UtilityContent("keys",details=listOf(RichText("cmd"),RichText("Shift"),RichText("p"))))
        ui.onNodeWithText("Keyboard shortcut",substring=false).assertDoesNotExist()
        ui.onNode(hasContentDescription("Keyboard shortcut. Command plus Shift plus P")).assertExists()
        assertEquals(2,ui.onAllNodes(hasText("+"),true).fetchSemanticsNodes().size,"Every gap between keys carries one joiner")
        val first=bounds(hasText("Cmd"),true)
        val last=bounds(hasText("P"),true)
        assertTrue(first.right<=last.left,"The keys run left to right on one line")
        assertTrue(last.top<first.bottom,"A shortcut is one row, not a stack")
    }

    @Test fun a_long_quote_collapses_behind_show_more() {
        utility(UtilityContent("quote",rich=RichText(List(40){"words that run on"}.joinToString(" ")),secondary=RichText("Sam")))
        ui.onNodeWithText("Show more").assertHasClickAction().performClick()
        ui.onNodeWithText("Show less").assertExists()
    }

    @Test fun a_short_quote_offers_no_show_more() {
        utility(UtilityContent("quote",rich=RichText("Short."),secondary=RichText("Sam")))
        ui.onNodeWithText("Show more").assertDoesNotExist()
    }

    @Test fun a_shortcut_inside_a_sentence_stays_in_the_line() {
        render(message(MessagePart("a","text","Press ",rich=RichText("Press ")),MessagePart("k","utility","",utility=UtilityContent("keys",details=listOf(RichText("Ctrl"),RichText("P")))),
            MessagePart("b","text"," to find it.",rich=RichText(" to find it."))))
        val press=bounds(hasText("Press"),true)
        val ctrl=bounds(hasText("Ctrl"),true)
        val to=bounds(hasText("to"),true)
        assertTrue(press.right<=ctrl.left && ctrl.right<=to.left,"Words and keycaps run left to right")
        assertTrue(ctrl.top<press.bottom && to.top<ctrl.bottom,"Text, keys and text share one line, not three blocks")
        assertTrue(ctrl.height<40f,"An inline keycap is compact")
        ui.onNode(hasContentDescription("Control plus P")).assertExists()
        assertEquals(1,keyFlow(listOf(MessagePart("k","utility","",utility=UtilityContent("keys",details=listOf(RichText("Esc")))),MessagePart("t","text",".",rich=RichText(".")))).size,"Closing punctuation stays on the key")
        val paren=keyFlow(listOf(MessagePart("a","text","Save (",rich=RichText("Save (")),MessagePart("k","utility","",utility=UtilityContent("keys",details=listOf(RichText("S")))),MessagePart("b","text",")",rich=RichText(")"))))
        assertEquals(listOf(1,3),paren.map { it.size },"Brackets hug the shortcut they enclose")
    }

    @Test fun menu_copy_gives_the_value_not_the_source() {
        fun one(value:UtilityContent)=shareCopy(message(MessagePart("u","utility","",utility=value)))
        assertEquals("#FF5733",one(UtilityContent("swatch",rgba=0xff5733ffL)))
        assertEquals("#6E84D280",one(UtilityContent("swatch",rgba=0x6e84d280L)))
        assertEquals("Ctrl+Shift+P",one(UtilityContent("keys",details=listOf(RichText("Ctrl"),RichText("Shift"),RichText("P")))))
        assertEquals("Less.\n— Sam, Notes",one(UtilityContent("quote",rich=RichText("Less."),secondary=RichText("Sam"),details=listOf(RichText("Notes")))))
        assertEquals("Press Ctrl+P.",shareCopy(message(MessagePart("a","text","Press",rich=RichText("Press")),MessagePart("k","utility","",utility=UtilityContent("keys",details=listOf(RichText("Ctrl"),RichText("P")))),MessagePart("b","text",".",rich=RichText(".")))))
        assertNull(shareCopy(message(MessagePart("t","text","Hello",rich=RichText("Hello")))),"Ordinary text keeps the message copy")
    }

    @Test fun key_faces_name_modifiers_for_every_platform() {
        assertNotEquals(keyFace("Backspace").glyph,keyFace("Left").glyph,"Backspace is not drawn as a left arrow")
        assertEquals(KeyFace(null,"Ctrl","Control"),keyFace("ctrl"),"A PC Ctrl key prints only its name")
        assertEquals("keyboard_control_key",keyFace("ctrl",mac=true).glyph,"A Mac combination carries the ⌃ legend")
        assertTrue(macCombo(listOf("Cmd","Ctrl","Q")))
        assertEquals("Keyboard shortcut. Control plus Shift plus P",keysSpoken(listOf("Ctrl","Shift","P")))
        assertEquals(KeyFace("keyboard_option_key","Option","Option"),keyFace("opt"))
        assertEquals(KeyFace(null,"Esc","Escape"),keyFace("Escape"))
        assertEquals(KeyFace("arrow_upward","","Up arrow"),keyFace("Up"))
        assertEquals("K",keyFace(" k ").word)
        assertEquals("F12",keyFace("F12").word)
    }

    @Test fun a_swatch_is_an_opaque_rounded_rectangle_over_its_value() {
        utility(UtilityContent("swatch",display="#6e84d280",copy="#6e84d280",rgba=0x6e84d280L))
        val card=ui.onNode(hasContentDescription("Color #6E84D2, blue, 50% opacity"))
        card.assertExists()
        val tile=card.fetchSemanticsNode().boundsInRoot
        assertTrue(tile.width>=200f && tile.height>=120f,"One swatch is a generous sample, not a text-height chip")
        assertEquals("#FF5733",swatchHex(0xff5733ffL))
        assertNull(swatchOpacity(0xff5733ffL),"An opaque colour says nothing about opacity")
        assertEquals(50,swatchOpacity(0x6e84d280L))
        assertEquals(1f,swatchColor(0x6e84d280L).alpha,"The sample is drawn opaque, never see-through")
        assertEquals("white",swatchName(0xffffffffL))
        assertEquals("orange",swatchName(0xff5733ffL))
    }

    @Test fun consecutive_swatches_form_one_palette_row() {
        val sw={c:Long->MessagePart("s$c","utility","",utility=UtilityContent("swatch",rgba=c))}
        render(message(sw(0xff5733ffL),MessagePart("t","text"," "),sw(0x2e7d32ffL),sw(0x6e84d2ffL)))
        val a=bounds(hasContentDescription("Color #FF5733",substring=true))
        val b=bounds(hasContentDescription("Color #2E7D32",substring=true))
        val c=bounds(hasContentDescription("Color #6E84D2",substring=true))
        assertEquals(a.top,b.top,"Swatches sit side by side")
        assertEquals(b.top,c.top)
        assertTrue(a.right<b.left && b.right<c.left)
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
