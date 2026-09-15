package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class SettingsPresentationTest {
    @get:Rule val ui = createComposeRule()

    @Test fun home_links_keep_real_destinations_and_contact_code() {
        var destination = ""
        ui.setContent { MaterialTheme {
            Box(Modifier.size(390.dp, 740.dp)) {
                SettingsPage(MessengerState(profileName = "Sam", address = "@sam:example.test")) { destination = it }
            }
        } }
        for ((label, route) in listOf("Profile" to "profile", "Privacy" to "privacy", "Devices" to "device",
            "Appearance" to "appearance", "About" to "about")) {
            ui.onNodeWithText(label).performScrollTo().performClick()
            assertEquals(route, destination)
        }
        ui.onNodeWithContentDescription("My contact code").performScrollTo().performClick()
        assertEquals("contact-code", destination)
    }

    @Test fun privacy_rows_change_account_preferences_and_disable_while_busy() {
        var state by mutableStateOf(MessengerState(readReceipts = true, typingIndicators = true, presenceSharing = true, allowRequests = true))
        val commands = mutableListOf<Pair<String, Map<String, Any?>>>()
        ui.setContent { MaterialTheme {
            Box(Modifier.size(390.dp, 740.dp)) {
                PersonalPage("privacy", state, { name, fields ->
                    if (fields.isNotEmpty()) commands += name to fields
                    if (name == "organize") state = state.copy(readReceipts = false)
                }, {})
            }
        } }
        ui.onNodeWithText("Let contacts see when you read messages").performClick()
        ui.onNodeWithContentDescription("Read receipts").assertIsOff()
        assertEquals("organize" to mapOf("peer" to null, "value" to mapOf("ReadReceipts" to false)), commands.single())
        ui.runOnIdle { state = state.copy(busy = true) }
        ui.onNodeWithContentDescription("Typing indicators").assertIsNotEnabled()
        ui.onNodeWithContentDescription("Allow message requests").assertIsNotEnabled()
        ui.onNodeWithContentDescription("Share activity status").assertIsNotEnabled()
    }

    @Test fun appearance_rows_keep_all_existing_sections() {
        var destination = ""
        ui.setContent { MaterialTheme {
            Box(Modifier.size(390.dp, 740.dp)) {
                AppearancePage(Appearance(), { it }, false, {}, navigate = { destination = it }) {}
            }
        } }
        for ((label, route) in listOf("Colors & backgrounds" to "appearance-colors", "Typography" to "appearance-type",
            "Layout" to "appearance-layout", "Motion & media" to "appearance-media", "Dice, coins & cards" to "appearance-objects")) {
            ui.onNodeWithText(label).performScrollTo().performClick()
            assertEquals(route, destination)
        }
    }
}
