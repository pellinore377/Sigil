package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class SyncNoticeTest {
    @get:Rule val ui=createComposeRule()
    @Test fun compact_notice_keeps_full_diagnostics_available_and_dismisses_explicitly() {
        val message="Sync: updating groups — Device storage could not complete the operation (sqlite-5). Your stored keys have not been reset."
        var dismissed=false
        ui.setContent {MaterialTheme {Box(Modifier.width(390.dp)){SyncNotice(message){dismissed=true}}}}
        ui.onNodeWithText("Sync paused: device storage is busy.").assertIsDisplayed()
        ui.onNodeWithText(message).assertDoesNotExist()
        ui.onNodeWithContentDescription("Show error details").performClick()
        ui.onNodeWithText(message).assertIsDisplayed()
        assertFalse(dismissed)
        ui.onNodeWithContentDescription("Dismiss notice").performClick()
        assertTrue(dismissed)
    }
}
