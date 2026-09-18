package org.sigil

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.requiredSize
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/// The list against a real layout: what it reports, how deep it holds, and where the reader stays.
class TimelineBufferTest {
    @get:Rule val ui = createComposeRule()
    private val chat = ChatSummary("peer", "Maya Chen", "", "", true, emptyList())
    private fun message(index: Int) = ChatMessage("m$index", "a", "Message $index", index % 2 == 0, "09:00", "Read",
        false, emptyList(), emptyList(), null, true, 5_000_000L - index * 10, peer = "peer")
    private fun conversation(count: Int, from: Int = 0) = MessengerState(phase = "connected", chats = listOf(chat), selected = "peer",
        messages = (from until from + count).map(::message), timelineLoaded = true, more = true,
        timelineBuffer = TimelineBufferDepth(3f, 2f))
    /// Older messages are deeper in the list, so the reader reaches them by dragging down.
    private fun older(times: Int = 1) = repeat(times) {
        ui.onNodeWithTag("timeline").performTouchInput { swipeDown(startY = centerY - 250f, endY = centerY + 250f, durationMillis = 120) }
        ui.waitForIdle()
        repeat(4) { ui.mainClock.advanceTimeBy(160); ui.waitForIdle() }
    }
    private fun onScreen(range: IntRange) = range.filter { runCatching { ui.onNodeWithText("Message $it").assertIsDisplayed() }.isSuccess }

    @Test fun the_depth_the_list_reports_is_a_message_depth_not_a_list_index() {
        val reports = mutableListOf<Int>()
        ui.setContent { Box(Modifier.requiredSize(390.dp, 720.dp)) {
            SigilApp(NativeCore::palette, NativeCore::analyze, conversation(200), { name, fields ->
                if (name == "viewport") reports += fields["end"] as Int
            })
        } }
        ui.waitForIdle()
        older(2)
        val end = reports.last()
        assertTrue(end > 0, "the viewport reached past the newest message")
        // The typing row leads the list, so a list index would name a message one shallower than this one.
        assertEquals(end, onScreen(0..199).last())
    }

    /// Returning to the latest messages replaces the loaded history; the list must report the new shallower
    /// depth, or the core never learns that the reader needs paging again.
    @Test fun the_list_reports_its_depth_again_after_the_history_under_it_is_replaced() {
        val state = mutableStateOf(conversation(200))
        val reports = mutableListOf<Int>()
        ui.setContent { Box(Modifier.requiredSize(390.dp, 720.dp)) {
            SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { name, fields ->
                if (name == "viewport") reports += fields["end"] as Int
            })
        } }
        ui.waitForIdle()
        older(3)
        val deep = reports.max()
        assertTrue(deep > 8, "the reader scrolled well past the newest message")
        // The history the reader was in is dropped and a short live list takes its place.
        ui.runOnIdle { state.value = conversation(0) }
        ui.waitForIdle()
        ui.runOnIdle { state.value = conversation(12) }
        ui.waitForIdle()
        assertTrue(reports.last() < deep, "the shallower depth was reported: ${reports.takeLast(4)} against $deep")
    }

    @Test fun the_buffer_holds_several_screens_past_what_the_reader_can_see() {
        ui.setContent { Box(Modifier.requiredSize(390.dp, 720.dp)) {
            SigilApp(NativeCore::palette, NativeCore::analyze, conversation(200), { _, _ -> })
        } }
        ui.waitForIdle()
        older(3)
        val visible = onScreen(0..199).size
        val composed = ui.onAllNodesWithText("Message ", substring = true).fetchSemanticsNodes().size
        assertTrue(visible > 0, "the conversation is on screen")
        assertTrue(composed >= visible * 3, "several screens are held composed: $composed against $visible on screen")
    }

    @Test fun neither_older_history_nor_a_new_message_moves_what_the_reader_is_looking_at() {
        val state = mutableStateOf(conversation(40, from = 2))
        ui.setContent { Box(Modifier.requiredSize(390.dp, 720.dp)) {
            SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { _, _ -> })
        } }
        ui.waitForIdle()
        older(2)
        val anchor = onScreen(2..41).first()
        assertTrue(anchor > 2, "the reader has scrolled away from the newest message")
        val place = ui.onNodeWithText("Message $anchor").fetchSemanticsNode().boundsInRoot
        // Older history lands behind the reader.
        ui.runOnIdle { state.value = state.value.copy(messages = (2 until 92).map(::message)) }
        ui.waitForIdle()
        assertEquals(place, ui.onNodeWithText("Message $anchor").fetchSemanticsNode().boundsInRoot)
        // Two messages arrive at the head and push every other index along.
        ui.runOnIdle { state.value = state.value.copy(messages = (0 until 92).map(::message)) }
        ui.waitForIdle()
        assertEquals(place, ui.onNodeWithText("Message $anchor").fetchSemanticsNode().boundsInRoot)
    }
}
