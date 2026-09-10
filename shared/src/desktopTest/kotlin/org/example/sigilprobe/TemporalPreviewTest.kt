package org.sigil

import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import org.junit.Rule
import org.junit.Test
import java.util.Locale
import kotlin.test.*

class TemporalPreviewTest {
    @get:Rule val ui=createComposeRule()
    @Test fun native_dates_use_locale_and_show_the_authoritative_offset() {
        val now=1772945999L
        val us=temporalPreview("Reminder","07/05/27 9:30am",now,"America/New_York",Locale.US)!!
        val gb=temporalPreview("Reminder","07/05/27 9:30am",now,"America/New_York",Locale.UK)!!
        assertEquals("2027-07-05T09:30:00",us.source)
        assertEquals("2027-05-07T09:30:00",gb.source)
        assertTrue(us.label.contains("UTC-04:00"))
        val gap=temporalPreview("Reminder","tomorrow 2:30am",now,"America/New_York",Locale.US)!!
        assertEquals("2026-03-08T03:30:00",gap.source)
        assertTrue(gap.label.contains("3:30"))
        assertEquals("1 minute 30 seconds",temporalPreview("Timer","1m 30s",now,"UTC",Locale.US)!!.label)
        assertNull(temporalPreview("Reminder","not a date",now,"UTC",Locale.US))
    }
    @Test fun a_changed_or_invalid_date_cannot_send_the_previous_preview() {
        val requests=mutableListOf<Pair<String,String?>>()
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalTemporalPreview provides {kind,input->temporalPreview(kind,input,1772945999,"America/New_York",Locale.US)}) {
            StructuredBuilder("Reminder",true,{}, {source,zone->requests+=source to zone})
        }}}
        ui.onNodeWithText("Title").performTextInput("Synthetic reminder")
        ui.waitUntil(3000) {ui.onAllNodes(hasText("UTC-04:00",substring=true)).fetchSemanticsNodes().isNotEmpty()}
        ui.onNodeWithText("Send").assertIsEnabled()
        ui.onNodeWithText("When").performTextReplacement("not a date")
        ui.onNodeWithText("Send").assertIsNotEnabled()
        ui.waitUntil(3000) {ui.onAllNodes(hasText("Use a date",substring=true)).fetchSemanticsNodes().isNotEmpty()}
        ui.onNodeWithText("Send").assertIsNotEnabled()
        ui.onNodeWithText("When").performTextReplacement("07/05/27 9:30am")
        ui.waitUntil(3000) {ui.onAllNodes(hasText("Jul 5, 2027",substring=true)).fetchSemanticsNodes().isNotEmpty()}
        ui.onNodeWithText("Send").performClick()
        assertEquals(listOf<Pair<String,String?>>("remind::2027-07-05T09:30:00::Synthetic reminder;" to "America/New_York"),requests)
    }
}
