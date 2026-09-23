package org.sigil

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class TableProgressCardTest {
    @get:Rule val ui=createComposeRule()

    private fun message(vararg parts:MessagePart)=ChatMessage("card","author","",false,"9:41","read",false,emptyList(),emptyList(),null,true,peer="conversation",parts=parts.toList())
    private fun render(message:ChatMessage,width:Int=400) {ui.setContent {MaterialTheme {Box(Modifier.width(width.dp)) {MessageCards(message,{""},null)}}}}

    @Test fun slack_goes_to_text_columns_and_figures_hug_the_end() {
        val widths=tableWidths(listOf(40f,20f),listOf(40f,20f),200f,10f,listOf(false,true))
        assertEquals(listOf(170f,20f),widths)
    }

    @Test fun a_narrow_card_drops_trailing_columns_instead_of_scrolling() {
        val widths=tableWidths(List(6) {100f},List(6) {56f},300f,12f)
        assertEquals(4,widths.size)
        assertTrue(widths.sum()+12f*3<=300.5f)
    }

    @Test fun long_columns_give_way_before_short_ones() {
        val widths=tableWidths(listOf(400f,60f),listOf(80f,60f),300f,12f)
        assertEquals(60f,widths[1])
        assertEquals(228f,widths[0],.5f)
    }

    @Test fun a_table_counts_what_it_cannot_show_and_prints_no_type_word() {
        val rows=List(7) {listOf(RichText("Task $it"),RichText("$it"))}
        val table=TableContent(listOf(RichText("Task"),RichText("Days")),rows,listOf(null,rows.indices.toList()),List(7) {null},null)
        render(message(MessagePart("t","table","",table=table)))
        ui.onNodeWithText("5 of 7 rows").assertExists()
        ui.onNodeWithText("Task 5").assertDoesNotExist()
        ui.onNodeWithText("Table").assertDoesNotExist()
    }

    @Test fun a_short_row_shows_its_missing_cell_silently() {
        val table=TableContent(listOf(RichText("Item"),RichText("Owner"),RichText("Status")),listOf(listOf(RichText("Prototype"),RichText("Ari"))),listOf(null,null,null),listOf(null),null)
        render(message(MessagePart("t","table","",table=table)))
        ui.onNodeWithText("Status").assertExists()
        ui.onNodeWithText("—").assertDoesNotExist()
    }

    @Test fun progress_leads_with_its_title_and_figure_and_reports_its_range() {
        val progress=UtilityContent("progress",display="75%",ratio=.75f,rich=RichText("Sprint burndown"))
        render(message(MessagePart("p","utility","",utility=progress)))
        val node=ui.onNodeWithContentDescription("Progress. Sprint burndown. 75%").fetchSemanticsNode()
        assertEquals(.75f,node.config[SemanticsProperties.ProgressBarRangeInfo].current)
        ui.onNodeWithText("Progress").assertDoesNotExist()
    }

    @Test fun a_narrow_table_counts_its_fitted_columns_and_ends_flush() {
        val columns=List(6) {RichText("Column $it")}
        val table=TableContent(columns,listOf(List(6) {RichText("Value $it")}),List(6) {null},listOf(null),null)
        render(message(MessagePart("t","table","",table=table)),240)
        ui.onNodeWithText("of 6 columns",substring=true).assertExists()
        val card=ui.onNodeWithContentDescription("Table, 1 row, 6 columns").fetchSemanticsNode().boundsInRoot
        val foot=ui.onNodeWithText("of 6 columns",substring=true).fetchSemanticsNode().boundsInRoot
        assertEquals(card.bottom,foot.bottom,1f)
    }
}
