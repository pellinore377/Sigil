package org.sigil

internal class VoiceDraftTarget {
    var fields:Map<String,Any?> = emptyMap()
        private set
    fun start(target:Map<String,Any?>) {
        fields=target.filterKeys {it in setOf("peer","reply_author","reply_message","thread_author","thread_message")}.toMap()
    }
    fun clear() {fields=emptyMap()}
}
