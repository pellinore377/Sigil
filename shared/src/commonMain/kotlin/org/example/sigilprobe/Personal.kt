package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.selection.selectable
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.draw.clip
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay

val LocalNotificationPanel=staticCompositionLocalOf<(@Composable (MessengerState,Command)->Unit)?> {null}

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
            Text("${selected.size} people will receive an invitation.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            val unverified = state.chats.filter { it.id in selected && !it.verified }
            unverified.forEach { person ->
                Text(person.name, style = MaterialTheme.typography.titleMedium)
                ContactRequestPanel(person, state.busy, command)
            }
            SigilButton({ command("group_create", mapOf("name" to title.trim(), "description" to description, "peers" to selected)) }, enabled = title.isNotBlank() && unverified.isEmpty() && !state.busy) { Text("Create group") }
        } else {
            if (selected.isNotEmpty()) LazyRow(contentPadding = PaddingValues(horizontal = 20.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                items(selected, key = { it }) { id -> InputChip(true, { selected = selected - id }, { Text(state.chats.find { it.id == id }?.name ?: "Note to Self") }, itemMotion(), trailingIcon = { Glyph("close", 15) }) }
            }
            OutlinedTextField(query, { query = it }, Modifier.fillMaxWidth().padding(20.dp), label = { Text("Name or user address") }, placeholder = { Text("@someone:example.com") }, singleLine = true, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, autoCorrectEnabled = false))
            LazyColumn(Modifier.weight(1f)) {
                item(key = "note-to-self") { SettingsLink("edit_note", "Note to Self", "A private space for your own thoughts") { selected = if ("self" in selected) selected - "self" else listOf("self") } }
                items(state.chats.filter { it.id != "self" && !it.group && (it.name.contains(query, true) || it.address.contains(query, true)) }, key = { it.id }) { chat ->
                    ChatRow(chat, chat.id in selected, open = { selected = if (chat.id in selected) selected - chat.id else selected.filter { it != "self" } + chat.id })
                }
            }
            SigilButton({ if (selected.size == 1) open(selected.single()) else creatingGroup = true }, Modifier.align(Alignment.End).padding(20.dp), enabled = selected.isNotEmpty()) { Text("Next") }
        }
    }
}
@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun NewCallDialog(state: MessengerState, command: Command, close: () -> Unit) {
    var query by rememberSaveable { mutableStateOf("") }
    var selected by rememberSaveable { mutableStateOf<String?>(null) }
    val features = LocalClientFeatures.current
    val contacts = state.chats.filter { it.id != "self" && it.verified && !it.hidden && !it.archived }
    val target = contacts.firstOrNull { it.id == selected }
    val enabled = features.calls && state.phase == "connected" && !state.busy && state.call == null && target != null
    fun start(video: Boolean) {
        if (!enabled || video && !features.videoCalls) return
        command("call_start", mapOf("peer" to target!!.id, "video" to video))
        close()
    }
    AlertDialog(onDismissRequest = close, title = { Text("New call") }, text = {
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            OutlinedTextField(query, { query = it }, Modifier.fillMaxWidth(), singleLine = true, label = { Text("Name or address") }, leadingIcon = { Glyph("search", 22) })
            val matches = contacts.filter { it.name.contains(query, true) || it.address.contains(query, true) }
            if (matches.isEmpty()) Text(if (query.isBlank()) "No contacts available." else "No matches.", color = MaterialTheme.colorScheme.onSurfaceVariant)
            else LazyColumn(Modifier.fillMaxWidth().heightIn(max = 320.dp)) {
                items(matches, key = { it.id }) { contact ->
                    Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).selectable(selected == contact.id, role = androidx.compose.ui.semantics.Role.RadioButton, onClick = { selected = contact.id }).padding(vertical = 10.dp, horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        Avatar(contact.name, 40, contact.avatar)
                        Text(contact.name, Modifier.weight(1f), maxLines = 2, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis, style = MaterialTheme.typography.titleMedium)
                        RadioButton(selected == contact.id, onClick = null)
                    }
                }
            }
        }
    }, confirmButton = {
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            SigilTextButton({ start(false) }, enabled = enabled) { Glyph("call", 20); Spacer(Modifier.width(8.dp)); Text("Audio call") }
            if (features.videoCalls) SigilTextButton({ start(true) }, enabled = enabled) { Glyph("videocam", 20); Spacer(Modifier.width(8.dp)); Text("Video call") }
        }
    }, dismissButton = { SigilTextButton(close) { Text("Cancel") } })
}
@OptIn(ExperimentalLayoutApi::class)
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
    SettingsDetailLayout(when(page) { "device" -> "Devices"; "profile" -> "Profile"; "privacy" -> "Privacy"; "notifications" -> "Notifications"; "storage" -> "Data and storage"; else -> "About" }, back, continuous = page == "privacy") {
            when(page) {
                "device" -> {
                    state.devices.filter { state.ui["device_hidden.${it.id}"] != "true" }.forEach { device ->
                        var details by remember(device.id) { mutableStateOf(false) }
                        Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                            Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(18.dp)).heightIn(min = 72.dp).padding(horizontal = 12.dp, vertical = 12.dp),
                                verticalAlignment = Alignment.CenterVertically) {
                                CompositionLocalProvider(LocalContentColor provides MaterialTheme.colorScheme.onSurfaceVariant) { Glyph(if (device.current) "smartphone" else "devices", 24) }
                                Column(Modifier.weight(1f).padding(horizontal = 12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                    Text(nameOf(device), style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                    Text(when { device.revoked == true -> "Signed out"; device.current -> "This device"; device.revoked == false -> "Signed in"; else -> "Previously linked" },
                                        style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                }
                                Symbol("edit", "Rename ${nameOf(device)}") { deviceName = nameOf(device); renaming = device }
                            }
                            FlowRow(Modifier.padding(horizontal = 12.dp), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                                if (device.current) SigilTextButton({ command("sign_out", emptyMap()) }, enabled = !state.busy) { Text("Sign out") }
                                else if (device.revoked != true) SigilTextButton({ revoking = device }, enabled = !state.busy) { Text("Remove device") }
                                else SigilTextButton({ command("organize", mapOf("peer" to null, "value" to mapOf("UiSetting" to mapOf("key" to "device_hidden.${device.id}", "value" to "true")))) }, enabled = !state.busy) { Text("Remove from list") }
                                SigilTextButton({ details = !details }) { Glyph(if (details) "expand_less" else "expand_more", 18); Spacer(Modifier.width(8.dp)); Text("Details") }
                            }
                            Expandable(details) {
                                Column(Modifier.padding(horizontal = 12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                    Text("Device ID", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                    androidx.compose.foundation.text.selection.SelectionContainer { Text(device.id, fontFamily = LocalCodeFont.current, style = MaterialTheme.typography.bodySmall) }
                                    (device.fingerprint ?: state.fingerprint.takeIf { device.current })?.let { fingerprint ->
                                        Text("Encryption fingerprint", Modifier.padding(top = 8.dp), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                        androidx.compose.foundation.text.selection.SelectionContainer { Text(fingerprint.chunked(4).joinToString(" "), fontFamily = LocalCodeFont.current, style = MaterialTheme.typography.bodySmall) }
                                    }
                                }
                            }
                        }
                    }
                    state.devicesNext?.let { cursor -> SigilTextButton({ command("devices", mapOf("cursor" to cursor)) }, enabled = !state.busy) { Text("Load more devices") } }
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        SigilButton({ command("device_link", mapOf("action" to "sponsor")) }, enabled = !state.busy) { Text("Link a new device") }
                        SigilTextButton({ command("devices", emptyMap()) }, enabled = !state.busy) { Text("Refresh") }
                    }
                }
                "profile" -> {
                    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) { Avatar(state.profileName.ifEmpty { state.address.removePrefix("@") }, 88, state.profileAvatar) }
                    if (LocalClientFeatures.current.files) FlowRow(Modifier.align(Alignment.CenterHorizontally), horizontalArrangement = Arrangement.spacedBy(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        SigilTextButton({ command("photo_choose", emptyMap()) }, enabled = !state.busy) { Text("Change photo") }
                        SigilTextButton({ command("photo_remove", emptyMap()) }, enabled = !state.busy) { Text("Remove photo") }
                    }
                    SettingsNote("Your name and photo are shared with approved contacts and are visible to your server. They do not change your encryption identity.")
                    if (state.photoPending) {
                        SettingsNote("Photo change waiting to upload")
                        FlowRow(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                            SigilTextButton({ command("photo_retry", emptyMap()) }, enabled = !state.busy) { Text("Retry upload") }
                            SigilTextButton({ command("photo_cancel", emptyMap()) }, enabled = !state.busy) { Text("Discard change") }
                        }
                    }
                    var name by remember(state.profileRevision) { mutableStateOf(state.profileName) }
                    OutlinedTextField(name, { name = it }, Modifier.fillMaxWidth(), shape = RoundedCornerShape(16.dp), label = { Text("Display name") })
                    SettingsNote(state.address)
                    SigilButton({ command("set_profile", mapOf("revision" to state.profileRevision, "name" to name.trim())) }, enabled = !state.busy && state.profileRevision != null) { Text("Save name") }
                    AccountAccessSection(state.accountAccess, state.busy, command)
                    SettingsSectionLabel("This device")
                    SigilTextButton({ command("sign_out", emptyMap()) }, enabled = !state.busy) { Text("Sign out of this device") }
                }
                "privacy" -> {
                    SettingsNote("Account preferences. Conversations can have their own overrides.")
                    SettingsToggle("Read receipts", "Let contacts see when you read messages", state.readReceipts, !state.busy) { command("organize", mapOf("peer" to null, "value" to mapOf("ReadReceipts" to it))) }
                    SettingsToggle("Typing indicators", "Show when you are writing", state.typingIndicators, !state.busy) { command("organize", mapOf("peer" to null, "value" to mapOf("TypingIndicators" to it))) }
                    state.allowRequests?.let { enabled -> SettingsToggle("Allow message requests", "Let people request a conversation", enabled, !state.busy) { command("contact_policy", mapOf("enabled" to it)) } }
                    SettingsToggle("Share activity status", "Let contacts see your activity", state.presenceSharing, !state.busy) { command("organize", mapOf("peer" to null, "value" to mapOf("PresenceSharing" to it))) }
                }
                "notifications" -> {
                    val panel=LocalNotificationPanel.current
                    if(panel!=null)panel(state,command) else {
                    SettingsSectionLabel("On this device")
                    SettingsNote("Notifications keep message content private. Snoozed conversations do not produce message alerts.")
                    state.notifications?.let { settings ->
                        if (!settings.enabled) { SettingsNote("Notifications are disabled in Android."); SigilButton({ command("notification_permission", emptyMap()) }) { Text("Enable notifications") } }
                        SettingsToggle("Message notifications", "Notify you of new messages", settings.messages, !state.busy) { command("notification_change", mapOf("key" to "messages", "enabled" to it)) }
                        SettingsToggle("Incoming call notifications", "Notify you of incoming calls", settings.calls, !state.busy) { command("notification_change", mapOf("key" to "incoming", "enabled" to it)) }
                        if (!settings.fullScreen) { SettingsNote("Android needs permission to show incoming calls on the lock screen."); SigilButton({ command("notification_full_screen", emptyMap()) }) { Text("Allow full-screen call alerts") } }
                        if (!settings.unrestricted) { SettingsNote("Battery optimization delays messages and calls while the phone sleeps."); SigilButton({ command("notification_battery", emptyMap()) }) { Text("Allow unrestricted battery use") } }
                        SettingsChoice("Show in notifications", listOf("full" to "Name and message", "name" to "Name only", "none" to "No name or message"), settings.content, !state.busy) { command("notification_content", mapOf("level" to it)) }
                    }
                    SigilTextButton({ command("notification_system_settings", emptyMap()) }) { Text("Sounds and Android notification settings") }
                    SettingsSectionLabel("Background delivery")
                    state.push?.let { push ->
                        SettingsNote(push.status)
                        SettingsNote("Push wakes Sigil to check for messages. It does not contain your messages or contact names. Without push, Android checks periodically and incoming calls can be delayed.")
                        if (push.distributors.isEmpty()) SettingsNote("Google notifications require a configured app build and server. UnifiedPush is also available with an installed distributor.")
                        FlowRow(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                            push.distributors.forEach { service ->
                                SigilTextButton({ command("push_select", mapOf("distributor" to service.id)) }, enabled = !state.busy) { Text(if (push.enabled && push.distributor == service.id) "Reconnect ${service.name}" else "Use ${service.name}") }
                            }
                            if (push.enabled) SigilTextButton({ command("push_disable", emptyMap()) }, enabled = !state.busy) { Text("Use periodic sync instead") }
                        }
                    }
                    SigilTextButton({ command("notification_settings", emptyMap()) }, enabled = !state.busy) { Text("Refresh delivery status") }
                    }
                }
                "storage" -> {
                    SettingsSectionLabel("This device")
                    state.storage?.let { storage ->
                        Text("${storageBytes(storage.database + storage.media)} on this device", Modifier.padding(horizontal = 12.dp), style = MaterialTheme.typography.headlineSmall)
                        SettingsNote("Messages and downloaded attachments are stored encrypted on this device.")
                        Column {
                            SettingsValue("Message database", storageBytes(storage.database))
                            SettingsValue("Media cache", storageBytes(storage.mediaUsed))
                            SettingsValue("Cache limit", storageBytes(storage.budget))
                        }
                        Text("${storageBytes(storage.media)} allocated for media", Modifier.padding(horizontal = 12.dp), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        SettingsSectionLabel("Encrypted history recovery")
                        SettingsNote(if (storage.restoring) "Restoring encrypted history…" else if (!storage.recovery) "Not enabled" else storage.checkpoint?.let { "Last backup · $it" } ?: "Waiting for the first backup")
                        if (storage.restoring) { LinearProgressIndicator(Modifier.fillMaxWidth()); SettingsNote("You can leave this page. The import resumes after interruptions.") }
                        if (storage.recovery && storage.unprotected > 0) SettingsNote("${storage.unprotected} records waiting for backup")
                        if (!storage.recovery) {
                            SettingsNote("Keep a recovery key to restore your encrypted history after signing in on a replacement device.")
                            FlowRow(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                                SigilButton({ command("recovery_generate", emptyMap()) }, enabled = !state.busy) { Text("Set up recovery") }
                                SigilTextButton({ command("recovery_restore_open", emptyMap()) }, enabled = !state.busy) { Text("Restore with a recovery key") }
                            }
                        } else {
                            SettingsNote("This controls your encrypted backup. It does not delete messages on this device.")
                            SettingsChoice("Keep backed-up history", listOf("" to "Until I delete it", "30" to "30 days", "90" to "90 days", "365" to "One year"),
                                storage.historyDays?.toString().orEmpty(), enabled = !state.busy && !storage.restoring) { command("recovery_policy", mapOf("days" to it.toIntOrNull())) }
                        }
                    }
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        SigilTextButton({ command("storage", emptyMap()) }, enabled = !state.busy) { Text("Refresh") }
                        SigilTextButton({ command("history_open", emptyMap()) }) { Text("Browse saved history") }
                    }
                    if (state.transfers.isNotEmpty()) SettingsSectionLabel("Transfers")
                    state.transfers.forEach { transfer ->
                        Row(Modifier.fillMaxWidth().heightIn(min = 72.dp).padding(horizontal = 12.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                            CompositionLocalProvider(LocalContentColor provides MaterialTheme.colorScheme.onSurfaceVariant) { Glyph("upload_file", 24) }
                            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                                Text(transfer.name, style = MaterialTheme.typography.titleMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
                                Text(transfer.phase, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                            Symbol("close", "Cancel ${transfer.name}") { command("file_cancel", mapOf("request" to transfer.request)) }
                        }
                    }
                }
                else -> {
                    Text("Sigil", Modifier.padding(horizontal = 12.dp), style = MaterialTheme.typography.headlineSmall)
                    SettingsNote("Modern correspondence.")
                    SettingsValue("Version", "Development build · 0.1")
                    SettingsSectionLabel("Credits and licenses")
                    Text("Newsreader, Google Sans Flex, Google Sans Code, and Material Symbols.", Modifier.padding(horizontal = 12.dp), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    Text("Animated Noto Emoji by Google · CC BY 4.0. Lottie by Airbnb · Apache 2.0.", Modifier.padding(horizontal = 12.dp), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    androidx.compose.foundation.text.selection.SelectionContainer {
                        Text("https://googlefonts.github.io/noto-emoji-animation/\nhttps://creativecommons.org/licenses/by/4.0/", Modifier.padding(horizontal = 12.dp),
                            style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
    }
}
private fun storageBytes(bytes: Long): String = when { bytes < 1024 -> "$bytes B"; bytes < 1024 * 1024 -> "${(bytes + 1023) / 1024} KiB"; else -> "${(bytes + 1024 * 1024 - 1) / (1024 * 1024)} MiB" }
