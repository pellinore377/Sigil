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

class DataCardTest {
    @get:Rule val ui=createComposeRule()

    private fun message(vararg parts:MessagePart)=ChatMessage("card","author","",false,"9:41","read",false,emptyList(),emptyList(),null,true,peer="conversation",parts=parts.toList())
    private fun render(message:ChatMessage) {ui.setContent {MaterialTheme {Box(Modifier.width(400.dp)) {MessageCards(message,{""},null)}}}}
    private fun bounds(matcher:SemanticsMatcher,unmerged:Boolean=false)=ui.onNode(matcher,unmerged).fetchSemanticsNode().boundsInRoot

    @Test fun poll_figures_lead_the_option_instead_of_trailing_it() {
        val items=listOf(CardItem("a","Making",false,true,count=2L),CardItem("b","Waiting",false,true,count=4L))
        render(message(MessagePart("p","poll","Which one",voters=6L,closed=true,items=items)))
        val share=bounds(hasText("33%"),true)
        val label=bounds(hasText("Making"),true)
        assertTrue(share.right<=label.left,"The vote figures belong on the leading edge, not past the label")
        assertEquals(share.left,bounds(hasText("67%"),true).left,"Every option's figures share one left edge")
    }

    @Test fun a_rating_draws_stars_and_lets_the_bubble_close_around_them() {
        render(message(MessagePart("r","utility","",utility=UtilityContent("rating",display="4/5",copy="4/5",ratio=.8f))))
        val stars=bounds(hasContentDescription("4/5"))
        assertTrue(stars.width>0f,"The stars carry the rating as one described row")
        assertTrue(bounds(hasContentDescription("Copy rating")).right<stars.right+64f,"The header must wrap its content, not stretch the bubble")
    }

    @Test fun a_calculation_reads_as_one_line_with_an_equals_sign() {
        render(message(MessagePart("c","utility","",utility=UtilityContent("calculation",display="48",rich=RichText("12 * 4")))))
        val expression=bounds(hasText("12 * 4"))
        val equals=bounds(hasText("="))
        val result=bounds(hasText("48"))
        assertTrue(expression.right<=equals.left && equals.right<=result.left,"Expression, equals and result run left to right")
        assertTrue(result.top<expression.bottom,"The result sits beside the expression, not under it")
    }

    @Test fun a_pie_legend_keeps_each_value_next_to_its_own_category() {
        val chart=ChartContent("pie",RichText("Split"),false,0f,emptyList(),emptyList(),null,
            listOf(ChartPoint(RichText("Making"),.25f,.25f,"42",null,.33f,"33"),ChartPoint(RichText("Waiting"),.75f,.75f,"84",null,.67f,"67")))
        render(message(MessagePart("h","chart","",chart=chart)))
        val label=bounds(hasText("Making"))
        val value=bounds(hasText("42 · 33%"))
        assertTrue(value.left>=label.right && value.left-label.right<24f,"The value rides beside its label")
        assertTrue(value.top<label.bottom,"Legend rows are one line, not a stacked ordinal")
        ui.onNodeWithText("33% of total").assertDoesNotExist()
    }

    @Test fun previews_offer_no_route_into_an_expanded_view() {
        val table=TableContent(listOf(RichText("Item"),RichText("Count")),listOf(listOf(RichText("Bolts"),RichText("12"))),listOf(null,listOf(0)),listOf(null),null)
        render(message(MessagePart("t","table","",table=table)))
        ui.onNodeWithText("Open table").assertDoesNotExist()
        ui.onNodeWithContentDescription("Row 1, column 1").assertHasNoClickAction()
    }

    @Test fun a_recipe_preview_carries_no_open_button() {
        val recipe=RecipeContent(RichText("Soup"),4,4,1500L,listOf(RichText("Stock")),listOf(false),listOf(RichText("Simmer")))
        render(message(MessagePart("r","recipe","",recipe=recipe)))
        ui.onNodeWithText("Open recipe").assertDoesNotExist()
        ui.onNodeWithText("1 ingredients · 1 steps").assertHasNoClickAction()
    }

    @Test fun only_chart_diagram_table_qr_and_recipe_reach_a_details_view() {
        val chart=ChartContent("bar",RichText("Split"),false,0f,emptyList(),emptyList(),null,listOf(ChartPoint(RichText("A"),0f,1f,"1",null,1f,"100")))
        val table=TableContent(listOf(RichText("Item")),listOf(listOf(RichText("Bolts"))),listOf(null),listOf(null),null)
        val diagram=DiagramContent("flow",RichText("Flow"),200f,120f,listOf(DiagramNode(RichText("Start"),"process",0f,0f)),emptyList(),emptyList())
        val recipe=RecipeContent(RichText("Soup"),null,null,null,emptyList(),emptyList(),emptyList())
        val qr=UtilityContent("qr",qr=QrContent("text",1,"1","hello"))
        for (part in listOf(MessagePart("a","chart","",chart=chart),MessagePart("b","table","",table=table),
            MessagePart("c","diagram","",diagram=diagram),MessagePart("d","recipe","",recipe=recipe),MessagePart("e","utility","",utility=qr)))
            assertNotNull(message(part).detailsPart(),"${part.kind} keeps a details view")
        for (part in listOf(MessagePart("f","text","hello"),MessagePart("g","poll","Which"),MessagePart("h","note","Remember"),
            MessagePart("i","utility","",utility=UtilityContent("quote",rich=RichText("Simple."))),
            MessagePart("j","utility","",utility=UtilityContent("progress",display="75%",ratio=.75f))))
            assertNull(message(part).detailsPart(),"${part.kind} has no details view")
    }
}
