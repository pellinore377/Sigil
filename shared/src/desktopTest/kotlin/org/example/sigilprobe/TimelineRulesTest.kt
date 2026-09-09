package org.sigil

import kotlin.test.*
import org.junit.Test

class TimelineRulesTest {
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
