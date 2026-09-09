package org.sigil.compose

import android.content.ClipData
import android.content.ClipboardManager
import android.os.Handler
import android.os.Looper
import android.os.PersistableBundle
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy

@Composable
internal fun RecoveryDialog(secret: String, busy: Boolean, dismiss: () -> Unit, enable: () -> Unit) {
    val context = LocalContext.current
    var saved by remember(secret) { mutableStateOf(false) }
    var check by remember(secret) { mutableStateOf("") }
    AlertDialog(onDismissRequest = { if (!busy) dismiss() }, properties = DialogProperties(securePolicy = SecureFlagPolicy.SecureOn),
        title = { Text("Save your recovery key") },
        text = { Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("Keep this in your password manager or write it down somewhere safe. Your server cannot replace it. Anyone with this key and access to your backup can read that history.")
            Text(secret.chunked(4).joinToString(" "), fontFamily = org.sigil.LocalCodeFont.current)
            TextButton({
                val clipboard = context.getSystemService(ClipboardManager::class.java)
                val clip = ClipData.newPlainText("Sigil recovery key", secret)
                clip.description.extras = PersistableBundle().apply { putBoolean("android.content.extra.IS_SENSITIVE", true) }
                clipboard.setPrimaryClip(clip)
                Handler(Looper.getMainLooper()).postDelayed({
                    if (clipboard.primaryClip?.getItemAt(0)?.text?.toString() == secret) clipboard.clearPrimaryClip()
                }, 60_000)
            }) { Text("Copy for one minute") }
            Row { Checkbox(saved, { saved = it }, enabled = !busy); Text("I saved my recovery key.", Modifier.padding(top = 12.dp)) }
            OutlinedTextField(check, { if (it.length <= 8) check = it }, label = { Text("Last 8 characters of your saved key") }, singleLine = true, enabled = !busy)
        } },
        confirmButton = { TextButton(enable, enabled = saved && check == secret.takeLast(8) && !busy) { Text("Enable encrypted backups") } },
        dismissButton = { TextButton(dismiss, enabled = !busy) { Text("Cancel") } })
}
