package org.sigil

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class DataCardTest {
    @get:Rule val ui=createComposeRule()

    private fun message(vararg parts:MessagePart)=ChatMessage("card","author","",false,"9:41","read",false,emptyList(),emptyList(),null,true,peer="conversation",parts=parts.toList())
    private fun render(message:ChatMessage) {ui.setContent {MaterialTheme {Box(Modifier.width(400.dp)) {MessageCards(message,{""},null)}}}}
    private fun bounds(matcher:SemanticsMatcher,unmerged:Boolean=false)=ui.onNode(matcher,unmerged).fetchSemanticsNode().boundsInRoot

    @Test fun a_poll_leads_with_its_question_and_reads_each_result_as_state() {
        val items=listOf(CardItem("a","Making",false,true,count=2L),CardItem("b","Waiting",true,true,count=4L))
        render(message(MessagePart("p","poll","Which one",voters=6L,closed=true,items=items)))
        ui.onNodeWithText("Poll").assertDoesNotExist()
        ui.onNode(hasContentDescription("Poll. Which one")).assertExists()
        ui.onNode(hasStateDescription("33%, 2 of 6 votes")).assertExists()
        ui.onNode(hasStateDescription("67%, 4 of 6 votes, most votes")).assertIsSelected()
        ui.onNodeWithText("Voting closed · 6 votes").assertExists()
    }

    @Test fun poll_foot_says_when_results_wait_for_a_vote() {
        val items=listOf(CardItem("a","Making",false,true),CardItem("b","Waiting",false,true))
        assertEquals("Results after you vote",pollFoot(MessagePart("p","poll","Which one",items=items)))
        assertEquals("Multiple choice · No votes yet",pollFoot(MessagePart("p","poll","Which one",multiple=true,voters=0L,items=items)))
        assertEquals("Multiple choice · 1 voter",pollFoot(MessagePart("p","poll","Which one",multiple=true,voters=1L,items=items)))
    }

    @Test fun a_rating_draws_one_star_per_point_and_reads_as_stars() {
        render(message(MessagePart("r","utility","",utility=UtilityContent("rating",display="4/5",copy="4/5",ratio=.8f))))
        val stars=bounds(hasContentDescription("Rating. 4 out of 5 stars"))
        assertTrue(stars.width>0f,"The stars carry the rating as one described row")
        ui.onNodeWithContentDescription("Copy rating").assertDoesNotExist()
        assertEquals(RatingScale("8","10",10,8f),ratingScale("8/10",.8f))
        assertEquals(3.5f,ratingScale("3.5/5",.7f).filled)
        assertEquals(5,ratingScale("70/100",.7f).stars)
        assertEquals("70 out of 100",ratingSpoken("70/100",ratingScale("70/100",.7f)))
        assertEquals("3 out of 2.5",ratingSpoken("3/2.5",ratingScale("3/2.5",1f)))
        assertEquals("3.5 out of 5 stars",ratingSpoken("3.5/5",ratingScale("3.5/5",.7f)))
    }

    @Test fun a_diagram_leads_with_its_title_and_reads_its_relationships() {
        val nodes=listOf(DiagramNode(RichText("Lead"),"default",232f,24f),DiagramNode(RichText("Design"),"default",128f,168f),DiagramNode(RichText("Build"),"default",336f,168f))
        val edges=listOf(DiagramEdge(0,1,RichText(""),false,144f),DiagramEdge(0,2,RichText(""),false,240f))
        render(message(MessagePart("d","diagram","",diagram=DiagramContent("org",RichText("Team"),600f,264f,nodes,edges,emptyList()))))
        ui.onNodeWithText("Diagram").assertDoesNotExist()
        ui.onNode(hasContentDescription("Org chart. Team. Lead: Design, Build")).assertExists()
        val states=DiagramContent("state",RichText("Call"),400f,400f,listOf(DiagramNode(RichText("Idle"),"default",24f,24f),DiagramNode(RichText("Ringing"),"default",24f,168f)),
            listOf(DiagramEdge(0,1,RichText("invite"),false,144f),DiagramEdge(1,0,RichText("decline"),false,240f)),emptyList())
        assertEquals("State diagram. Call. Idle to Ringing, invite. Ringing to Idle, decline",states.summary())
    }

    private fun node(label:String,x:Float,row:Int)=DiagramNode(RichText(label),"default",x,24f+144f*row)
    private fun edge(from:Int,to:Int,label:String="")=DiagramEdge(from,to,RichText(label),false,0f)
    private fun geometry(kind:String,nodes:List<DiagramNode>,edges:List<DiagramEdge>):LayerGeometry? {
        val diagram=DiagramContent(kind,RichText(""),0f,0f,nodes,edges,emptyList())
        val plan=assertNotNull(diagram.layerPlan(372.dp) {48.dp})
        val chips=edges.indices.filter {edges[it].label.text.isNotBlank()}.associateWith {IntSize(60,24)}
        return Density(1f).layerGeometry(diagram,plan,372,nodes.map {IntSize(120,44)},chips)
    }

    @Test fun labels_on_edges_merging_into_one_state_ride_their_sources_and_never_overlap() {
        val g=assertNotNull(geometry("state",listOf(node("Idle",24f,0),node("Busy",232f,0),node("Ringing",128f,1)),listOf(edge(0,2,"invite"),edge(1,2,"retry"))))
        val (a,b)=g.chips.getValue(0) to g.chips.getValue(1)
        assertFalse(a.overlaps(b),"Two labels into one state stay apart")
        assertEquals(g.tiles[0].center.x,a.center.x,1f); assertEquals(g.tiles[1].center.x,b.center.x,1f)
        assertTrue(a.top>=g.tiles[0].bottom && a.bottom<=g.strokes[0].points[1].y,"Each label sits on its source's own drop, above the shared bus")
        assertEquals(setOf(2),g.strokes[0].sharp,"The merge into Ringing is a square tee")
    }

    @Test fun a_fan_out_meets_its_bus_in_a_square_tee() {
        val g=assertNotNull(geometry("org",listOf(node("Lead",128f,0),node("Design",24f,1),node("Build",232f,1)),listOf(edge(0,1),edge(0,2))))
        assertTrue(g.strokes.all {it.sharp==setOf(1)},"Only the corner above each child is rounded")
    }

    @Test fun interleaved_families_and_shared_label_segments_fall_back() {
        val nodes=listOf(node("A",24f,0),node("B",232f,0),node("C",24f,1),node("D",232f,1))
        assertNull(geometry("org",nodes,listOf(edge(0,3),edge(1,2))),"Crossing buses cannot say who owns which child")
        assertNull(geometry("state",listOf(node("Idle",24f,0),node("Ringing",24f,1),node("Busy",232f,1)),listOf(edge(0,1,"invite"),edge(0,1,"retry"),edge(0,2))),"Two labels on one shared segment")
        val wide=DiagramContent("state",RichText(""),0f,0f,(0..5).map {node("S$it",24f+208f*(it%4),it/4)},(0..11).map {edge(it%6,(it+1)%6,"go")},emptyList())
        assertNull(wide.layerPlan(372.dp) {24.dp},"A crowded state machine reads as its list of transitions")
    }

    @Test fun a_calculation_leads_with_its_expression_and_lands_on_the_result() {
        render(message(MessagePart("c","utility","",utility=UtilityContent("calculation",display="-12345.5",copy="-12345.5",rich=RichText("12 * 4")))))
        ui.onNodeWithContentDescription("Calculation. 12 times 4 equals minus 12,345.5").assertExists()
        ui.onNodeWithText("Calculation").assertDoesNotExist()
        ui.onNodeWithContentDescription("Copy calculation").assertDoesNotExist()
    }

    @Test fun a_conversion_rounds_for_the_glance_and_names_both_quantities() {
        render(message(MessagePart("v","utility","",utility=UtilityContent("conversion",display="5 mi",alternate="8.0467 km",copy="8.04672 km"))))
        ui.onNodeWithContentDescription("Conversion. 5\u00A0mi is 8.05\u00A0km").assertExists()
        assertEquals("0.0026",glanceNumber("0.00264"))
        assertEquals("1,234",readableNumber("1234"))
        assertEquals("0",readableNumber("-0"))
    }

    @Test fun a_pie_legend_keeps_each_value_next_to_its_own_category() {
        val chart=ChartContent("pie",RichText("Split"),false,0f,emptyList(),emptyList(),null,
            listOf(ChartPoint(RichText("Making"),.25f,.25f,"42",null,.33f,"33"),ChartPoint(RichText("Waiting"),.75f,.75f,"84",null,.67f,"67")))
        render(message(MessagePart("h","chart","",chart=chart)))
        val label=bounds(hasText("Making"),true)
        val value=bounds(hasText("42"),true)
        assertTrue(value.left>=label.right,"The value trails its label")
        assertTrue(value.top<label.bottom,"Legend rows are one line, not a stacked ordinal")
        ui.onNodeWithText("Making").performClick()
        ui.onNodeWithText("42 · 33%").assertExists()
        ui.onNodeWithText("33% of total").assertDoesNotExist()
    }

    @Test fun previews_offer_no_route_into_an_expanded_view() {
        val table=TableContent(listOf(RichText("Item"),RichText("Count")),listOf(listOf(RichText("Bolts"),RichText("12"))),listOf(null,listOf(0)),listOf(null),null)
        render(message(MessagePart("t","table","",table=table)))
        ui.onNodeWithText("Open table").assertDoesNotExist()
        ui.onNodeWithText("Bolts").assertHasNoClickAction()
        ui.onNodeWithText("Table").assertDoesNotExist()
    }

    @Test fun a_recipe_preview_carries_no_open_button() {
        val recipe=RecipeContent(RichText("Soup"),4,4,1500L,listOf(RichText("Stock")),listOf(false),listOf(RichText("Simmer")))
        render(message(MessagePart("r","recipe","",recipe=recipe)))
        ui.onNodeWithText("Open recipe").assertDoesNotExist()
        ui.onNodeWithText("4 servings · 25 min").assertHasNoClickAction()
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
