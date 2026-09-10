package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*

class TableTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    private fun show(table: TableContent) {
        val chat = ChatSummary("self", "@sam:example.test", "", "", true, emptyList())
        val message = ChatMessage("table", "sam", "Table", true, "9:33", "sent", false, emptyList(), emptyList(), null, true,
            timestamp = 1000, parts = listOf(MessagePart("card", "table", "Table", table = table)))
        ui.runOnUiThread { ui.activity.setSigilContent { SigilApp(NativeCore::palette, NativeCore::analyze,
            MessengerState(phase = "connected", chats = listOf(chat), selected = "self", messages = listOf(message)), { _, _ -> }) } }
    }
    @Test fun expanded_table_sorts_numerically_keeps_headers_and_copies_original_cells() {
        val numbers = listOf(10, 2) + (20..77).toList()
        val rows = numbers.mapIndexed { i, n -> listOf(RichText("Person ${i + 1}"), RichText(n.toString())) }
        val copies = rows.map { it.joinToString("\t") { cell -> cell.text } }
        show(TableContent(listOf(RichText("Name"), RichText("Count")), rows, listOf(null, numbers.indices.sortedBy { numbers[it] }), copies, "Name\tCount\n" + copies.joinToString("\n")))
        ui.onNodeWithText("Open table · 60 rows").performClick()
        val sort = ui.onNodeWithContentDescription("Sort column 2")
        val top = sort.fetchSemanticsNode().boundsInRoot.top
        sort.performClick().assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Ascending"))
        fun rowTop(n: Int) = ui.onNode(hasContentDescription("Row $n, column 1") and hasAnyAncestor(isDialog())).fetchSemanticsNode().boundsInRoot.top
        assertTrue(rowTop(2) < rowTop(1))
        ui.onNode(hasContentDescription("Row 2, column 1") and hasAnyAncestor(isDialog())).performClick()
        ui.onNodeWithText("Copy cell").performClick()
        val clipboard = ui.activity.getSystemService(android.content.ClipboardManager::class.java)
        ui.runOnIdle { assertEquals("Person 2", clipboard.primaryClip!!.getItemAt(0).text.toString()) }
        ui.onNodeWithText("Copy row").performClick()
        ui.runOnIdle { assertEquals("Person 2\t2", clipboard.primaryClip!!.getItemAt(0).text.toString()) }
        ui.onNodeWithText("Done").performClick()
        ui.onNode(hasScrollToIndexAction() and hasAnyAncestor(isDialog())).performScrollToIndex(40)
        assertEquals(top, sort.fetchSemanticsNode().boundsInRoot.top, .5f)
        sort.assertIsDisplayed()
        ui.onNodeWithContentDescription("Copy table").performClick()
        ui.runOnIdle { assertTrue(clipboard.primaryClip!!.getItemAt(0).text.toString().startsWith("Name\tCount\nPerson 1\t10")) }
        ui.onNodeWithContentDescription("Close table").performClick()
        ui.onNodeWithTag("composer").assertIsDisplayed()
    }
    @Test fun concealed_cells_do_not_leak_through_details_or_copy() {
        val hidden = RichText("Hidden number", listOf(RichSpan(0, 13, reveal = "spoiler")))
        show(TableContent(listOf(RichText("Private")), listOf(listOf(hidden)), listOf(null), listOf(null), null))
        ui.onNodeWithText("Open table · 1 row").performClick()
        ui.onNodeWithContentDescription("Copy table").assertIsNotEnabled()
        ui.onNodeWithText("Hidden number").assertDoesNotExist()
        ui.onNode(hasContentDescription("Row 1, column 1") and hasAnyAncestor(isDialog())).performSemanticsAction(SemanticsActions.OnLongClick) { it() }
        ui.onNodeWithText("Copy cell").assertIsNotEnabled()
        ui.onNodeWithText("Copy row").assertIsNotEnabled()
        ui.onNodeWithText("Hidden number").assertDoesNotExist()
    }
}
