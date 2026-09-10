package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class CreatePanelTest {
    @get:Rule val ui=createComposeRule()
    @Test fun categories_drill_down_and_search_finds_tools_across_categories_by_intent() {
        val opened=mutableListOf<String>()
        var closed=0
        ui.setContent {MaterialTheme {Box(Modifier.width(400.dp).height(600.dp)) {CreatePanel({closed++},opened::add)}}}
        ui.onNodeWithContentDescription("Plan & Organize").assertExists()
        ui.onNodeWithContentDescription("Poll").assertDoesNotExist()
        ui.onNodeWithContentDescription("Ask & Decide").performClick()
        ui.onNodeWithContentDescription("Poll").assertExists()
        ui.onNodeWithContentDescription("Note").assertDoesNotExist()
        ui.onNodeWithText("Search tools").performTextInput("spreadsheet")
        ui.onNodeWithContentDescription("Table").performClick()
        assertEquals(listOf("Table"),opened)
        ui.onNodeWithContentDescription("Clear tool search").performClick()
        ui.onNodeWithContentDescription("Poll").assertExists()
        ui.onNodeWithContentDescription("Back to categories").performClick()
        ui.onNodeWithText("Search tools").performTextInput("COIN flip")
        ui.onNodeWithContentDescription("Randomizer").assertExists()
        ui.onNodeWithContentDescription("Poll").assertDoesNotExist()
        ui.onNodeWithText("Search tools").performTextReplacement("unavailable-tool")
        ui.onNodeWithText("No matching tools. Use Help to explore SigilText syntax.").assertExists()
        ui.onNodeWithContentDescription("Help").performClick()
        assertEquals(listOf("Table","Help"),opened)
        ui.onNodeWithContentDescription("Clear search").performClick()
        ui.onNodeWithContentDescription("Back to attachments").performClick()
        assertEquals(1,closed)
    }
}
