package org.sigil

import kotlin.test.*
import org.junit.Test

class VoiceDraftTargetTest {
    @Test fun capture_retains_reply_thread_until_explicit_clear() {
        val draft=VoiceDraftTarget()
        val source=mutableMapOf<String,Any?>("peer" to "peer","reply_author" to "author","reply_message" to "quoted","thread_author" to "author","thread_message" to "root","caption" to "old caption")
        draft.start(source)
        source["peer"]="another conversation"
        assertEquals("peer",draft.fields["peer"])
        assertEquals("root",draft.fields["thread_message"])
        assertEquals("quoted",draft.fields["reply_message"])
        assertFalse("caption" in draft.fields)
        draft.clear()
        assertTrue(draft.fields.isEmpty())
        draft.start(mapOf("peer" to "next"))
        assertEquals(mapOf("peer" to "next"),draft.fields)
    }
}
