@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
package org.sigil

import kotlin.test.*
import org.junit.Test

class TimelineRulesTest {
    @Test fun material_preferences_round_trip_and_reject_invalid_ranges() {
        for(mode in listOf("Personalized","Global","Conversational")) {
            val value=Appearance(objectMode=mode,diceStyle=ObjectStyle(color=0x128854,transmission=.6f),coinStyle=defaultObjectStyle(1).copy(roughness=.7f),cardStyle=defaultObjectStyle(2).copy(border=2),replaySeconds=30)
            assertEquals(value,decodeAppearance(value.encode()))
        }
        assertEquals(defaultObjectStyle(0),decodeObjectStyle("bad,bad,bad,NaN,Infinity,-1,9,9,9,999",0))
        assertEquals(0,decodeAppearance(Appearance(replaySeconds=1).encode()).replaySeconds)
    }
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
    @Test fun only_messages_that_arrive_at_the_head_of_a_live_list_carry_arrival_motion() {
        val arrivals=TimelineArrivals()
        arrivals.update(listOf("c","b","a"), loaded=false, live=true)
        assertEquals(emptySet(), arrivals.pending())
        arrivals.update(listOf("c","b","a"), loaded=true, live=true)
        assertEquals(emptySet(), arrivals.pending())
        arrivals.update(listOf("e","d","c","b","a"), loaded=true, live=true)
        assertEquals(setOf("e","d"), arrivals.pending())
        // Older history appended behind what is loaded is not an arrival.
        arrivals.update(listOf("e","d","c","b","a","z","y"), loaded=true, live=true)
        assertEquals(setOf("e","d"), arrivals.pending())
        // An archived or filtered view shows what it is given without motion.
        val quiet=TimelineArrivals()
        quiet.update(listOf("a"), loaded=true, live=false)
        quiet.update(listOf("b","a"), loaded=true, live=false)
        assertEquals(emptySet(), quiet.pending())
    }
    @Test fun an_arrival_plays_once_however_often_the_message_is_composed() {
        val arrivals=TimelineArrivals()
        arrivals.update(listOf("b","a"), loaded=true, live=true)
        arrivals.update(listOf("c","b","a"), loaded=true, live=true)
        assertEquals(true, arrivals.claim("c"))
        // Scrolled out of the buffer and back: the message is simply there.
        assertEquals(false, arrivals.claim("c"))
        arrivals.update(listOf("c","b","a"), loaded=true, live=true)
        assertEquals(false, arrivals.claim("c"))
        // A message the reader never saw arrive never animates.
        assertEquals(false, arrivals.claim("a"))
    }
    @Test fun an_arrival_that_landed_screens_away_is_simply_there_when_the_reader_reaches_it() {
        var clock=0L
        val arrivals=TimelineArrivals { clock }
        arrivals.update(listOf("b","a"), loaded=true, live=true)
        arrivals.update(listOf("c","b","a"), loaded=true, live=true)
        // The reader is far enough up the timeline that nothing composes the new head for a while.
        clock=ArrivalGraceMillis+1
        assertEquals(false, arrivals.claim("c"))
        // A second arrival that the reader is watching still rises into place.
        arrivals.update(listOf("d","c","b","a"), loaded=true, live=true)
        assertEquals(setOf("d"), arrivals.pending())
        assertEquals(true, arrivals.claim("d"))
    }
    @Test fun the_list_keeps_several_screens_composed_on_the_depth_the_core_reports() {
        var depth: TimelineBufferDepth?=null
        val window=TimelineCacheWindow { depth }
        val density=androidx.compose.ui.unit.Density(1f)
        with(window) { with(density) {
            // One screen either way before the core has answered, never less than the list had without a window.
            assertEquals(1000, calculateAheadWindow(1000))
            assertEquals(1000, calculateBehindWindow(1000))
            depth=TimelineBufferDepth(3f, 2f)
            assertEquals(3000, calculateAheadWindow(1000))
            assertEquals(2000, calculateBehindWindow(1000))
        } }
    }
    @Test fun a_scan_publishes_what_the_reader_can_see_and_never_stalls_below_the_target() {
        // A refresh under a reader who is 192 deep holds its pages back until it has caught up with them.
        assertEquals(false, timelinePublishes(held=64, onScreen=192, want=256, last=false))
        assertEquals(true, timelinePublishes(held=192, onScreen=192, want=256, last=false))
        // The last page always publishes, however short.
        assertEquals(true, timelinePublishes(held=10, onScreen=192, want=256, last=true))
        // Returning to the latest messages drops the target below what is on screen: the scan stops at the
        // target, so it must publish there rather than run out of pages having published nothing.
        assertEquals(true, timelinePublishes(held=128, onScreen=192, want=128, last=false))
        assertEquals(false, timelinePublishes(held=64, onScreen=192, want=128, last=false))
    }
    @Test fun the_anchor_keeps_the_readers_place_across_prepends_and_deletions() {
        val anchor=TimelineAnchor()
        assertEquals(null, anchor.settle(listOf("c","b","a"), lead=1))
        anchor.record("b", 40)
        // Two newer messages arrive: the anchored message moved two places down the list.
        assertEquals(4 to 40, anchor.settle(listOf("e","d","c","b","a"), lead=1))
        anchor.record("b", 40)
        // Nothing moved under the reader, so nothing is scrolled.
        assertEquals(null, anchor.settle(listOf("e","d","c","b","a"), lead=1))
        anchor.record("b", 12)
        // The anchor itself was deleted: hold the nearest message that survived it.
        assertEquals(3 to 12, anchor.settle(listOf("d","c","a"), lead=1))
        anchor.record("c", 5)
        // Older history appended behind the reader leaves the anchor exactly where it was.
        assertEquals(null, anchor.settle(listOf("d","c","a","z","y"), lead=1))
    }
    @Test fun an_anchor_the_list_never_held_moves_nothing() {
        assertEquals(null, anchorPlace(listOf("a","b"), listOf("a","b"), null))
        assertEquals(null, anchorPlace(listOf("a","b"), listOf("a","b"), "zz"))
        assertEquals(0, anchorPlace(listOf("a"), listOf("b","a"), "b"))
    }
    @Test fun swiping_away_replies_and_towards_the_edge_threads_on_both_sides() {
        assertEquals("reply", swipeAction(false, 60f))
        assertEquals("thread", swipeAction(false, -60f))
        assertEquals("reply", swipeAction(true, -60f))
        assertEquals("thread", swipeAction(true, 60f))
    }
}
