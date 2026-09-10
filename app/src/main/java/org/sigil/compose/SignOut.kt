package org.sigil.compose

import org.sigil.SigilTextButton

import android.app.ActivityManager
import android.content.Context
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

internal object NativeSignOut {
    private fun preferences(context: Context) = context.getSharedPreferences("sign_out", Context.MODE_PRIVATE)
    fun stage(context: Context): String = preferences(context).getString("stage", "").orEmpty()
    fun pending(context: Context) = stage(context).isNotEmpty()
    fun save(context: Context, stage: String) {
        check(preferences(context).edit().putString("stage", stage).commit())
        context.stopService(android.content.Intent(context, LocationService::class.java))
    }
    fun erase(context: Context): Boolean = context.getSystemService(ActivityManager::class.java).clearApplicationUserData()
}

@Composable
internal fun SignOutDialog(stage: String, busy: Boolean, issue: String?, command: (String) -> Unit) {
    var saved by remember { mutableStateOf(false) }
    var localOnly by remember { mutableStateOf(false) }
    val confirming = stage == "confirm"
    AlertDialog(onDismissRequest = { if (confirming && !busy) command("cancel") },
        title = { Text(if (confirming) "Sign out of this device?" else "Finish signing out") },
        text = { Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("This removes Sigil’s messages, drafts, downloaded media, keys and settings from this phone. Other devices and your account remain. The app will close; reopen it to sign in.")
            if (confirming) {
                Text("To restore history, you need a completed encrypted backup and its recovery key. Unsynced changes will be lost.")
                Row { Checkbox(saved, { saved = it }, enabled = !busy); Text("I have saved what I need, or accept losing this device’s history.", Modifier.padding(top = 12.dp)) }
            } else if (stage == "confirmed") Text("The server confirmed this device’s revocation. Local data still needs to be removed.")
            else if (!busy) {
                Text("Server revocation is not confirmed. Retry, or remove local data and revoke this device from another signed-in device.")
                Row { Checkbox(localOnly, { localOnly = it }); Text("Remove local data without server confirmation.", Modifier.padding(top = 12.dp)) }
            }
            issue?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            if (busy) { LinearProgressIndicator(Modifier.fillMaxWidth()); Text("Signing out…") }
        } },
        confirmButton = { SigilTextButton({ command(if (confirming) "confirm" else if (stage == "confirmed" || localOnly) "erase" else "retry") }, enabled = !busy && (!confirming || saved)) { Text(if (confirming) "Sign out" else if (stage == "confirmed" || localOnly) "Remove local data" else "Retry revocation") } },
        dismissButton = { if (confirming) SigilTextButton({ command("cancel") }, enabled = !busy) { Text("Cancel") } })
}
