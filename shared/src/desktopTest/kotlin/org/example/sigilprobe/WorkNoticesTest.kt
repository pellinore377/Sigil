package org.sigil

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class WorkNoticesTest {
    @Test fun recovery_is_specific_to_the_failed_worker() {
        val notices = WorkNotices()
        var shown = notices.update(null, "attachments", "Upload failed")
        shown = notices.update(shown, "sync", null)
        assertEquals("Upload failed", shown)
        shown = notices.update(shown, "backup", "Backup failed")
        shown = notices.update(shown, "backup", null)
        assertEquals("Upload failed", shown)
        assertNull(notices.update(shown, "attachments", null))
    }
    @Test fun recovery_preserves_newer_action_errors_and_dismissal_does_not_resurrect_old_notices() {
        val notices = WorkNotices()
        notices.update(null, "sync", "Offline")
        assertEquals("Camera denied", notices.update("Camera denied", "sync", null))
        var shown = notices.update(null, "attachments", "Upload failed")
        shown = notices.update(shown, "backup", "Backup failed")
        notices.dismiss()
        assertNull(notices.update(null, "backup", null))
        assertEquals("Upload failed", notices.update(null, "attachments", "Upload failed"))
    }
}
