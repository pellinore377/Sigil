package org.sigil

import kotlin.test.*
import org.junit.Test

class TimelineRulesTest {
    @Test fun old_appearance_values_keep_defaults_and_invalid_sizes_cannot_break_layout() {
        assertEquals(Appearance(font = "Google Sans Flex", mode = "Dark"), decodeAppearance("Google Sans Flex|Dark|555555|false"))
        for (size in listOf("NaN", "Infinity", "0", "200")) {
            val value = decodeAppearance("Newsreader|Light|555555|false|$size|false|999")
            assertEquals(1f, value.textScale)
            assertEquals(1, value.previewLines)
        }
        assertEquals(null, decodeChat(null).gradient)
        assertEquals(false, decodeChat("|false").gradient)
        assertEquals(null, decodeChat(ChatTheme().encode()).gradient)
    }
    @Test fun emoji_matching_keeps_text_and_unknown_sequences_literal() {
        assertEquals(3, animatedEmoji("😀 ❤️ 👍🏽")?.size)
        assertEquals(null, animatedEmoji("😀 hello"))
        assertEquals(null, animatedEmoji(""))
        assertEquals(null, animatedEmoji("😀\u200d😀"))
        assertEquals("1f600", animatedEmoji("😀")?.single()?.key)
    }
    @Test fun swiping_away_replies_and_towards_the_edge_threads_on_both_sides() {
        assertEquals("reply", swipeAction(false, 60f))
        assertEquals("thread", swipeAction(false, -60f))
        assertEquals("reply", swipeAction(true, -60f))
        assertEquals("thread", swipeAction(true, 60f))
    }
}
