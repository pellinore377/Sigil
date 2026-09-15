package org.sigil

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class CameraViewfinderTest {
    @get:Rule val ui=createComposeRule()
    @Test fun controls_overlay_the_full_preview_and_capture_requires_explicit_action() {
        var photo by mutableStateOf(false)
        var captured=0
        ui.setContent {MaterialTheme {Box(Modifier.width(412.dp).height(420.dp).testTag("camera-frame")) {
            CameraViewfinder(Modifier.fillMaxSize(),photo=photo,ready=true,busy=false,issue=null,close={},capture={captured++;photo=true},retake={photo=false},flip={}) {
                Box(Modifier.matchParentSize().background(Color.Gray).testTag("synthetic-camera"))
            }
        }}}
        val preview=ui.onNodeWithTag("synthetic-camera").fetchSemanticsNode().boundsInRoot
        val close=ui.onNodeWithContentDescription("Back to attachments").fetchSemanticsNode().boundsInRoot
        val shutter=ui.onNodeWithContentDescription("Take photo").fetchSemanticsNode().boundsInRoot
        val flip=ui.onNodeWithContentDescription("Switch camera").fetchSemanticsNode().boundsInRoot
        assertEquals(ui.onNodeWithTag("camera-frame").fetchSemanticsNode().boundsInRoot.height,preview.height,1f)
        assertTrue(close.top>=preview.top && close.bottom<shutter.top)
        assertTrue(shutter.bottom<=preview.bottom && shutter.center.x<flip.center.x)
        assertEquals(0,captured)
        ui.onNodeWithContentDescription("Take photo").performClick()
        ui.onNodeWithContentDescription("Retake").assertIsDisplayed()
        ui.onNodeWithContentDescription("Take photo").assertDoesNotExist()
        assertEquals(1,captured)
    }
}
