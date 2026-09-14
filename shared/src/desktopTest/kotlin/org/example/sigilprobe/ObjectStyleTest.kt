package org.sigil

import org.junit.Test
import org.junit.Assert.*

class ObjectStyleTest {
    @Test fun old_default_follows_body_and_custom_details_survive() {
        val migrated=decodeObjectStyle("7038ba,148f8a,f2d182,0.19,0.85,2.4,0.055,0.7,0.3,1,0",0)
        assertTrue(migrated.followBody)
        val red=migrated.copy(color=0xff0000).detailColor
        assertTrue((red shr 16)>((red shr 8) and 255))
        assertEquals((red shr 8) and 255,red and 255)
        val explicit=migrated.copy(second=0x148f8a,followBody=false)
        assertEquals(explicit,decodeObjectStyle(explicit.encode(),0))
        assertEquals(0x148f8a,explicit.copy(color=0xff0000).detailColor)
        val gray=migrated.copy(color=0x444444).detailColor
        assertEquals(gray shr 16,gray and 255)
    }
}
