package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.runtime.*
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
        ui.setContent { CompositionLocalProvider(LocalPlacePanel provides { target, back, done -> PlacePanel(back) { fields ->
            assertEquals("self", target["peer"])
            if (attempts++ == 0) false else { sent = fields; done(); true }
        } }) { SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", selected = "self", chats = listOf(chat)), { _, _ -> }) } }
        val header = ui.onNodeWithTag("main-header").fetchSemanticsNode()
        ui.onNodeWithContentDescription("Attachments").performClick()
        ui.onNodeWithContentDescription("Place").performClick()
        ui.onNodeWithText("Pin", useUnmergedTree = true).performClick()
        ui.onNodeWithText("Enter coordinates").performScrollTo().performClick()
        ui.onNodeWithText("Latitude").performScrollTo().performTextInput("91")
        ui.onNodeWithText("Longitude").performScrollTo().performTextInput("0")
        ui.onNodeWithText("Set pin").performScrollTo().performClick()
        ui.onNodeWithText("Send place").performScrollTo().assertIsNotEnabled()
        ui.onNodeWithText("Latitude").performScrollTo().performTextReplacement("0")
        ui.onNodeWithText("Set pin").performScrollTo().performClick()
        ui.onNodeWithText("Caption · optional").performScrollTo().performTextInput("Synthetic pin")
        ui.onNodeWithText("Send place").performScrollTo().performClick()
        ui.onNodeWithText("Could not share this place. You can try again.").performScrollTo().assertIsDisplayed()
        assertNull(sent)
        val currentHeader = ui.onNodeWithTag("main-header").fetchSemanticsNode()
        assertEquals(header.id, currentHeader.id)
        assertEquals(header.boundsInRoot, currentHeader.boundsInRoot)
        ui.onNodeWithText("Send place").performScrollTo().performClick()
        ui.waitUntil { sent != null }
        assertEquals(0, sent!!["latitude_e6"]); assertEquals(0, sent!!["longitude_e6"])
        assertEquals(true, sent!!["pin"]); assertNull(sent!!["live"]); assertNull(sent!!["accuracy_cm"])
        assertEquals("Synthetic pin", sent!!["label"])
        ui.onNodeWithTag("composer").assertIsDisplayed()
    }
    @Test fun liveCardShowsStalenessExpiryAndAuthorizedStop() {
        val now = System.currentTimeMillis() / 1000
        var part by mutableStateOf(MessagePart("card", "location", "Synthetic location", locationMode = "live", sampledAt = now - 120, until = now + 900, canStop = true))
        val message = ChatMessage("message", "author", "", true, "9:00", "sent", false, emptyList(), emptyList(), null, true, peer = "self")
        val commands = mutableListOf<Pair<String, Map<String, Any?>>>()
        ui.setContent { androidx.compose.material3.MaterialTheme { LocationCard(message, part, "Sam") { name, fields -> commands += name to fields } } }
        ui.onNodeWithText("waiting for an update", substring = true).assertExists()
        ui.onNodeWithContentDescription("Open Live location").performClick()
        ui.onNode(hasText("Synthetic location") and hasAnyAncestor(isDialog())).assertIsDisplayed()
        ui.onNodeWithContentDescription("Close map").performClick()
        ui.onNodeWithText("Stop sharing").performClick()
        assertEquals("location_stop", commands.single().first)
        assertEquals(mapOf("peer" to "self", "author" to "author", "message" to "message", "card" to "card"), commands.single().second)
        ui.runOnIdle { part = part.copy(stopped = true, canStop = false) }
        ui.onNodeWithText("Location sharing ended").assertIsDisplayed()
        ui.onNodeWithText("Stop sharing").assertDoesNotExist()
        ui.runOnIdle { part = part.copy(stopped = false, until = now - 1, canStop = true) }
        ui.onNodeWithText("Location sharing ended").assertIsDisplayed()
        ui.onNodeWithText("Stop sharing").assertDoesNotExist()
    }
}
