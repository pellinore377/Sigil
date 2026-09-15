package org.sigil

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.foundation.layout.requiredSize
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class FloatingShellTest {
    @get:Rule val ui = createComposeRule()
    private val chat = ChatSummary("peer", "Maya Chen", "", "", true, emptyList())

    @Test fun inbox_header_is_absent_until_the_outgoing_conversation_is_disposed() {
        val state=mutableStateOf(MessengerState(phase="connected",chats=listOf(chat)))
        ui.setContent {Box(Modifier.requiredSize(390.dp,844.dp)) {
            SigilApp(NativeCore::palette,NativeCore::analyze,state.value,{_,_->})
        }}
        ui.waitForIdle()
        val original=ui.onNodeWithTag("main-header").fetchSemanticsNode().boundsInRoot
        ui.mainClock.autoAdvance=false
        ui.runOnIdle {state.value=state.value.copy(selected=chat.id)}
        ui.mainClock.advanceTimeBy(600)
        ui.onNodeWithTag("main-header").assertDoesNotExist()
        ui.runOnIdle {state.value=state.value.copy(selected=null)}
        var sampledOutgoing=false
        var removedAt=0
        var headerAt=0
        repeat(60) {frame->
            ui.mainClock.advanceTimeBy(16)
            val outgoing=ui.onAllNodesWithTag("conversation-page").fetchSemanticsNodes().isNotEmpty()
            if(!outgoing && removedAt==0)removedAt=(frame+1)*16
            if(ui.onAllNodesWithTag("main-header").fetchSemanticsNodes().isNotEmpty() && headerAt==0)headerAt=(frame+1)*16
            if(outgoing) {
                sampledOutgoing=true
                ui.onNodeWithTag("main-header").assertDoesNotExist()
                ui.onNodeWithContentDescription("Search conversations").assertDoesNotExist()
            }
        }
        assertTrue(sampledOutgoing)
        assertTrue(headerAt>=removedAt && removedAt>0)
        println("Reverse transition: conversation removed at ${removedAt}ms; inbox header mounted at ${headerAt}ms")
        ui.mainClock.advanceTimeBy(600)
        ui.onNodeWithTag("conversation-page").assertDoesNotExist()
        ui.onNodeWithTag("main-header").assertIsDisplayed()
        assertEquals(original,ui.onNodeWithTag("main-header").fetchSemanticsNode().boundsInRoot)
        ui.onNodeWithContentDescription("Search conversations").assertHasClickAction().assertIsDisplayed()
    }

    @Test fun wallpaper_travels_with_the_conversation_in_both_directions() {
        val state=mutableStateOf(MessengerState(phase="connected",chats=listOf(chat)))
        ui.setContent {Box(Modifier.requiredSize(390.dp,844.dp)) {
            CompositionLocalProvider(LocalWallpaper provides {_,modifier->Box(modifier.testTag("moving-wallpaper"));true}) {
                SigilApp(NativeCore::palette,NativeCore::analyze,state.value,{_,_->})
            }
        }}
        ui.waitForIdle();ui.mainClock.autoAdvance=false
        ui.runOnIdle {state.value=state.value.copy(selected=chat.id)}
        ui.mainClock.advanceTimeBy(112)
        val entering=ui.onNodeWithTag("conversation-page").getUnclippedBoundsInRoot()
        assertTrue(entering.top.value>20)
        assertEquals(entering.top,ui.onNodeWithTag("moving-wallpaper").getUnclippedBoundsInRoot().top)
        ui.mainClock.advanceTimeBy(500)
        ui.runOnIdle {state.value=state.value.copy(selected=null)}
        ui.mainClock.advanceTimeBy(112)
        val leaving=ui.onNodeWithTag("conversation-page").getUnclippedBoundsInRoot()
        assertTrue(leaving.top.value>20)
        assertEquals(leaving.top,ui.onNodeWithTag("moving-wallpaper").getUnclippedBoundsInRoot().top)
        ui.mainClock.advanceTimeBy(500)
        ui.onNodeWithTag("conversation-page").assertDoesNotExist()
        ui.onNodeWithTag("main-navigation").assertIsDisplayed()
    }

    @Test fun thread_uses_message_composer_and_preserves_thread_target() {
        val commands = mutableListOf<Pair<String, Map<String, Any?>>>()
        ui.setContent {
            SigilApp(NativeCore::palette, NativeCore::analyze,
                MessengerState(phase = "connected", chats = listOf(chat), selected = chat.id, threadTarget = ThreadTarget("author", "root")),
                { name, fields -> commands += name to fields })
        }
        ui.onNodeWithText("Thread").assertExists()
        ui.onNodeWithTag("composer").performTextInput("A reply")
        ui.onNodeWithContentDescription("Send message").performClick()
        ui.runOnIdle {
            val post = commands.single { it.first == "post" }.second
            assertEquals("root", post["thread_message"])
            assertEquals("author", post["thread_author"])
            assertEquals("A reply", post["text"])
        }
    }

    @Test fun header_and_composer_are_inset_and_timeline_extends_behind_them() {
        ui.setContent {
            SigilApp(NativeCore::palette, NativeCore::analyze,
                MessengerState(phase = "connected", chats = listOf(chat), selected = chat.id), { _, _ -> })
        }
        ui.waitForIdle()
        val header = ui.onNodeWithTag("conversation-header").fetchSemanticsNode().boundsInRoot
        val timeline = ui.onNodeWithTag("timeline").fetchSemanticsNode().boundsInRoot
        val input = ui.onNodeWithTag("composer").fetchSemanticsNode().boundsInRoot
        assertTrue(header.left > timeline.left)
        assertTrue(header.top > timeline.top)
        assertTrue(input.bottom < timeline.bottom)
    }

    @Test fun leaving_thread_for_pins_clears_target_and_renders_each_pin_once() {
        val commands = mutableListOf<Pair<String, Map<String, Any?>>>()
        val message = ChatMessage("pin", "author", "Meet at the station", false, "9:30", "", true, emptyList(), emptyList(), null, true)
        ui.setContent {
            SigilApp(NativeCore::palette, NativeCore::analyze,
                MessengerState(phase = "connected", chats = listOf(chat), selected = chat.id,
                    threadTarget = ThreadTarget("author", "root"), messages = listOf(message)),
                { name, fields -> commands += name to fields })
        }
        ui.onNodeWithContentDescription("Conversation menu").performClick()
        ui.onNodeWithText("Pins").performClick()
        ui.waitForIdle()
        ui.onNodeWithText("Thread").assertDoesNotExist()
        ui.onAllNodesWithText("Meet at the station").assertCountEquals(1)
        ui.runOnIdle {
            val filter = commands.last { it.first == "timeline_filter" }.second
            assertEquals("Pins", filter["category"])
            assertEquals(null, filter["thread_message"])
            assertEquals(null, filter["thread_author"])
        }
    }

@Test fun conversation_chrome_waits_for_timeline_entry() {
    val state = mutableStateOf(MessengerState(phase = "connected", chats = listOf(chat)))
    ui.setContent { Box(Modifier.requiredSize(390.dp, 844.dp)) { SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { _, _ -> }) } }
    ui.waitForIdle()
    ui.mainClock.autoAdvance = false
    ui.runOnIdle { state.value = state.value.copy(selected = chat.id) }
    ui.mainClock.advanceTimeBy(128)
    ui.onNodeWithTag("conversation-header").assertIsNotDisplayed()
    ui.onNodeWithTag("conversation-footer").assertIsNotDisplayed()
    ui.onNodeWithTag("timeline-body").assertExists()
    ui.mainClock.advanceTimeBy(400)
    ui.onNodeWithTag("conversation-header").assertIsDisplayed()
    ui.onNodeWithTag("conversation-footer").assertIsDisplayed()
    ui.onNodeWithTag("main-header").assertDoesNotExist()
}

@Test fun appearance_wallpaper_extends_under_preview_chrome() {
    ui.setContent {
        SigilTheme(Appearance(), palette = NativeCore::palette) {
            CompositionLocalProvider(LocalWallpaper provides { _, modifier -> Box(modifier.testTag("preview-wallpaper")); true }) {
                ChatAppearance(ChatTheme(), NativeCore::analyze, "peer", { _, _ -> }, {}) {}
            }
        }
    }
    val preview = ui.onNodeWithTag("timeline-preview").fetchSemanticsNode().boundsInRoot
    val wallpaper = ui.onNodeWithTag("preview-wallpaper").fetchSemanticsNode().boundsInRoot
    val footer = ui.onNodeWithTag("preview-footer").fetchSemanticsNode().boundsInRoot
    assertEquals(preview, wallpaper)
    assertTrue(wallpaper.bottom >= footer.bottom)
    assertTrue(wallpaper.top < footer.top)
}

@Test fun density_setting_changes_actual_inbox_row_height_and_roundtrips() {
    val appearance = mutableStateOf(Appearance())
    ui.setContent {
        SigilTheme(appearance.value, palette = NativeCore::palette) {
            AppearancePage(appearance.value, NativeCore::analyze, false, {}, section = "appearance-layout", update = { appearance.value = it })
        }
    }
    val comfortable = ui.onNodeWithTag("inbox-density-preview").fetchSemanticsNode().boundsInRoot.height
    ui.onNodeWithText("Compact").performClick()
    val compact = ui.onNodeWithTag("inbox-density-preview").fetchSemanticsNode().boundsInRoot.height
    assertTrue(comfortable - compact >= 16f)
    ui.runOnIdle { assertTrue(decodeAppearance(appearance.value.encode()).compact) }
    ui.onNodeWithText("Comfortable").performClick()
    assertEquals(comfortable, ui.onNodeWithTag("inbox-density-preview").fetchSemanticsNode().boundsInRoot.height)
    ui.runOnIdle { assertEquals(false, decodeAppearance(appearance.value.encode()).compact) }
}

    @Test fun desktop_notes_use_main_workspace() {
        ui.setContent {
            Box(Modifier.requiredSize(1280.dp, 800.dp)) {
                SigilApp(NativeCore::palette, NativeCore::analyze,
                    MessengerState(phase = "connected", chats = listOf(chat),
                        searchHits = listOf(SearchHit(chat.id, "note", "author", "Ferry times", "", noted = true))),
                    { _, _ -> }, wideLayout = true)
            }
        }
        ui.onNodeWithContentDescription("Notes").performClick()
        ui.waitForIdle()
        assertTrue(ui.onNodeWithTag("notes-grid").fetchSemanticsNode().boundsInRoot.width > 600f)
        ui.onNodeWithText("Ferry times").assertExists()
    }
    @Test fun inbox_collections_scroll_beneath_inset_header_with_rows() {
        val chats = (0..30).map { chat.copy(id = "peer-$it", displayName = "Conversation $it") }
        ui.setContent { Box(Modifier.requiredSize(390.dp, 740.dp)) {
            SigilApp(NativeCore::palette, NativeCore::analyze,
                MessengerState(phase = "connected", chats = chats, collectionsEnabled = true,
                    collections = listOf(CollectionItem("friends", "Friends"), CollectionItem("family", "Family and neighbours"))), { _, _ -> })
        } }
        ui.waitForIdle()
        val header = ui.onNodeWithTag("main-header").fetchSemanticsNode().boundsInRoot
        val list = ui.onNodeWithTag("inbox-list").fetchSemanticsNode().boundsInRoot
        val collections = ui.onNodeWithTag("inbox-collections").fetchSemanticsNode().boundsInRoot
        assertTrue(header.left > list.left && header.top > list.top)
        assertTrue(collections.top > header.bottom)
        val short = ui.onNodeWithContentDescription("Friends").fetchSemanticsNode().boundsInRoot
        val long = ui.onNodeWithContentDescription("Family and neighbours").fetchSemanticsNode().boundsInRoot
        assertEquals(short.size, long.size)
        val footer = ui.onNodeWithTag("main-navigation").fetchSemanticsNode().boundsInRoot
        assertTrue(footer.left > list.left && footer.right < list.right && footer.bottom < list.bottom)
        ui.onNodeWithContentDescription("Messages", useUnmergedTree = true).assertIsSelected()
        ui.onNodeWithTag("inbox-list").performScrollToIndex(12)
        ui.onNodeWithTag("inbox-collections").assertDoesNotExist()
        assertEquals(header, ui.onNodeWithTag("main-header").fetchSemanticsNode().boundsInRoot)
        ui.onNodeWithContentDescription("Settings", useUnmergedTree = true).performClick()
        ui.onNodeWithContentDescription("Settings", useUnmergedTree = true).assertIsSelected()
    }

    @Test fun disabling_collections_removes_the_hidden_filter() {
        val state = mutableStateOf(MessengerState(phase = "connected", collectionsEnabled = true,
            collections = listOf(CollectionItem("friends", "Friends")), chats = listOf(
                chat.copy(id = "friend", displayName = "Maya", collections = listOf("friends")),
                chat.copy(id = "other", displayName = "Robin"))))
        ui.setContent { Box(Modifier.requiredSize(390.dp, 740.dp)) {
            SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { _, _ -> })
        } }
        ui.onNodeWithContentDescription("Friends").performClick()
        ui.onNodeWithText("Maya").assertIsDisplayed()
        ui.onNodeWithText("Robin").assertDoesNotExist()
        ui.runOnIdle { state.value = state.value.copy(collectionsEnabled = false) }
        ui.onNodeWithTag("inbox-collections").assertDoesNotExist()
        ui.onNodeWithText("Maya").assertIsDisplayed()
        ui.onNodeWithText("Robin").assertIsDisplayed()
    }

    @Test fun status_protection_hides_content_under_system_icons_and_fades_below() {
        ui.setContent { SigilTheme(Appearance(), palette = NativeCore::palette) {
            Box(Modifier.requiredSize(100.dp, 120.dp).background(Color.Red).testTag("status-protection-preview")) {
                StatusFade(48.dp)
            }
        } }
        val pixels = ui.onNodeWithTag("status-protection-preview").captureToImage().toPixelMap()
        val x = pixels.width / 2
        fun at(y: Int) = pixels[x, y * pixels.height / 120]
        assertEquals(at(0), at(40))
        assertTrue(at(60) != at(40) && at(60) != Color.Red)
        assertTrue(at(70) != at(60) && at(70) != Color.Red)
        assertEquals(Color.Red, at(80))
        assertEquals(Color.Red, at(90))
    }

    @Test fun main_headers_and_footer_keep_approved_proportions_and_grow_for_large_text() {
        val state = mutableStateOf(MessengerState(phase = "connected", chats = listOf(chat)))
        ui.setContent { Box(Modifier.requiredSize(390.dp, 740.dp)) {
            SigilApp(NativeCore::palette, NativeCore::analyze, state.value, { _, _ -> })
        } }
        fun headerHeight() = ui.onNodeWithTag("main-header").getUnclippedBoundsInRoot().let { it.bottom - it.top }
        assertEquals(68.dp, headerHeight())
        assertEquals(64.dp, ui.onNodeWithTag("main-navigation").getUnclippedBoundsInRoot().let { it.bottom - it.top })
        ui.onNodeWithContentDescription("Calls").performClick()
        assertEquals(68.dp, headerHeight())
        ui.onNodeWithContentDescription("Settings").performClick()
        assertEquals(68.dp, headerHeight())
        ui.onNodeWithText("Privacy").performClick()
        assertEquals(68.dp, headerHeight())
        ui.onAllNodesWithText("Privacy").assertCountEquals(1)
        ui.runOnIdle { state.value = state.value.copy(ui = mapOf("appearance" to Appearance(textScale = 1.3f).encode())) }
        assertTrue(headerHeight() in 70.dp..72.dp)
        ui.onNodeWithText("Privacy").assertIsDisplayed()
    }

    @Test fun draft_is_in_header_and_notes_is_the_third_navigation_destination() {
        ui.setContent { Box(Modifier.requiredSize(390.dp, 740.dp)) {
            SigilApp(NativeCore::palette, NativeCore::analyze,
                MessengerState(phase = "connected", chats = listOf(chat), searchHits = listOf(
                    SearchHit(chat.id, "note", "author", "Ferry times", "", noted = true))), { _, _ -> })
        } }
        ui.onAllNodesWithContentDescription("New conversation").assertCountEquals(1)
        val header = ui.onNodeWithTag("main-header").fetchSemanticsNode().boundsInRoot
        val draft = ui.onNodeWithContentDescription("New conversation").fetchSemanticsNode().boundsInRoot
        assertTrue(draft.top >= header.top && draft.bottom <= header.bottom && draft.right <= header.right)
        val tabs = listOf("Messages", "Calls", "Notes", "Settings").map {
            ui.onNodeWithContentDescription(it).fetchSemanticsNode().boundsInRoot.center.x
        }
        assertEquals(tabs.sorted(), tabs)
        ui.onNodeWithContentDescription("Notes").performClick()
        ui.onNodeWithContentDescription("Notes").assertIsSelected()
        ui.onNodeWithTag("main-navigation").assertIsDisplayed()
        ui.onNodeWithText("Ferry times").assertIsDisplayed()
        ui.onNodeWithText("Search notes").assertDoesNotExist()
        ui.onNodeWithContentDescription("Search notes").performClick()
        ui.onNodeWithText("Search notes").assertIsDisplayed()
        ui.onNodeWithContentDescription("Close notes search").performClick()
        ui.onNodeWithContentDescription("Messages").performClick()
        ui.onNodeWithContentDescription("New conversation").performClick()
        ui.onNodeWithText("New conversation").assertIsDisplayed()
    }

    @Test fun conversation_selection_keeps_primary_actions_visible_and_preserves_overflow() {
        val commands = mutableListOf<Pair<String, Map<String, Any?>>>()
        ui.setContent { Box(Modifier.requiredSize(320.dp, 740.dp)) {
            SigilApp(NativeCore::palette, NativeCore::analyze,
                MessengerState(phase = "connected", chats = listOf(chat)), { name, fields -> commands += name to fields })
        } }
        ui.onNodeWithText(chat.name).performTouchInput { longClick() }
        ui.onNodeWithText("1 selected").assertIsDisplayed()
        ui.onNodeWithText(chat.name).assertIsSelected()
        ui.onNodeWithContentDescription("Pin conversations").assertIsDisplayed()
        ui.onNodeWithContentDescription("Add to collection").assertIsDisplayed()
        ui.onNodeWithContentDescription("New conversation").assertDoesNotExist()
        ui.onNodeWithContentDescription("More conversation actions").performClick()
        ui.onNodeWithText("Snooze or unsnooze").assertIsDisplayed()
        ui.onNodeWithText("Block contacts").assertIsDisplayed()
        ui.onNodeWithText("Delete conversations").assertIsDisplayed()
        ui.onNodeWithText("Mark unread").performClick()
        ui.onNodeWithContentDescription("New conversation").assertIsDisplayed()
        ui.runOnIdle {
            assertEquals(listOf("organize"), commands.map { it.first })
            assertEquals(chat.id, commands.single().second["peer"])
            assertEquals(mapOf("Unread" to true), commands.single().second["value"])
        }
    }

    @Test fun returning_to_a_minimized_call_reopens_it_without_restarting_backend() {
        val commands = mutableListOf<Pair<String, Map<String, Any?>>>()
        val call = CallSummary("active-call", "active", true, 100, listOf(
            CallParticipant("participant", chat.id, chat.name, false, true, true, false, false)), name = chat.name)
        ui.setContent { Box(Modifier.requiredSize(390.dp, 740.dp)) {
            SigilApp(NativeCore::palette, NativeCore::analyze,
                MessengerState(phase = "connected", chats = listOf(chat), calls = listOf(call),
                    call = ActiveCall(call, chat.name, connection = "connected")),
                { name, fields -> commands += name to fields })
        } }
        ui.onNodeWithContentDescription("Minimize call").assertIsDisplayed().performClick()
        ui.onNodeWithContentDescription("Calls").performClick()
        ui.onNodeWithTag("main-page-calls").assertIsDisplayed()
        ui.onNodeWithContentDescription("Return to call with ${chat.name}").performClick()
        ui.onNodeWithContentDescription("Minimize call").assertIsDisplayed()
        ui.onNodeWithTag("main-page-calls").assertDoesNotExist()
        ui.runOnIdle { assertTrue(commands.none { it.first in listOf("call_resume", "call_start", "call_redial") }) }
    }

}
