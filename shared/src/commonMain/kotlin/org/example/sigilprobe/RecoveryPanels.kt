package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.ClipboardManager
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties
import kotlinx.coroutines.*
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject

// Platforms may secure the window that shows a secret and copy it as sensitive.
class SecretSurface(val properties: DialogProperties = DialogProperties(), val copy: ((String) -> Unit)? = null)
val LocalSecretSurface = staticCompositionLocalOf { SecretSurface() }

internal fun recoveryCode(text: String) = text.filterNot(Char::isWhitespace)

@Composable internal fun RecoveryCodeField(code: String, enabled: Boolean, update: (String) -> Unit, done: () -> Unit) {
    TextField(code, { if (it.length <= 160) update(it) }, Modifier.fillMaxWidth().testTag("recovery-code"), placeholder = { Text("Recovery code") }, singleLine = true, enabled = enabled,
        shape = RoundedCornerShape(14.dp), colors = quietFieldColors(), textStyle = LocalTextStyle.current.copy(fontFamily = LocalCodeFont.current),
        keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Characters, autoCorrectEnabled = false, keyboardType = KeyboardType.Password, imeAction = ImeAction.Done),
        keyboardActions = KeyboardActions(onDone = { done() }))
}

@Composable internal fun StartOverDialog(busy: Boolean, dismiss: () -> Unit, confirm: () -> Unit) {
    AlertDialog(onDismissRequest = { if (!busy) dismiss() }, icon = { Glyph("restart_alt") }, title = { Text("Start over with a new identity?") },
        text = { Text("Your contacts will have to accept your new identity before you can message them again, and your old devices will be signed out.") },
        confirmButton = { SigilTextButton(confirm, enabled = !busy) { Text("Start over", color = MaterialTheme.colorScheme.error) } },
        dismissButton = { SigilTextButton(dismiss, enabled = !busy) { Text("Cancel") } })
}

// Clears outlive the dialog, so the minute holds after it closes.
private val clipboardScope by lazy { CoroutineScope(SupervisorJob() + Dispatchers.Main) }
private fun copyForAMinute(clipboard: ClipboardManager, code: String) {
    clipboard.setText(AnnotatedString(code))
    clipboardScope.launch { delay(60_000); if (clipboard.getText()?.text == code) clipboard.setText(AnnotatedString("")) }
}

@Composable internal fun RecoveryCodeDialog(close: () -> Unit) {
    val access = LocalServiceAccess.current
    val secret = LocalSecretSurface.current
    val clipboard = LocalClipboardManager.current
    var code by remember { mutableStateOf<String?>(null) }
    var failed by remember { mutableStateOf(false) }
    var copied by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        code = runCatching { access?.invoke("{\"command\":\"recovery_code\"}")?.json?.let { Json.parseToJsonElement(it).jsonObject.optional("code") } }.getOrNull()
        failed = code == null
    }
    AlertDialog(onDismissRequest = close, properties = secret.properties, title = { Text("Recovery code") }, text = {
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("Anyone with this code can recover your account. Keep it in a password manager or write it down somewhere safe.", style = MaterialTheme.typography.bodyMedium)
            when {
                code != null -> Text(code!!, Modifier.testTag("recovery-code-value"), fontFamily = LocalCodeFont.current, style = MaterialTheme.typography.bodyLarge)
                failed -> Text("Couldn’t show the code. Try again.", Modifier.semantics { liveRegion = LiveRegionMode.Polite }, color = MaterialTheme.colorScheme.error)
                else -> LinearProgressIndicator(Modifier.fillMaxWidth())
            }
            code?.let { value -> SigilTextButton({ secret.copy?.invoke(value) ?: copyForAMinute(clipboard, value); copied = true }) { Text(if (copied) "Copied for one minute" else "Copy for one minute") } }
        }
    }, confirmButton = { SigilTextButton(close) { Text("Done") } })
}

internal fun addedOn(seconds: Long): String {
    val (year, month, day) = civilFromDays(seconds.floorDiv(86_400L))
    return "Added $day ${listOf("Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec")[month - 1]} $year"
}

@Composable internal fun AccountRecoverySettings(state: MessengerState, command: Command) {
    val recovery = state.accountRecovery
    val passkeys = recovery?.passkeys.orEmpty()
    var removing by remember { mutableStateOf<RecoveryPasskey?>(null) }
    var revealing by remember { mutableStateOf(false) }
    removing?.let { key -> AlertDialog(onDismissRequest = { removing = null }, title = { Text("Remove this passkey?") },
        text = { Text(if (passkeys.size == 1) "It’s your only passkey. Without it, only a recovery code can bring your account back." else "${key.label.ifBlank { "This passkey" }} will no longer recover your account.") },
        confirmButton = { SigilTextButton({ command("passkey_remove", mapOf("credential" to key.id)); removing = null }, enabled = !state.busy) { Text("Remove passkey") } },
        dismissButton = { SigilTextButton({ removing = null }) { Text("Cancel") } }) }
    if (revealing) RecoveryCodeDialog { revealing = false }
    SettingsNote("Get your conversations back on a new phone or computer. To sign out an old device, use Devices.")
    if (recovery?.ready == false) SettingsNote("Recovery is still being set up.")
    SettingsGroupLabel("Passkeys")
    if (passkeys.isEmpty()) SettingsNote("No passkeys yet.")
    else SettingsGroup(*passkeys.map { key -> @Composable {
        Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(start = 12.dp, top = 8.dp, bottom = 8.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(16.dp)) {
            CompositionLocalProvider(LocalContentColor provides MaterialTheme.colorScheme.onSurfaceVariant) { Glyph("passkey", 24) }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(key.label.ifBlank { "Passkey" }, style = MaterialTheme.typography.titleMedium)
                Text(addedOn(key.created), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            SigilTextButton({ removing = key }, enabled = !state.busy) { Text("Remove") }
        }
    } }.toTypedArray())
    if (LocalClientFeatures.current.passkeys) SigilButton({ command("passkey_create", emptyMap()) }, enabled = !state.busy) { Text("Add a passkey") }
    else SettingsNote("Passkeys aren’t available here. A recovery code works everywhere.")
    SettingsGroupLabel("Recovery code")
    SettingsNote("An optional backup for when no passkey is at hand. Anyone with it can recover your account.")
    SigilTextButton({ revealing = true }, enabled = !state.busy) { Text("Show recovery code") }
}
