package org.sigil

/** Only a completed attempt from the same worker resolves its notice. */
class WorkNotices {
    private val failures = linkedMapOf<String, String>()
    fun update(current: String?, source: String, message: String?): String? {
        val previous = failures.remove(source)
        if (message != null) {
            failures[source] = message
            return message
        }
        return if (previous != null && current == previous) failures.values.lastOrNull() else current
    }
    fun dismiss() { failures.clear() }
}

fun fileWorkNotice(source: String, message: String?): String? = message?.let {
    val label = when (source) {
        "attachments" -> "Attachment transfer"
        "backup" -> "Backup sync"
        "maintenance" -> "Storage maintenance"
        "recovery_media" -> "Backup attachments"
        "push" -> "Notification registration"
        else -> "Background work"
    }
    "$label: $it"
}
