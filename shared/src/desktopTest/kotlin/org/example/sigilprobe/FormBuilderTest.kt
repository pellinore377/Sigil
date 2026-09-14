package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class FormBuilderTest {
    @get:Rule val ui=createComposeRule()
    @Test fun recipe_entries_are_literal_and_require_no_source_syntax() {
        val sent=mutableListOf<String>()
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalBuilderSource provides NativeCore::builderSource) {Box(Modifier.width(420.dp).height(760.dp)) {FormBuilder("Recipe",true,{}){source,_->sent+=source}}}}}
        ui.onNodeWithText("Title").performTextInput("Dinner")
        ui.onNodeWithText("Servings (optional)").performTextInput("4")
        ui.onNodeWithText("Ingredient 1").performScrollTo().performTextInput("redact::literal;")
        ui.onNodeWithText("Ingredient 2").assertExists()
        ui.onNodeWithText("Step 1").performScrollTo().performTextInput("Stir")
        ui.waitUntil(5000){ui.onAllNodes(isEnabled() and hasText("Send")).fetchSemanticsNodes().size==1}
        ui.onNodeWithText("Send").performClick()
        assertEquals(1,sent.size)
        assertTrue(NativeCore.structuredPreview(sent.single()).contains("redact::literal;"))
        assertFalse(sent.single().contains("\n- redact::literal;"))
    }
    @Test fun invalid_chart_edits_cannot_send_the_previously_valid_result() {
        val sent=mutableListOf<String>()
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalBuilderSource provides NativeCore::builderSource) {Box(Modifier.width(420.dp).height(760.dp)) {FormBuilder("Chart",true,{}){source,_->sent+=source}}}}}
        ui.onNodeWithText("Title").performTextInput("Values")
        ui.onAllNodesWithText("Label / X")[0].performTextInput("A")
        ui.onAllNodesWithText("Value / Y")[0].performTextInput("2")
        ui.waitUntil(5000){ui.onAllNodes(isEnabled() and hasText("Send")).fetchSemanticsNodes().size==1}
        ui.onAllNodesWithText("Value / Y")[0].performTextReplacement("NaN")
        ui.onNodeWithText("Send").assertIsNotEnabled()
        assertTrue(sent.isEmpty())
    }
}
