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

@Composable
internal fun RestoreRecoveryDialog(busy: Boolean, issue: String?, dismiss: () -> Unit, restore: (String) -> Unit) {
    var secret by remember { mutableStateOf("") }
    var reviewed by remember { mutableStateOf(false) }
    AlertDialog(onDismissRequest = { if (!busy) dismiss() }, properties = DialogProperties(securePolicy = SecureFlagPolicy.SecureOn),
        title = { Text("Restore encrypted history") },
        text = { Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("Enter the recovery key you saved for this account. It stays on this device and is never sent to the server.")
            OutlinedTextField(secret, { value -> if (value.length <= 256) secret = value.filterNot { it.isWhitespace() }.lowercase() }, label = { Text("Recovery key") }, enabled = !busy, singleLine = true,
                visualTransformation = androidx.compose.ui.text.input.PasswordVisualTransformation(), keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(autoCorrectEnabled = false, keyboardType = androidx.compose.ui.text.input.KeyboardType.Password))
            Text("This device can verify the backup’s integrity, but cannot independently confirm that the server supplied the newest backup. Restore only from a server you trust.")
            Row { Checkbox(reviewed, { reviewed = it }, enabled = !busy); Text("I understand and want to restore this backup.", Modifier.padding(top = 12.dp)) }
            Text("Restoring history does not approve contacts or copy a previous device’s messaging keys.", style = MaterialTheme.typography.bodySmall)
            issue?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            if (busy) LinearProgressIndicator(Modifier.fillMaxWidth())
        } },
        confirmButton = { TextButton({ restore(secret) }, enabled = !busy && reviewed && secret.length == 64 && secret.all { it in '0'..'9' || it in 'a'..'f' }) { Text("Restore history") } },
        dismissButton = { TextButton(dismiss, enabled = !busy) { Text("Cancel") } })
}

@Composable
internal fun AccountRecoveryDialog(sso: Boolean, busy: Boolean, issue: String?, dismiss: () -> Unit, recover: (String, String?) -> Unit) {
    var method by remember { mutableStateOf(if (sso) "sso" else "invitation") }
    var invitation by remember { mutableStateOf("") }
    var confirmed by remember { mutableStateOf(false) }
    AlertDialog(onDismissRequest = { if (!busy) dismiss() }, properties = DialogProperties(securePolicy = SecureFlagPolicy.SecureOn),
        title = { Text("Recover a lost account") },
        text = { Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("Recovery signs out all previous devices and creates a new encryption identity on this phone. Your contacts will need to verify it. Your recovery key restores backed-up history after sign-in.")
            Text("If you still have a signed-in device, you can use device linking instead.", style = MaterialTheme.typography.bodySmall)
            if (sso) Row { RadioButton(method == "sso", { method = "sso" }, enabled = !busy); Text("Sign in with SSO", Modifier.padding(top = 12.dp)) }
            Row { RadioButton(method == "invitation", { method = "invitation" }, enabled = !busy); Text("Administrator recovery invitation", Modifier.padding(top = 12.dp)) }
            if (method == "invitation") {
                Text("Ask your administrator for a recovery invitation for your existing account.", style = MaterialTheme.typography.bodySmall)
                OutlinedTextField(invitation, { if (it.length <= 256) invitation = it.trim() }, label = { Text("Recovery invitation") }, singleLine = true, enabled = !busy,
                    visualTransformation = androidx.compose.ui.text.input.PasswordVisualTransformation(), keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(autoCorrectEnabled = false, keyboardType = androidx.compose.ui.text.input.KeyboardType.Password))
            }
            Row { Checkbox(confirmed, { confirmed = it }, enabled = !busy); Text("Sign out my previous devices and recover this account.", Modifier.padding(top = 12.dp)) }
            issue?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            if (busy) LinearProgressIndicator(Modifier.fillMaxWidth())
        } },
        confirmButton = { TextButton({ recover(method, invitation.takeIf { method == "invitation" }) }, enabled = confirmed && !busy && (method == "sso" || invitation.length == 64 && invitation.all { it in '0'..'9' || it in 'a'..'f' })) { Text("Continue recovery") } },
        dismissButton = { TextButton(dismiss, enabled = !busy) { Text("Cancel") } })
}
