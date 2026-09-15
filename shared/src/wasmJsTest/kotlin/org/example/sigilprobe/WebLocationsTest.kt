@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)
package org.sigil

import kotlin.test.*

class WebLocationsTest {
    @Test fun once_and_pin_do_not_stop_an_existing_live_share() {
        val locations=WebLocations()
        locations.shared("live",true)
        locations.shared("once",true)
        assertTrue(locations.live)
        locations.shared("pin",true)
        assertTrue(locations.live)
        assertEquals(3,locations.completion)
    }
    @Test fun hidden_completion_does_not_restart_capture_and_schedules_cleanup() {
        val locations=WebLocations()
        locations.shared("live",false)
        assertFalse(locations.live)
        assertEquals(1,locations.completion)
        assertNull(locations.point)
    }
}
