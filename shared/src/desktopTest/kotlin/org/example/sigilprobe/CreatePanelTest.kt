package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class CreatePanelTest {
    @get:Rule val ui=createComposeRule()
    @Test fun panel_height_tracks_actual_rows_instead_of_reserving_empty_space() {
        var preferred=0.dp
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalComposerPanelHeight provides {value:Dp->preferred=value}) {
            Box(Modifier.width(400.dp).height(600.dp)) {CreatePanel({},{})}
        }}}
        ui.waitForIdle()
        val first=preferred
        assertTrue(first>250.dp && first<380.dp)
        val timer=ui.onNodeWithContentDescription("Timer").fetchSemanticsNode().boundsInRoot
        val dots=ui.onNodeWithContentDescription("Create page 1").fetchSemanticsNode().boundsInRoot
        assertTrue(dots.top-timer.bottom<20f)
        ui.onNodeWithContentDescription("Create page 3").performClick()
        ui.waitForIdle()
        assertTrue(preferred<first)
        ui.onNodeWithContentDescription("Weather").assertIsDisplayed()
    }
    @Test fun paged_tools_keep_frequent_creators_first_and_support_swiping() {
        val opened=mutableListOf<String>()
        var closed=0
        ui.setContent {MaterialTheme {Box(Modifier.width(400.dp).height(600.dp)) {CreatePanel({closed++},opened::add)}}}
        ui.onNodeWithText("Search tools").assertDoesNotExist()
        ui.onNodeWithContentDescription("Note").assertExists()
        listOf("Contact","Poll","Checklist","Recipe","Dice","Coin","Cards","Random Number","Note","Task","Reminder","Timer").forEach {ui.onNodeWithContentDescription(it).assertIsDisplayed()}
        val firstRow=listOf("Contact","Poll","Checklist","Recipe").map {ui.onNodeWithContentDescription(it).fetchSemanticsNode().boundsInRoot}
        assertTrue(firstRow.all {it.top==firstRow.first().top})
        assertTrue(firstRow.zipWithNext().all {(left,right)->left.right<=right.left})
        ui.onNodeWithContentDescription("Randomizer").assertDoesNotExist()
        ui.onNodeWithContentDescription("Poll").assertExists()
        ui.onNodeWithContentDescription("Dice").performClick()
        ui.onNodeWithTag("create-pages").performTouchInput {swipeLeft()}
        ui.onNodeWithContentDescription("Create page 2").assertIsSelected()
        ui.onNodeWithContentDescription("Note").assertIsNotDisplayed()
        ui.onNodeWithContentDescription("Table").performClick()
        ui.onNodeWithContentDescription("Help").performClick()
        assertEquals(listOf("Dice","Table","Help"),opened)
        ui.onNodeWithContentDescription("Back to attachments").performClick()
        assertEquals(1,closed)
    }
}
