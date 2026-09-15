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

class BuilderSizingTest {
    @get:Rule val ui=createComposeRule()
    @Test fun coin_reports_natural_height_independent_of_scroll_viewport() {
        var cap by mutableStateOf(420.dp)
        var natural=0.dp
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalBuilderSource provides NativeCore::builderSource,
            LocalComposerConfirmation provides remember {ComposerConfirmation()},LocalComposerPanelHeight provides {natural=it}) {
            Box(Modifier.width(380.dp).height(cap)) {RandomizerBuilder(true,{},initialMode="Coin",send={})}
        }}}
        ui.waitForIdle()
        val full=natural
        println("Coin natural height: $full; viewport: $cap")
        assertTrue(full in 150.dp..320.dp,"Coin natural height: $full")
        ui.runOnIdle {cap=120.dp}
        ui.waitForIdle()
        assertEquals(full,natural)
        ui.onNodeWithText("Heads or tails").performScrollTo().assertIsDisplayed()
    }
    @Test fun growing_poll_keeps_all_fields_reachable_inside_a_small_viewport() {
        var natural=0.dp
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalComposerConfirmation provides remember {ComposerConfirmation()},
            LocalComposerPanelHeight provides {natural=it}) {Box(Modifier.width(380.dp).height(240.dp)) {
            StructuredBuilder("Poll",true,{},{_,_->})
        }}}}
        repeat(6) {index->ui.onNodeWithText("Option ${index+1}").performScrollTo().performTextInput("Choice ${index+1}")}
        ui.onNodeWithText("Option 7").performScrollTo().assertIsDisplayed()
        println("Poll natural height: $natural; viewport: 240.dp")
        assertTrue(natural>600.dp)
        ui.onNodeWithText("Question").performScrollTo().assertIsDisplayed()
    }
}
