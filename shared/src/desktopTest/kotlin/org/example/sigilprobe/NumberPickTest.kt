package org.sigil

import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.foundation.layout.Box
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import org.junit.Rule
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

class NumberPickTest {
    @get:Rule val ui = createComposeRule()
    @Test fun figures_use_a_true_minus_and_grouped_thousands() {
        assertEquals("42", numberFigure("42"))
        assertEquals("−10", numberFigure("-10"))
        assertEquals("1,000,000", numberFigure("1000000"))
        assertEquals("0", numberFigure("-0"))
    }
    @Test fun range_reads_from_the_stored_alternate() {
        assertEquals("1–100", numberRange("Between 1 and 100")!!.label())
        assertEquals("−10 to 10", numberRange("Between -10 and 10")!!.label())
        assertNull(numberRange("Something else"))
    }
    @Test fun card_leads_with_its_type_word_and_result() {
        val value = UtilityContent("random", display = "7", alternate = "Between -10 and 10", copy = "7", motion = RandomizerMotion("number", frames = listOf("-10", "0", "10"), result = "7"))
        ui.setContent { MaterialTheme { NumberPickCard(value) } }
        ui.onNodeWithContentDescription("Number pick. 7, from −10 to 10").assertExists()
    }
    @Test fun drum_never_repeats_a_step_and_lands_on_something_new() {
        val frames = listOf("4", "1", "4", "2", "5", "1", "5", "2", "4", "1", "5", "3")
        val seq = drumSequence(frames, "4", 7)
        seq.zipWithNext().forEach { (a, b) -> assertTrue(a != b, "$seq") }
        assertTrue(seq.last() != "4")
        assertEquals(List(27) { "7" }, drumSequence(listOf("7"), "7", 1))
    }
    @Test fun extreme_figure_wraps_instead_of_clipping_at_large_text() {
        val value = UtilityContent("random", display = "-9223372036854775808", alternate = "Between -9223372036854775808 and 9223372036854775807",
            motion = RandomizerMotion("number", frames = listOf("-9223372036854775808", "9223372036854775807"), result = "-9223372036854775808"))
        ui.setContent { MaterialTheme { CompositionLocalProvider(LocalDensity provides Density(1f, 1.3f)) { Box(Modifier.width(260.dp)) { NumberDrum(value.motion!!, false) } } } }
        ui.onAllNodesWithText("−9,223,372,036,854,775,808").fetchSemanticsNodes().also { assertTrue(it.isNotEmpty()) }.forEach { node ->
            val layouts = mutableListOf<TextLayoutResult>()
            node.config[SemanticsActions.GetTextLayoutResult].action!!(layouts)
            assertFalse(layouts.single().hasVisualOverflow)
        }
    }
    @Test fun choice_reads_its_type_and_category() {
        val food = UtilityContent("pick", display = "food", motion = RandomizerMotion("choice", frames = listOf("sushi", "tacos"), result = "sushi"))
        assertEquals("Category pick. Food: sushi", choiceDescription(food))
        assertEquals("Card pick. Museum", choiceDescription(UtilityContent("pick", display = "Choice", motion = RandomizerMotion("choice", result = "Museum"))))
        assertEquals("Category pick. Yes or no: no", choiceDescription(UtilityContent("pick", display = "yesno", motion = RandomizerMotion("choice", result = "no"))))
        ui.setContent { MaterialTheme { CompositionLocalProvider(LocalSolidMaterial provides { _, _, m -> Box(m) }) { UtilityCard(food) } } }
        ui.onNodeWithContentDescription("Category pick. Food: sushi").assertExists()
    }
}
