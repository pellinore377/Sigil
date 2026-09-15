package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.runtime.*
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import org.junit.*
import org.junit.Assert.*
import org.sigil.*

class LocationTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    @Test fun pinUsesComposerPanelAndKeepsInputAfterFailedSend() {
        var sent: Map<String, Any?>? = null
        var attempts = 0
        val chat = ChatSummary("self", "@sam:example.test", "", "", true, emptyList(), displayName = "Sam")
        ui.setContent { CompositionLocalProvider(LocalPlacePanel provides { target, back, done -> PlacePanel(back, initialMode=target["location_mode"] as? String ?: "once", caption=target["location_caption"] as? String ?: "", mapContent={choose->Box(Modifier.fillMaxSize().testTag("pin-map").clickable {choose(0.0,0.0)})}) { fields ->
            assertEquals("self", target["peer"])
            if (attempts++ == 0) false else { sent = fields; done(); true }
        } }) { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", selected = "self", chats = listOf(chat)), { _, _ -> }) } }
        val header = ui.onNodeWithTag("conversation-header").fetchSemanticsNode()
        ui.onNodeWithTag("composer").performTextInput("Synthetic pin")
        ui.onNodeWithContentDescription("Attachments").performClick()
        ui.onNodeWithContentDescription("Drop a pin").performScrollTo().performClick()
        ui.onNodeWithText("Drop a pin").assertExists()
        ui.onAllNodesWithContentDescription("Use my location").assertCountEquals(1)
        ui.onNodeWithText("Enter coordinates").assertDoesNotExist()
        ui.onNodeWithText("Caption").assertDoesNotExist()
        ui.onNodeWithContentDescription("Send place").assertIsNotEnabled()
        ui.onNodeWithTag("pin-map").performClick()
        ui.onNodeWithContentDescription("Send place").performClick()
        ui.onNodeWithText("Could not share this place. You can try again.").performScrollTo().assertIsDisplayed()
        assertNull(sent)
        val currentHeader = ui.onNodeWithTag("conversation-header").fetchSemanticsNode()
        assertEquals(header.id, currentHeader.id)
        assertEquals(header.boundsInRoot, currentHeader.boundsInRoot)
        ui.onNodeWithContentDescription("Send place").performClick()
        ui.waitUntil { sent != null }
        assertEquals(0, sent!!["latitude_e6"]); assertEquals(0, sent!!["longitude_e6"])
        assertEquals(true, sent!!["pin"]); assertNull(sent!!["live"]); assertNull(sent!!["accuracy_cm"])
        assertEquals("Synthetic pin", sent!!["label"])
        ui.onNodeWithTag("composer").assertIsDisplayed()
    }
    @Test fun switchingLocationModesKeepsWritingAndRequiresTheirOwnSelection() {
        var sends = 0
        val chat = ChatSummary("self", "@sam:example.test", "", "", true, emptyList(), displayName = "Sam")
        ui.setContent { CompositionLocalProvider(LocalPlacePanel provides { target, back, done ->
            PlacePanel(back, initialMode=target["location_mode"] as? String ?: "once",
                caption=target["location_caption"] as? String ?: "",
                mapContent={choose->Box(Modifier.fillMaxSize().testTag("picker-map").clickable {choose(1.0,2.0)})}) {
                sends++; done(); true
            }
        }) { SigilApp(NativeCore::palette, NativeCore::analyze,
            MessengerState(phase="connected", selected="self", chats=listOf(chat)), {_,_->}) } }
        ui.onNodeWithTag("composer").performTextInput("Keep this draft")
        ui.onNodeWithContentDescription("Attachments").performClick()
        ui.onNodeWithContentDescription("Drop a pin").performScrollTo().performClick()
        ui.onAllNodesWithContentDescription("Use my location").assertCountEquals(1)
        ui.onNodeWithTag("picker-map").performClick()
        ui.onNodeWithContentDescription("Send place").assertIsEnabled()
        ui.onNodeWithContentDescription("Back to attachments").performClick()
        ui.onNodeWithContentDescription("One-time location").performScrollTo().performClick()
        ui.onNodeWithText("One-time location").assertExists()
        ui.onAllNodesWithContentDescription("Use my location").assertCountEquals(1)
        ui.onNodeWithTag("picker-map").performClick()
        ui.onNodeWithContentDescription("Send place").assertIsNotEnabled()
        ui.onNodeWithContentDescription("Back to attachments").performClick()
        ui.onNodeWithContentDescription("Real-time location").performScrollTo().performClick()
        ui.onNodeWithText("Real-time location").assertExists()
        ui.onAllNodesWithContentDescription("Use my location").assertCountEquals(1)
        ui.onNodeWithText("1 hour").performScrollTo().performClick()
        ui.onNodeWithText("1 hour").assertIsSelected()
        ui.onNodeWithTag("picker-map").performClick()
        ui.onNodeWithContentDescription("Share live location").assertIsNotEnabled()
        ui.onNodeWithTag("composer").assertTextEquals("Keep this draft")
        assertEquals(0, sends)
    }
    @Test fun liveCardShowsRemainingTimeAndAuthorizedStopOnlyInViewer() {
        val now = System.currentTimeMillis() / 1000
        var part by mutableStateOf(MessagePart("card", "location", "Synthetic location", locationMode = "live", sampledAt = now - 120, until = now + 900, canStop = true))
        val message = ChatMessage("message", "author", "", true, "9:00", "sent", false, emptyList(), emptyList(), null, true, peer = "self")
        val commands = mutableListOf<Pair<String, Map<String, Any?>>>()
        ui.setContent { androidx.compose.material3.MaterialTheme { LocationCard(message, part, "Sam") { name, fields -> commands += name to fields } } }
        ui.onNodeWithText("waiting for an update", substring = true).assertDoesNotExist()
        ui.onNodeWithText("left", substring = true).assertExists()
        ui.onNodeWithText("Stop sharing").assertDoesNotExist()
        ui.onNodeWithContentDescription("Open Live location").performClick()
        ui.onNode(hasText("Sam") and hasAnyAncestor(isDialog())).assertIsDisplayed()
        ui.onNodeWithText("Stop sharing").performClick()
        assertEquals("location_stop", commands.single().first)
        assertEquals(mapOf("peer" to "self", "author" to "author", "message" to "message", "card" to "card"), commands.single().second)
        ui.onNodeWithContentDescription("Close map").performClick()
        ui.runOnIdle { part = part.copy(stopped = true, canStop = false) }
        ui.onNodeWithText("Sharing ended").assertIsDisplayed()
        ui.onNodeWithText("Stop sharing").assertDoesNotExist()
        ui.runOnIdle { part = part.copy(stopped = false, until = now - 1, canStop = true) }
        ui.onNodeWithText("Sharing ended").assertIsDisplayed()
        ui.onNodeWithText("Stop sharing").assertDoesNotExist()
    }
}
