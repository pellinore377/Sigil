package org.sigil.compose

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Handler
import android.os.Looper
import android.os.PersistableBundle
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy
import org.sigil.LocalCodeFont
import org.sigil.SigilTextButton

@Composable
internal fun RecoveryCodeDialog(code: String, dismiss: () -> Unit) {
    val context = LocalContext.current
    AlertDialog(dismiss, properties = DialogProperties(securePolicy = SecureFlagPolicy.SecureOn), title = { Text("Recovery code") }, text = {
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("Keep this code in your password manager or somewhere safe. With it you can recover this account on a new device. Your server cannot replace it.", style = MaterialTheme.typography.bodyMedium)
            DisableSelection { Text(code, Modifier.testTag("recovery-code"), fontFamily = LocalCodeFont.current, style = MaterialTheme.typography.bodyLarge) }
        }
    }, confirmButton = { SigilTextButton(dismiss) { Text("Done") } }, dismissButton = { SigilTextButton({ copySecret(context, code) }) { Text("Copy for one minute") } })
}

/** Marks the clip sensitive and clears it after a minute if it is still ours. */
internal fun copySecret(context: Context, secret: String) {
    val clipboard = context.getSystemService(ClipboardManager::class.java)
    val clip = ClipData.newPlainText("Sigil recovery code", secret)
    clip.description.extras = PersistableBundle().apply { putBoolean("android.content.extra.IS_SENSITIVE", true) }
    clipboard.setPrimaryClip(clip)
    Handler(Looper.getMainLooper()).postDelayed({ if (clipboard.primaryClip?.getItemAt(0)?.text?.toString() == secret) clipboard.clearPrimaryClip() }, 60_000)
}
