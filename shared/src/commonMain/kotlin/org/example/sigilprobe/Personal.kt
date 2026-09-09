package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay

@Composable
internal fun NewConversation(state: MessengerState, command: Command, back: () -> Unit, open: (String) -> Unit, titleChanged: (String) -> Unit = {}) {
    var query by remember { mutableStateOf("") }
    var selected by remember { mutableStateOf(listOf<String>()) }
    var creatingGroup by remember { mutableStateOf(false) }
    SideEffect { titleChanged(if (creatingGroup) "New group" else "New conversation") }
    BackAction(creatingGroup) { creatingGroup = false }
    var title by remember { mutableStateOf("") }
    var description by remember { mutableStateOf("") }
    LaunchedEffect(query) { if (query.startsWith("@") && query.contains(':')) { delay(650); command("find", mapOf("address" to query.trim())) } }
    Column(Modifier.fillMaxSize().imePadding()) {
        Header(if (creatingGroup) "New group" else "New conversation", { if (creatingGroup) creatingGroup = false else back() })
        if (creatingGroup) Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(24.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            OutlinedTextField(title, { title = it }, Modifier.fillMaxWidth(), label = { Text("Group name") })
            OutlinedTextField(description, { description = it }, Modifier.fillMaxWidth(), label = { Text("Description · optional") })
            Text("${selected.size} people will receive an invitation.")
            val unverified = state.chats.filter { it.id in selected && !it.verified }
            unverified.forEach { person ->
                Text(person.name, style = MaterialTheme.typography.titleMedium)
                ContactRequestPanel(person, state.busy, command)
            }
            SigilButton({ command("group_create", mapOf("name" to title.trim(), "description" to description, "peers" to selected)) }, enabled = title.isNotBlank() && unverified.isEmpty() && !state.busy) { Text("Create group") }
        } else {
            if (selected.isNotEmpty()) LazyRow(contentPadding = PaddingValues(horizontal = 20.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                items(selected) { id -> InputChip(true, { selected = selected - id }, { Text(state.chats.find { it.id == id }?.name ?: "Note to Self") }, trailingIcon = { Glyph("close", 15) }) }
            }
            OutlinedTextField(query, { query = it }, Modifier.fillMaxWidth().padding(20.dp), label = { Text("Name or user address") }, placeholder = { Text("@someone:example.com") }, singleLine = true, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, autoCorrectEnabled = false))
            LazyColumn(Modifier.weight(1f)) {
                item { SettingRow("edit_note", "Note to Self", "A private space for your own thoughts") { selected = if ("self" in selected) selected - "self" else listOf("self") } }
                items(state.chats.filter { it.id != "self" && !it.group && (it.name.contains(query, true) || it.address.contains(query, true)) }, key = { it.id }) { chat ->
                    ChatRow(chat, chat.id in selected, open = { selected = if (chat.id in selected) selected - chat.id else selected.filter { it != "self" } + chat.id })
                }
            }
            SigilButton({ if (selected.size == 1) open(selected.single()) else creatingGroup = true }, Modifier.align(Alignment.End).padding(20.dp), enabled = selected.isNotEmpty()) { Text("Next") }
        }
    }
}
@Composable
internal fun SettingsPage(state: MessengerState, navigate: (String) -> Unit) {
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
        Row(Modifier.fillMaxWidth().clickable { navigate("profile") }.padding(24.dp), verticalAlignment = Alignment.CenterVertically) {
            Avatar(state.profileName.ifEmpty { state.address.removePrefix("@") }, 64, state.profileAvatar)
            Column(Modifier.weight(1f).padding(start = 16.dp)) { Text(state.profileName.ifEmpty { state.address.substringBefore(':').removePrefix("@") }, style = MaterialTheme.typography.headlineSmall); Text(state.address, style = MaterialTheme.typography.bodySmall) }
            Symbol("qr_code", "My contact code") { navigate("contact-code") }
        }
        SettingsSection("Account") {
            SettingRow("person", "Profile", "Display name and photo") { navigate("profile") }
            SettingRow("lock", "Privacy", "Read receipts, typing, and who can reach you") { navigate("privacy") }
            SettingRow("devices", "Devices", "Linked devices and verification") { navigate("device") }
            SettingRow("notifications", "Notifications", "Messages, calls, and sounds") { navigate("notifications") }
        }
        SettingsSection("Appearance") { SettingRow("palette", "Appearance", "Theme, typography, and layout") { navigate("appearance") } }
        SettingsSection("Data and storage") { SettingRow("database", "Data and storage", "Media, downloads, and cache") { navigate("storage") } }
        SettingsSection("Help") { SettingRow("info", "About", "Version, licenses, and support") { navigate("about") } }
        Spacer(Modifier.height(24.dp))
    }
}
@Composable
private fun SettingsSection(title: String, body: @Composable () -> Unit) {
    HorizontalDivider(Modifier.padding(horizontal = 24.dp), color = MaterialTheme.colorScheme.outlineVariant)
    Text(title, Modifier.padding(start = 24.dp, top = 12.dp), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    body()
}
@Composable
internal fun PersonalPage(page: String, state: MessengerState, command: Command, back: () -> Unit) {
    LaunchedEffect(page) { when (page) { "privacy" -> command("contact_policy", emptyMap()); "profile" -> command("profile", emptyMap()); "device" -> command("devices", emptyMap()); "storage" -> command("storage", emptyMap()); "notifications" -> command("notification_settings", emptyMap()) } }
    var renaming by remember { mutableStateOf<AccountDevice?>(null) }
    var deviceName by remember { mutableStateOf("") }
    fun nameOf(device: AccountDevice) = state.ui["device_name.${device.id}"] ?: device.label?.takeUnless { it.matches(Regex("[0-9a-fA-F]{32,}")) } ?: if (device.current) "This device" else "Linked device"
    renaming?.let { device -> AlertDialog(onDismissRequest = { renaming = null }, title = { Text("Device name") },
        text = { OutlinedTextField(deviceName, { deviceName = it.take(80) }, singleLine = true, label = { Text("Name") }) },
        confirmButton = { SigilTextButton({ command("organize", mapOf("peer" to null, "value" to mapOf("UiSetting" to mapOf("key" to "device_name.${device.id}", "value" to deviceName.trim())))); renaming = null }, enabled = deviceName.isNotBlank() && !state.busy) { Text("Save") } },
        dismissButton = { SigilTextButton({ renaming = null }) { Text("Cancel") } }) }
    var revoking by remember { mutableStateOf<AccountDevice?>(null) }
    revoking?.let { device -> AlertDialog(onDismissRequest = { revoking = null }, title = { Text("Sign out this device?") },
        text = { Text("${nameOf(device)} will lose access to the server. Messages already stored there remain on that device.") },
        confirmButton = { SigilTextButton({ command("revoke_device", mapOf("device" to device.id)); revoking = null }) { Text("Sign out device") } },
        dismissButton = { SigilTextButton({ revoking = null }) { Text("Cancel") } }) }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
        Header(when(page) { "device" -> "Devices"; "profile" -> "Profile"; "privacy" -> "Privacy"; "notifications" -> "Notifications"; "storage" -> "Data and storage"; else -> "About" }, back)
        Column(Modifier.padding(24.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            when(page) {
                "device" -> {
                    Text("Devices on your account", style = MaterialTheme.typography.titleLarge)
                    SigilButton({ command("device_link", mapOf("action" to "sponsor")) }, enabled = !state.busy) { Text("Link a new device") }
                    Text("Manage where you’re signed in. Removing a device stops its access to your account.", style = MaterialTheme.typography.bodyMedium)
                    SigilTextButton({ command("devices", emptyMap()) }, enabled = !state.busy) { Text("Refresh") }
                    state.devices.filter { state.ui["device_hidden.${it.id}"] != "true" }.forEach { device ->
                        var details by remember(device.id) { mutableStateOf(false) }
                        Surface(shape = RoundedCornerShape(20.dp), color = MaterialTheme.colorScheme.surface) {
                            Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                                Row(verticalAlignment = Alignment.CenterVertically) {
                                    Glyph(if (device.current) "smartphone" else "devices", 28)
                                    Column(Modifier.weight(1f).padding(start = 12.dp)) {
                                        Text(nameOf(device), style = MaterialTheme.typography.titleMedium)
                                        Text(when { device.revoked == true -> "Signed out"; device.current -> "This device"; device.revoked == false -> "Signed in"; else -> "Previously linked" }, style = MaterialTheme.typography.bodySmall)
                                    }
                                    Symbol("edit", "Rename ${nameOf(device)}") { deviceName = nameOf(device); renaming = device }
                                }
                                Row {
                                    if (device.current) SigilTextButton({ command("sign_out", emptyMap()) }, enabled = !state.busy) { Text("Sign out") }
                                    else if (device.revoked != true) SigilTextButton({ revoking = device }, enabled = !state.busy) { Text("Remove device") }
                                    else SigilTextButton({ command("organize", mapOf("peer" to null, "value" to mapOf("UiSetting" to mapOf("key" to "device_hidden.${device.id}", "value" to "true")))) }, enabled = !state.busy) { Text("Remove from list") }
                                    SigilTextButton({ details = !details }) { Text("Details"); Glyph(if (details) "expand_less" else "expand_more", 18) }
                                }
                                Expandable(details) {
                                    Text("Device ID", style = MaterialTheme.typography.labelMedium)
                                    androidx.compose.foundation.text.selection.SelectionContainer { Text(device.id, fontFamily = LocalCodeFont.current, style = MaterialTheme.typography.bodySmall) }
                                    (device.fingerprint ?: state.fingerprint.takeIf { device.current })?.let { fingerprint ->
                                        Text("Encryption fingerprint", style = MaterialTheme.typography.labelMedium)
                                        androidx.compose.foundation.text.selection.SelectionContainer { Text(fingerprint.chunked(4).joinToString(" "), fontFamily = LocalCodeFont.current, style = MaterialTheme.typography.bodySmall) }
                                    }
                                }
                            }
                        }
                    }
                    state.devicesNext?.let { cursor -> SigilTextButton({ command("devices", mapOf("cursor" to cursor)) }, enabled = !state.busy) { Text("Load more devices") } }
                }
                "profile" -> {
                    Avatar(state.profileName.ifEmpty { state.address.removePrefix("@") }, 88, state.profileAvatar)
                    Row { SigilTextButton({ command("photo_choose", emptyMap()) }, enabled = !state.busy) { Text("Change photo") }; SigilTextButton({ command("photo_remove", emptyMap()) }, enabled = !state.busy) { Text("Remove photo") } }
                    Text("Your name and photo are shared with approved contacts and are visible to your server. They do not change your encryption identity.", style = MaterialTheme.typography.bodySmall)
                    if (state.photoPending) {
                        Text("Photo change waiting to upload")
                        Row { SigilTextButton({ command("photo_retry", emptyMap()) }, enabled = !state.busy) { Text("Retry upload") }; SigilTextButton({ command("photo_cancel", emptyMap()) }, enabled = !state.busy) { Text("Discard change") } }
                    }
                    var name by remember(state.profileRevision) { mutableStateOf(state.profileName) }
                    OutlinedTextField(name, { name = it }, label = { Text("Display name") })
                    Text(state.address)
                    SigilButton({ command("set_profile", mapOf("revision" to state.profileRevision, "name" to name.trim())) }, enabled = !state.busy && state.profileRevision != null) { Text("Save name") }
                    HorizontalDivider()
                    AccountAccessSection(state.accountAccess, state.busy, command)
                    HorizontalDivider()
                    SigilTextButton({ command("sign_out", emptyMap()) }, enabled = !state.busy) { Text("Sign out of this device") }
                }
                "privacy" -> {
                    Text("These preferences apply across your account. Conversations can have their own overrides.")
                    state.allowRequests?.let { enabled -> Toggle("Allow message requests", enabled) { command("contact_policy", mapOf("enabled" to it)) } }
                    Toggle("Read receipts", state.readReceipts) { command("organize", mapOf("peer" to null, "value" to mapOf("ReadReceipts" to it))) }
                    Toggle("Typing indicators", state.typingIndicators) { command("organize", mapOf("peer" to null, "value" to mapOf("TypingIndicators" to it))) }
                    Toggle("Share activity status", state.presenceSharing) { command("organize", mapOf("peer" to null, "value" to mapOf("PresenceSharing" to it))) }
                }
                "notifications" -> {
                    Text("On this device", style = MaterialTheme.typography.titleLarge)
                    Text("Notifications keep message content private. Snoozed conversations do not produce message alerts.")
                    state.notifications?.let { settings ->
                        if (!settings.enabled) { Text("Notifications are disabled in Android."); SigilButton({ command("notification_permission", emptyMap()) }) { Text("Enable notifications") } }
                        Toggle("Message notifications", settings.messages) { command("notification_change", mapOf("key" to "messages", "enabled" to it)) }
                        Toggle("Incoming call notifications", settings.calls) { command("notification_change", mapOf("key" to "incoming", "enabled" to it)) }
                    }
                    SigilTextButton({ command("notification_system_settings", emptyMap()) }) { Text("Sounds and Android notification settings") }
                    HorizontalDivider()
                    Text("Background delivery", style = MaterialTheme.typography.titleLarge)
                    state.push?.let { push ->
                        Text(push.status)
                        Text("Push wakes Sigil to check for messages. It does not contain your messages or contact names. Without push, Android checks periodically and incoming calls can be delayed.", style = MaterialTheme.typography.bodySmall)
                        if (push.distributors.isEmpty()) Text("For instant delivery, install a UnifiedPush service and enable UnifiedPush on your server.", style = MaterialTheme.typography.bodySmall)
                        push.distributors.forEach { service ->
                            SigilTextButton({ command("push_select", mapOf("distributor" to service.id)) }, enabled = !state.busy) { Text(if (push.enabled && push.distributor == service.id) "Reconnect ${service.name}" else "Use ${service.name}") }
                        }
                        if (push.enabled) SigilTextButton({ command("push_disable", emptyMap()) }, enabled = !state.busy) { Text("Use periodic sync instead") }
                    }
                    SigilTextButton({ command("notification_settings", emptyMap()) }, enabled = !state.busy) { Text("Refresh delivery status") }
                }
                "storage" -> {
                    Text("This device", style = MaterialTheme.typography.titleLarge)
                    Text("Messages and downloaded attachments are stored encrypted on this device.")
                    state.storage?.let { storage ->
                        Text("Message database · ${storageBytes(storage.database)}")
                        Text("Media cache · ${storageBytes(storage.mediaUsed)} used of ${storageBytes(storage.budget)}")
                        Text("${storageBytes(storage.media)} allocated on disk", style = MaterialTheme.typography.bodySmall)
                        HorizontalDivider()
                        Text("Encrypted history recovery", style = MaterialTheme.typography.titleMedium)
                        Text(if (storage.restoring) "Restoring encrypted history…" else if (!storage.recovery) "Not enabled" else storage.checkpoint?.let { "Last backup · $it" } ?: "Waiting for the first backup")
                        if (storage.restoring) { LinearProgressIndicator(Modifier.fillMaxWidth()); Text("You can leave this page. The import resumes after interruptions.", style = MaterialTheme.typography.bodySmall) }
                        if (storage.recovery && storage.unprotected > 0) Text("${storage.unprotected} records waiting for backup", style = MaterialTheme.typography.bodySmall)
                        if (!storage.recovery) {
                            Text("Keep a recovery key to restore your encrypted history after signing in on a replacement device.")
                            SigilButton({ command("recovery_generate", emptyMap()) }, enabled = !state.busy) { Text("Set up recovery") }
                            SigilTextButton({ command("recovery_restore_open", emptyMap()) }, enabled = !state.busy) { Text("Restore with a recovery key") }
                        } else {
                            Text("Keep backed-up history", style = MaterialTheme.typography.titleSmall)
                            Text("This controls your encrypted backup. It does not delete messages on this device.", style = MaterialTheme.typography.bodySmall)
                            listOf(null to "Until I delete it", 30 to "30 days", 90 to "90 days", 365 to "One year").forEach { (days, label) ->
                                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                                    RadioButton(storage.historyDays == days, { command("recovery_policy", mapOf("days" to days)) }, enabled = !state.busy && !storage.restoring)
                                    Text(label)
                                }
                            }
                        }
                    }
                    SigilTextButton({ command("storage", emptyMap()) }, enabled = !state.busy) { Text("Refresh") }
                    SigilTextButton({ command("history_open", emptyMap()) }) { Text("Browse saved history") }
                    state.transfers.forEach { transfer -> SettingRow("upload_file", transfer.name, transfer.phase) { command("file_cancel", mapOf("request" to transfer.request)) } }
                }
                else -> { Text("Sigil", style = MaterialTheme.typography.displayMedium); Text("Modern correspondence."); Text("Development build · 0.1"); Text("Newsreader, Google Sans Flex, Google Sans Code, and Material Symbols."); Text("Animated Noto Emoji by Google · CC BY 4.0. Lottie by Airbnb · Apache 2.0."); androidx.compose.foundation.text.selection.SelectionContainer { Text("https://googlefonts.github.io/noto-emoji-animation/\nhttps://creativecommons.org/licenses/by/4.0/", style = MaterialTheme.typography.bodySmall) } }
            }
        }
    }
}
private fun storageBytes(bytes: Long): String = when { bytes < 1024 -> "$bytes B"; bytes < 1024 * 1024 -> "${(bytes + 1023) / 1024} KiB"; else -> "${(bytes + 1024 * 1024 - 1) / (1024 * 1024)} MiB" }
@Composable
internal fun CallHistoryPage(state: MessengerState, command: Command) {
    Column(Modifier.fillMaxSize()) {
        if (state.calls.isEmpty()) Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) { Text("Your calls will appear here.", style = MaterialTheme.typography.bodyLarge) }
        else LazyColumn { items(state.calls, key = { it.id }) { call ->
            val other = call.participants.filter { !it.own }
            val status = when (call.phase) { "ringing" -> "Incoming call"; "joining" -> "Connecting"; "active" -> "In progress"; "declined" -> "Unanswered"; else -> if (call.outgoing) "Outgoing" else "Incoming" }
            SettingRow(if (call.outgoing) "call_made" else "call_received", call.name.ifEmpty { other.joinToString(", ") { it.name }.ifEmpty { "Call" } }, "$status · ${call.time}") {
                when (call.phase) {
                    "active", "joining" -> command("call_resume", mapOf("call" to call.id))
                    "ringing" -> command("call_answer", mapOf("call" to call.id))
                    else -> command("call_redial", mapOf("call" to call.id, "video" to false, "name" to call.name))
                }
            }
        } }
    }
}
