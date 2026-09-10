package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class RandomizerBuilderTest {
    @get:Rule val ui=createComposeRule()
    @Test fun guided_fields_keep_each_modes_draft_and_only_send_valid_explicit_actions() {
        val sent=mutableListOf<String>()
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalRandomizerSource provides NativeCore::randomizerSource) {
            Box(Modifier.width(380.dp).height(700.dp)) {RandomizerBuilder(true,{},sent::add)}
        }}}
        ui.onNodeWithText("Count 1").performTextReplacement("3")
        ui.onNodeWithText("Sides 1").performTextReplacement("20")
        ui.onNodeWithText("Roll dice").performScrollTo().performClick()
        assertEquals(listOf("roll::3d20;"),sent)
        ui.onNodeWithText("Choice").performScrollTo().performClick()
        ui.onNodeWithText("Pick a choice").assertIsNotEnabled()
        ui.onNodeWithText("Choice 1").performTextInput("Fish,\nchips")
        ui.onNodeWithText("Choice 2").performTextInput("Fish, chips")
        ui.onNodeWithText("Pick a choice").assertIsNotEnabled()
        ui.onNodeWithText("Choice 2").performTextReplacement("redact::keep this;")
        ui.onNodeWithText("Pick a choice").performScrollTo().performClick()
        assertEquals(NativeCore.randomizerSource("Choice\nFish, chips\nredact::keep this;"),sent.last())
        ui.onNodeWithText("Dice").performScrollTo().performClick()
        ui.onNodeWithText("Count 1").assertTextContains("3")
        ui.onNodeWithText("Sides 1").assertTextContains("20")
        ui.onNodeWithText("Number").performScrollTo().performClick()
        ui.onNodeWithText("Minimum").performTextReplacement("101")
        ui.onNodeWithText("Pick a number").assertIsNotEnabled()
        ui.onNodeWithText("Minimum").performTextReplacement("-5")
        ui.onNodeWithText("Maximum").performTextReplacement("-1")
        ui.onNodeWithText("Pick a number").performScrollTo().performClick()
        assertEquals("pick::number::-5--1;",sent.last())
        ui.onNodeWithText("Coin").performScrollTo().performClick()
        ui.onNodeWithText("Flip coin").performScrollTo().performClick()
        assertEquals(listOf("roll::3d20;",NativeCore.randomizerSource("Choice\nFish, chips\nredact::keep this;"),"pick::number::-5--1;","pick::flip;"),sent)
    }
}
