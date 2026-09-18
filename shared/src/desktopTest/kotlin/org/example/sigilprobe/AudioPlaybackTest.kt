package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class AudioPlaybackTest {
    @get:Rule val ui=createComposeRule()
    @Test fun waveform_seeks_directly_and_exposes_accessible_progress() {
        val seeks=mutableListOf<Long>()
        var plays=0
        ui.setContent {MaterialTheme {AudioPlayback(0,10000,false,List(24){.3f},preview=true,modifier=Modifier.width(360.dp),play={plays++},seek=seeks::add)}}
        ui.onNodeWithTag("audio-time").assertTextEquals("00:10")
        ui.onNodeWithContentDescription("Play voice preview").performClick()
        assertEquals(1,plays)
        ui.onNodeWithTag("voice-preview-seek").performTouchInput {click(Offset(width*.75f,height/2f))}
        assertTrue(seeks.last() in 7400..7600)
        ui.onNodeWithTag("voice-preview-seek").performSemanticsAction(SemanticsActions.SetProgress) {assertTrue(it(5000f))}
        assertEquals(5000,seeks.last())
    }
}
