@file:OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
package org.sigil

import androidx.compose.foundation.lazy.layout.LazyLayoutCacheWindow
import androidx.compose.ui.unit.Density
import kotlin.math.roundToInt
import kotlin.time.TimeSource

/// Items the list shows before the messages; the viewport depth it reports is a message depth.
internal const val TimelineLead = 1
/// How long after arriving a message is still new enough to rise into place.
internal const val ArrivalGraceMillis = 700L

private fun elapsedMillis(): () -> Long { val from = TimeSource.Monotonic.markNow(); return { from.elapsedNow().inWholeMilliseconds } }

/// Messages that arrived at the head of a live list; a first load and older history carry no arrival motion.
/// A key is claimed once, and only while it is still new: a message that arrived screens away from the
/// reader is simply there when they scroll to it rather than replaying its rise on the way past.
internal class TimelineArrivals(private val now: () -> Long = elapsedMillis()) {
    private var known = emptySet<String>()
    private var started = false
    private val fresh = LinkedHashMap<String, Long>()
    fun update(keys: List<String>, loaded: Boolean, live: Boolean) {
        if (!loaded) return
        val at = now()
        if (started && live) keys.takeWhile { it !in known }.forEach { fresh.getOrPut(it) { at } }
        fresh.entries.retainAll { at - it.value <= ArrivalGraceMillis }
        while (fresh.size > 256) fresh.remove(fresh.keys.first())
        known = keys.toSet()
        started = true
    }
    /// True the first time a message that arrived while the reader was watching is composed, false ever after.
    fun claim(key: String): Boolean { val at = fresh.remove(key) ?: return false; return now() - at <= ArrivalGraceMillis }
    internal fun pending() = fresh.keys.toSet()
}

/// How deep the list keeps items composed on either side of the viewport; the core owns the depth.
/// One screen either way until the core has answered, which is before the list holds anything to compose.
internal class TimelineCacheWindow(private val depth: () -> TimelineBufferDepth?) : LazyLayoutCacheWindow {
    override fun Density.calculateAheadWindow(viewport: Int) = ((depth()?.ahead ?: 1f) * viewport).roundToInt()
    override fun Density.calculateBehindWindow(viewport: Int) = ((depth()?.behind ?: 1f) * viewport).roundToInt()
}

/// Whether a scan that holds `held` messages may publish them. It waits for what the reader can already see,
/// but never past the target: a target below what is on screen means the reader left that history behind.
fun timelinePublishes(held: Int, onScreen: Int, want: Int, last: Boolean) = last || held >= minOf(onScreen, want)

/// The index the reader's anchor moved to, or null when it is still where it was.
internal fun anchorPlace(keys: List<String>, previous: List<String>, anchor: String?): Int? {
    if (anchor == null) return null
    val places = keys.withIndex().associate { (index, key) -> key to index }
    places[anchor]?.let { return it }
    val from = previous.indexOf(anchor)
    if (from < 0) return null
    // The anchor itself is gone: hold on to the nearest message that survived it.
    for (step in 1 until previous.size) {
        places[previous.getOrNull(from + step)]?.let { return it }
        places[previous.getOrNull(from - step)]?.let { return it }
    }
    return null
}

/// Keeps the reader's place when the message list changes under them.
internal class TimelineAnchor {
    private var keys = emptyList<String>()
    private var key: String? = null
    private var offset = 0
    fun record(key: Any?, offset: Int) { this.key = key as? String; this.offset = offset }
    /// Item index and offset to restore, or null when the anchor did not move.
    /// `lead` counts the items the list shows before the messages.
    fun settle(next: List<String>, lead: Int): Pair<Int, Int>? {
        val was = keys.indexOf(key)
        val place = anchorPlace(next, keys, key)
        keys = next
        return place?.takeIf { it != was }?.let { (it + lead) to offset }
    }
}
