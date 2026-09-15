package org.sigil

import org.junit.Test
import kotlin.test.assertEquals

class ForegroundSyncTest {
    @Test fun foreground_wait_tracks_deadline_without_zero_delay_or_overflow() {
        assertEquals(250, foregroundSyncWait(2, 1750))
        assertEquals(50, foregroundSyncWait(2, 1999))
        assertEquals(50, foregroundSyncWait(1, 1750))
        assertEquals(1000, foregroundSyncWait(Long.MAX_VALUE, 1750))
        assertEquals(250, foregroundSyncWait(10, 1750, 250))
        assertEquals(100, foregroundSyncWait(2, 1900, 250))
    }
}
