package org.sigil

import kotlin.test.*
import org.junit.Test

class OperationalDashboardTest {
    private fun sample(time:Long)=OperationalSample(time,2,3,0,0,0,0,0,0,1048576,20,10,0,0,null,false,"test",33)
    @Test fun history_is_bounded_deduplicated_and_resets_on_server_clock_rewind() {
        var history=emptyList<OperationalSample>()
        repeat(200) { history=observationWindow(history,sample(1800000000L+it*15)) }
        assertEquals(120,history.size)
        assertEquals(history,observationWindow(history,history.last()))
        val later=observationWindow(history,sample(history.last().at+3600))
        assertEquals(1,later.size)
        assertEquals(listOf(sample(1700000000)),observationWindow(history,sample(1700000000)))
    }
    @Test fun storage_labels_keep_units_without_adding_overlapping_totals() {
        assertEquals("0 B",storageSize(0));assertEquals("1023 B",storageSize(1023))
        assertEquals("1.5 KiB",storageSize(1536));assertEquals("1.0 GiB",storageSize(1073741824))
        assertEquals("2027-01-15 08:00:00 UTC",observationTime(1800000000))
    }
}
