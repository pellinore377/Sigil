package org.sigil

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class ArithmeticLayoutTest {
    @get:Rule val ui=createComposeRule()

    private fun spoken(source:String)=arithSpoken(assertNotNull(parseArith(source),source))
    private fun words(source:String)=arithTokens(assertNotNull(parseArith(source))).map {(it as? ArithToken.Word)?.text ?: "^"}
    private fun draw(source:String,width:Int=400) {ui.setContent {MaterialTheme {Box(Modifier.width(width.dp)) {ArithmeticLine(arithTokens(parseArith(source)!!),MaterialTheme.typography.bodyLarge)}}}}
    private fun box(text:String)=ui.onNodeWithText(text).fetchSemanticsNode().boundsInRoot

    @Test fun operators_read_as_mathematics_on_screen_and_aloud() {
        assertEquals(listOf("17","×","34"),words("17 * 34"))
        assertEquals(listOf("(24","+","18)","÷","2"),words("(24 + 18) / 2"))
        assertEquals(listOf("−","3","−","−","4.5","mod","1,200"),words("-3 - -4.5 % 1200"))
        assertEquals("17 times 34",spoken("17*34"))
        assertEquals("open paren 24 plus 18 close paren divided by 2",spoken("(24 + 18) / 2"))
        assertEquals("2 to the power of 8",spoken("2 ^ 8"))
        assertEquals("minus 3 squared",spoken("-3^2"))
        assertEquals("2 to the power of 3 squared, end power",spoken("2^3^2"))
        assertEquals("2 to the power of minus 3 times 4",spoken("2^-3*4"))
        assertEquals("1 divided by open paren 2 times 10 to the power of minus 2 close paren",spoken("1 / 2e-2"))
        assertEquals("1.5 times 10 to the power of minus 3",spoken("1.5e-3"))
        assertEquals("minus 12,345.5",spokenFigure(readableNumber("-12345.5")))
    }

    @Test fun parsing_matches_the_calculator_grammar() {
        val pow=assertIs<Arith.Op>(parseArith("2^3^2"))
        assertEquals('^',pow.op); assertIs<Arith.Num>(pow.left); assertEquals('^',assertIs<Arith.Op>(pow.right).op)
        assertIs<Arith.Sign>(parseArith("-2^2"),"Unary minus sits under the power, as in the Rust calculator")
        assertNull(parseArith("2 +"))
        assertNull(parseArith("(2"))
        assertNull(parseArith("2 ** 3"))
    }

    @Test fun an_exponent_is_a_smaller_raised_figure() {
        draw("2 ^ 8")
        val base=box("2"); val power=box("8")
        assertTrue(power.height<base.height,"The exponent is set smaller")
        assertTrue(power.bottom<base.bottom-base.height*.25f,"The exponent sits above the base line")
        assertTrue(power.left>=base.right,"The exponent follows its base")
    }

    @Test fun nested_powers_keep_stepping_up() {
        draw("2 ^ 3 ^ 2")
        val base=ui.onAllNodesWithText("2")[0].fetchSemanticsNode().boundsInRoot; val first=box("3"); val second=ui.onAllNodesWithText("2")[1].fetchSemanticsNode().boundsInRoot
        assertTrue(second.bottom<=first.top+first.height*.5f&&second.height<first.height,"A power of a power rises clear of the first and shrinks again")
        assertTrue(first.height<base.height)
    }

    @Test fun a_long_expression_breaks_before_an_operator() {
        draw("1234567 * 7654321 + 1111111 * 2222222 - 3333333 / 4444444",140)
        val start=box("1,234,567").left
        assertTrue(ui.onAllNodes(hasText("−")or hasText("+")or hasText("×")or hasText("÷")).fetchSemanticsNodes().any {it.boundsInRoot.left==start},"A wrapped line starts with its operator at the start edge")
        assertTrue(box("4,444,444").top>box("1,234,567").bottom,"The expression wraps instead of clipping")
        val gap=ui.onAllNodesWithText("×")[1].fetchSemanticsNode().boundsInRoot.top-box("1,234,567").top
        assertTrue(gap>=box("1,234,567").height,"Wrapped lines keep their leading")
    }

    @Test fun a_powered_long_group_wraps_and_keeps_its_exponent_in_view() {
        draw("(12345.5 + 67890.25 + 11111.75 + 22222.5 + 33333.25 + 44444) ^ 2",200)
        val card=200f*ui.density.density
        val power=ui.onAllNodesWithText("2")[0].fetchSemanticsNode().boundsInRoot
        assertTrue(power.right<=card+.5f,"The exponent stays inside the card")
        assertTrue(power.top>box("(12,345.5").bottom,"The group wraps across lines")
    }

    @Test fun a_long_number_breaks_after_a_grouping_comma() {
        draw("1234567890123456789012345678901234567890 + 1",200)
        val right=ui.onAllNodes(hasText(",",substring=true)).fetchSemanticsNodes().maxOf {it.boundsInRoot.right}
        assertTrue(right<=200f*ui.density.density+.5f,"No digit group runs past the card")
    }

    @Test fun an_unbreakable_exponent_wider_than_the_card_reports_overflow() {
        var overflow=0; var fits=0
        ui.setContent {MaterialTheme {Box(Modifier.width(200.dp)) {
            ArithmeticLine(arithTokens(parseArith("2 ^ ("+List(40) {"1"}.joinToString(" + ")+")")!!),MaterialTheme.typography.bodyLarge,onOverflow={overflow++})
        }}}
        ui.waitForIdle()
        assertTrue(overflow>0,"The card is told to fall back to the wrapped source")
        ui.setContent {MaterialTheme {Box(Modifier.width(200.dp)) {
            ArithmeticLine(arithTokens(parseArith("(12345.5 + 67890.25 + 11111.75 + 22222.5) ^ 2")!!),MaterialTheme.typography.bodyLarge,onOverflow={fits++})
        }}}
        ui.waitForIdle()
        assertEquals(0,fits,"A wrappable expression keeps the typeset form")
    }
}
