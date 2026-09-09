package org.sigil.compose

import android.os.Bundle
import androidx.activity.ComponentActivity
import org.sigil.*

internal fun timelineFixture(): MessengerState {
    val chat = ChatSummary("peer", "@letters:example.com", "", "", true, emptyList(), displayName = "Letters", group = true)
    val messages = (1000 downTo 1).map { n -> ChatMessage(n.toString(), if (n % 3 == 0) "me" else "sam", "Letter $n — A little correspondence.\nA second line of synthetic text.", n % 3 == 0, "9:33am", "Delivered", n % 13 == 0, if (n % 7 == 0) listOf("👍") else emptyList(), emptyList(), null, true, timestamp = 1000L + n) }
    return MessengerState(phase = "connected", chats = listOf(chat), selected = "peer", messages = messages, people = mapOf("sam" to "Sam"))
}
class TimelineFixtureActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        check(packageName.endsWith(".acceptance"))
        val state = timelineFixture()
        setSigilContent { SigilApp(NativeCore::palette, NativeCore::analyze, state, { _, _ -> }) }
    }
}
