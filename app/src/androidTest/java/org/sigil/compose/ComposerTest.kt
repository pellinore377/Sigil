package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*

class ComposerTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    private lateinit var state: TextFieldState
    private val field get() = ui.onNodeWithTag("composer")
    private fun open(source: String) {
        state = TextFieldState(source)
        ui.runOnUiThread { ui.activity.setSigilContent { MaterialTheme { Composer(state, NativeCore::analyze) } } }
        field.performClick()
    }
    private fun rendered(): androidx.compose.ui.text.AnnotatedString {
        val results = mutableListOf<TextLayoutResult>()
        field.performSemanticsAction(SemanticsActions.GetTextLayoutResult) { it(results) }
        return results.single().layoutInput.text
    }
    @Test fun named_formatting_styles_the_editor_and_round_trips_source_after_ime_replacement() {
        val source = "underline::bold::Hello; red-blue::café; big1::wide; 👩🏽‍💻"
        open(source)
        val text = rendered()
        assertEquals("Hello café wide 👩🏽‍💻", text.text)
        assertTrue(text.spanStyles.any { it.start == 0 && it.end == 5 && it.item.fontWeight == FontWeight.Bold })
        assertTrue(text.spanStyles.any { it.start == 0 && it.end == 5 && it.item.textDecoration == TextDecoration.Underline })
        assertTrue(text.spanStyles.filter { it.start >= 6 && it.end <= 10 }.map { it.item.color }.distinct().size >= 2)
        field.performTextInputSelection(TextRange(7, 9), relativeToOriginalText = false)
        field.performTextInput("XY")
        assertEquals("Hello cXYé wide 👩🏽‍💻", rendered().text)
        ui.runOnIdle { assertEquals(source.replace("café", "cXYé"), state.text.toString()) }
        ui.onNodeWithText("Undo").performClick()
        assertEquals("Hello café wide 👩🏽‍💻", rendered().text)
        ui.onNodeWithText("Source").performClick()
        assertEquals(source, rendered().text)
    }
    @Test fun unimplemented_effect_previews_remain_identifiable_in_source() {
        val source = "spoiler::secret; shake::bold::moving; redact::discard;"
        open(source)
        assertEquals(source, rendered().text)
        ui.runOnIdle { assertEquals(source, state.text.toString()) }
    }
}
