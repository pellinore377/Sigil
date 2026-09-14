package org.sigil

import kotlin.test.*
import kotlinx.serialization.json.*

class StateDecoderTest {
    @Test fun requestsAndReceiptOwnershipSurviveBrowserPresentation() {
        val state=StateDecoder.state(Json.parseToJsonElement("""{"phase":"connected","address":"@alice:example.test","chats":[{"id":"peer","address":"@bob:example.test","request":"incoming","verified":false,"identity_review":"review"}]}""").jsonObject,MessengerState(),{it.toString()})
        assertEquals("incoming",state.chats.single().request)
        assertEquals("review",state.chats.single().identityReview)
        assertFalse(state.chats.single().verified)
        val messages=StateDecoder.messages(Json.parseToJsonElement("""{"messages":[{"id":"m","author":"a","text":"Visible text","mine":false,"timestamp":1,"read_by_me":true,"attachment":null,"parts":[]}]}""").jsonObject,"peer",{it.toString()})
        assertTrue(messages.single().readByMe)
        assertFalse(messages.single().mine)
        assertNull(messages.single().attachment)
    }
}
