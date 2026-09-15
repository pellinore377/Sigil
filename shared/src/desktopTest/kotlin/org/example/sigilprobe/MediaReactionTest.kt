package org.sigil

import kotlin.test.*

class MediaReactionTest {
    private val message = ChatMessage("shared-photo", "sender", "", false, "9:41", "read", false, emptyList(), listOf("❤️"), null, true, peer = "conversation")
    @Test fun targetsTheSharedMessageAndTogglesOnlyMyReaction() {
        assertEquals(mapOf("peer" to "conversation", "author" to "sender", "message" to "shared-photo", "emoji" to "❤️", "active" to false), mediaReaction(message, "❤️"))
        assertEquals(true, mediaReaction(message.copy(reactions = listOf("👍")), "👍")["active"])
    }
}
