@file:OptIn(androidx.compose.ui.test.ExperimentalTestApi::class)
package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class HelpPanelTest {
    @get:Rule val ui=createComposeRule()
    @Test fun search_and_copy_stay_local_until_explicit_sharing() {
        val sent=mutableListOf<String>()
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalHelpCatalog provides NativeCore::helpCatalog) {
            Box(Modifier.size(360.dp,360.dp)) {HelpPanel(true,{},send=sent::add)}
        }}}
        ui.onNodeWithText("Search SigilText").performTextInput("animations horizontal")
        ui.onNodeWithText("shake").assertIsDisplayed().performClick()
        ui.onNodeWithText("shake::Whoa;").assertIsDisplayed()
        ui.onNodeWithText("Copy example").performScrollTo().performClick()
        assertTrue(sent.isEmpty())
        ui.onNodeWithText("Send cheat sheet").performScrollTo().performClick()
        assertEquals(listOf("help::shake;"),sent)
        ui.onNodeWithContentDescription("Back to help").performClick()
        ui.onNodeWithText("Search SigilText").performTextReplacement("no matching construct")
        ui.onNodeWithText("No matching topics.").assertExists()
        assertEquals(1,sent.size)
    }
    @Test fun typed_reference_supports_keyboard_navigation_and_escape() {
        var closed by mutableStateOf(false)
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalHelpCatalog provides NativeCore::helpCatalog) {
            if(!closed)Box(Modifier.size(360.dp,360.dp)) {HelpPanel(true,{closed=true},"animations") {error("Browsing must not send")}}
        }}}
        ui.onNodeWithText("Search SigilText").assertIsFocused().performKeyInput {pressKey(Key.DirectionDown);pressKey(Key.Enter)}
        ui.onNodeWithText("shake::Whoa;").assertExists()
        ui.onNodeWithContentDescription("Back to help").performClick()
        ui.onNodeWithText("Search SigilText").performClick().performKeyInput {pressKey(Key.Escape)}
        ui.onNodeWithText("Search SigilText").assertDoesNotExist()
    }
}
