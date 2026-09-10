package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.unit.dp
import org.junit.Test
import kotlin.test.*

class TableBuilderTest {
    @OptIn(ExperimentalTestApi::class)
    @Test fun table_columns_rows_and_literal_preview_survive_restoration_before_explicit_send() = runComposeUiTest {
        val ui=this
        val sent=mutableListOf<String>()
        var registry by mutableStateOf(SaveableStateRegistry(null){true})
        var visible by mutableStateOf(true)
        ui.setContent {if(visible) {MaterialTheme {CompositionLocalProvider(LocalSaveableStateRegistry provides registry,LocalBuilderSource provides NativeCore::builderSource) {
            Box(Modifier.width(380.dp).height(700.dp)) {TableBuilder(true,{},sent::add)}
        }}}}
        ui.onNodeWithText("Enter rows").assertIsNotEnabled()
        ui.onNodeWithText("Column 1").performTextInput("Name")
        ui.onNodeWithText("Column 2").performTextInput("Count")
        ui.onNodeWithText("Enter rows").performClick()
        ui.onNodeWithText("Name").performTextInput("Fish |\nchips")
        ui.onNodeWithText("Count").performTextInput("2")
        ui.onNodeWithText("Add row").performScrollTo().performClick()
        ui.onNodeWithText("Name").performScrollTo().performTextInput("redact::literal;")
        ui.onNodeWithText("Count").performTextInput("10")
        ui.onNodeWithText("Name").performTextReplacement("x".repeat(17000))
        ui.onNodeWithText("Name").assertTextContains("redact::literal;")
        ui.onNodeWithText("That edit is too large. Shorten the text and try again.").assertExists()
        var saved:Map<String,List<Any?>> = emptyMap()
        ui.runOnIdle {saved=registry.performSave();visible=false}
        ui.waitForIdle()
        ui.runOnIdle {registry=SaveableStateRegistry(saved){true};visible=true}
        ui.onNodeWithText("Name").assertTextContains("redact::literal;")
        ui.onNodeWithContentDescription("Previous row").performClick()
        ui.onNodeWithText("Name").assertTextContains("Fish | chips")
        ui.onNodeWithText("Columns").performClick()
        ui.onNodeWithContentDescription("Remove column 2").performClick()
        ui.onNodeWithText("Keep column").performClick()
        ui.onNodeWithText("Column 2").assertTextContains("Count")
        ui.onNodeWithContentDescription("Remove column 2").performClick()
        ui.onNodeWithText("Remove",useUnmergedTree=true).performClick()
        ui.onNodeWithText("Column 2").assertDoesNotExist()
        assertTrue(sent.isEmpty())
        ui.onNodeWithText("Preview").performClick()
        ui.onNodeWithText("Fish | chips").assertExists()
        ui.onNodeWithText("redact::literal;").assertExists()
        ui.onNodeWithText("Send table").performScrollTo().performClick()
        assertEquals(listOf(NativeCore.builderSource("Table\nName\nFish | chips\nredact::literal;")),sent)
    }
}
