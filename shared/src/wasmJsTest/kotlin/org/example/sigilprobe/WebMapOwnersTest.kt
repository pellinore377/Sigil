package org.sigil

import kotlin.test.*

class WebMapOwnersTest {
    @Test fun expandedMapTakesOwnershipAndUnderlyingMapResumesOnClose() {
        val owners=WebMapOwners();val composer=Any();val expanded=Any()
        owners.attach(composer);assertSame(composer,owners.current)
        owners.attach(expanded);assertSame(expanded,owners.current)
        owners.attach(composer);assertSame(expanded,owners.current)
        owners.detach(expanded);assertSame(composer,owners.current)
        owners.detach(composer);assertNull(owners.current)
    }
    @Test fun UnderlyingMapDisposalDoesNotReleaseExpandedMap() {
        val owners=WebMapOwners();val composer=Any();val expanded=Any()
        owners.attach(composer);owners.attach(expanded);owners.detach(composer)
        assertSame(expanded,owners.current)
        owners.detach(expanded);assertNull(owners.current)
    }
}
