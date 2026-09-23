package org.sigil

import androidx.compose.runtime.*
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import org.junit.Rule
import org.junit.Test

class CallControlsTest {
    @get:Rule val ui = createComposeRule()
    private val call = ActiveCall(CallSummary("call-1", "active", true, 0, listOf(
        CallParticipant("self", "self", "You", true, true, true, true, false),
        CallParticipant("m1", "maya-id", "Maya", false, true, true, false, false))), "Maya", connection = "connected", camera = true)
    private fun page(features: ClientFeatures) = ui.setContent {
        SigilTheme(Appearance(), palette = NativeCore::palette) { CompositionLocalProvider(LocalClientFeatures provides features) { CallPage(call, emptyList(), { _, _ -> }, panel = "", setPanel = {}) } }
    }
    @Test fun platformWithoutRoutingOrScreenCaptureHidesThoseControls() {
        page(ClientFeatures(screenShare = false, audioRoute = false))
        ui.onNodeWithText("Mute").assertExists()
        ui.onNodeWithText("Speaker").assertDoesNotExist()
        ui.onNodeWithText("Share").assertDoesNotExist()
    }
    @Test fun defaultPlatformKeepsRoutingAndScreenShare() {
        page(ClientFeatures())
        ui.onNodeWithText("Speaker").assertExists()
        ui.onNodeWithText("Share").assertExists()
    }
}
