package org.sigil

import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class RecoveryPanelsTest {
    @get:Rule val ui=createComposeRule()
    @Test fun backup_requires_saved_key_confirmation() {
        var enabled=false
        val secret="0123456789abcdef".repeat(4)
        ui.setContent {MaterialTheme {RecoverySetup(secret,false,{}, {enabled=true})}}
        ui.onNodeWithText("Enable encrypted backups").assertIsNotEnabled()
        ui.onNode(isToggleable()).performClick()
        ui.onNodeWithText("Last 8 characters of your saved key").performTextInput("00000000")
        ui.onNodeWithText("Enable encrypted backups").assertIsNotEnabled()
        ui.onNodeWithText("Last 8 characters of your saved key").performTextReplacement(secret.takeLast(8))
        ui.onNodeWithText("Enable encrypted backups").performClick()
        assertTrue(enabled)
    }
    @Test fun restore_requires_consent_and_a_complete_key() {
        var restored:String?=null
        val secret="0123456789abcdef".repeat(4)
        ui.setContent {MaterialTheme {RecoveryRestore(false,null,{}, {restored=it})}}
        ui.onNodeWithText("Recovery key").performTextInput(secret.chunked(4).joinToString(" "))
        ui.onNodeWithText("Restore history").assertIsNotEnabled()
        assertNull(restored)
        ui.onNode(isToggleable()).performClick()
        ui.onNodeWithText("Restore history").performClick()
        assertEquals(secret,restored)
    }
}
