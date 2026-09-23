package org.sigil

import kotlin.test.*
import org.junit.Test

class RandomizerCaptionTest {
    @Test fun dice_details_group_in_written_order_and_read_as_a_breakdown() {
        val groups = diceGroups(listOf("d6 · 1", "d6 · 3", "d20 · 11", "d6 · 2").map { RichText(it) })
        assertEquals(listOf(DiceGroup(6, listOf(1, 3)), DiceGroup(20, listOf(11)), DiceGroup(6, listOf(2))), groups)
        assertEquals("2d6" to "1 · 3 = 4", diceGroupLine(groups[0]))
        assertEquals("d20" to "11", diceGroupLine(groups[1]))
        assertEquals("24d6" to "= 84", diceGroupLine(DiceGroup(6, List(24) { 3 + it % 2 })))
        assertTrue(diceGroups(listOf(RichText("not a die"))).isEmpty())
    }
    @Test fun caption_labels_breakdown_and_speech_follow_the_roll() {
        val one = captionModel(false, "17", listOf(DiceGroup(20, listOf(17))), 1)
        assertEquals("Result", one.label); assertFalse(one.breakdown); assertNull(one.note)
        assertEquals("Dice roll. Result: 17", one.spoken)
        val party = captionModel(false, "19", listOf(DiceGroup(6, listOf(5, 3)), DiceGroup(20, listOf(11))), 3)
        assertEquals("Total", party.label); assertTrue(party.breakdown)
        assertEquals("Dice roll. Total 19: 2d6, 5 and 3; d20, 11", party.spoken)
        val many = captionModel(false, "84", listOf(DiceGroup(6, List(24) { 3 + it % 2 })), 6)
        assertFalse(many.breakdown, "one large group would only repeat the total")
        assertEquals("6 of 24 dice shown", many.note)
        assertEquals("Dice roll. Total 84: 24d6, 84", many.spoken)
        val percentile = captionModel(false, "47", listOf(DiceGroup(100, listOf(47))), 1)
        assertEquals("Result", percentile.label); assertFalse(percentile.breakdown)
        assertEquals("1,234", captionModel(false, "1234", listOf(DiceGroup(100, List(30) { 41 })), 6).figure)
        val coin = captionModel(true, "Tails", emptyList(), 1)
        assertEquals("Result", coin.label); assertFalse(coin.breakdown); assertNull(coin.note)
        assertEquals("Coin flip. Result: Tails", coin.spoken)
    }
    @Test fun stage_says_only_its_type_outside_the_menu() {
        assertEquals("Coin flip", stageDescription(true, "Tails", emptyList(), false))
        assertEquals("Dice roll", stageDescription(false, "8", listOf(DieFace(6, 5), DieFace(6, 3)), false))
        assertEquals("Dice roll. d6, 5; d6, 3", stageDescription(false, "8", listOf(DieFace(6, 5), DieFace(6, 3)), true))
    }
}
