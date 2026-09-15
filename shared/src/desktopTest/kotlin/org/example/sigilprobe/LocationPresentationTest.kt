package org.sigil

import kotlin.test.*
import org.junit.Test

class LocationPresentationTest {
    @Test fun countdown_handles_expiry_stop_and_missing_end() {
        assertEquals("1h 1m left", locationRemaining(3661, 0, false))
        assertEquals("1:01 left", locationRemaining(61, 0, false))
        assertEquals("0:01 left", locationRemaining(61, 60, false))
        assertEquals("Sharing ended", locationRemaining(61, 61, false))
        assertEquals("Sharing ended", locationRemaining(61, 62, false))
        assertEquals("Sharing ended", locationRemaining(61, 0, true))
        assertEquals("Sharing ended", locationRemaining(null, 0, false))
    }
}
