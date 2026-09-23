package org.sigil.compose

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Handler
import android.os.Looper
import android.os.PersistableBundle

/** Marks the clip sensitive and clears it after a minute if it is still ours. */
internal fun copySecret(context: Context, secret: String) {
    val clipboard = context.getSystemService(ClipboardManager::class.java)
    val clip = ClipData.newPlainText("Sigil recovery code", secret)
    clip.description.extras = PersistableBundle().apply { putBoolean("android.content.extra.IS_SENSITIVE", true) }
    clipboard.setPrimaryClip(clip)
    Handler(Looper.getMainLooper()).postDelayed({ if (clipboard.primaryClip?.getItemAt(0)?.text?.toString() == secret) clipboard.clearPrimaryClip() }, 60_000)
}
