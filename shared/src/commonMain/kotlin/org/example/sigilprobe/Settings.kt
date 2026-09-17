package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

@Composable
internal fun SettingsPage(state: MessengerState, navigate: (String) -> Unit) {
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(LocalHomeContentPadding.current),
        horizontalAlignment = Alignment.CenterHorizontally) {
        val appearance = LocalAppearance.current
        Column(Modifier.widthIn(max = 680.dp).fillMaxWidth().padding(horizontal = 16.dp).testTag("settings-content")) {
            Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(18.dp)).clickable(role = Role.Button) { navigate("profile") }
                .heightIn(min = if (appearance.compact) 72.dp else 88.dp).padding(horizontal = 12.dp, vertical = 12.dp),
                verticalAlignment = Alignment.CenterVertically) {
                Avatar(state.profileName.ifEmpty { state.address.removePrefix("@") }, if (appearance.compact) 48 else 56, state.profileAvatar)
                Column(Modifier.weight(1f).padding(horizontal = 12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(state.profileName.ifEmpty { state.address.substringBefore(':').removePrefix("@") },
                        style = MaterialTheme.typography.headlineSmall, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    Text(state.address, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 2, overflow = TextOverflow.Ellipsis)
                }
                Symbol("qr_code", "My contact code") { navigate("contact-code") }
            }
            SettingsSection("Account") {
                SettingsLink("person", "Profile", "Display name and photo") { navigate("profile") }
                SettingsLink("lock", "Privacy", "Receipts, typing, and contact requests") { navigate("privacy") }
                SettingsLink("devices", "Devices", "Linked devices and verification") { navigate("device") }
                if (LocalClientFeatures.current.notifications) SettingsLink("notifications", "Notifications", "Messages, calls, and sounds") { navigate("notifications") }
            }
            SettingsSection("Personalize") { SettingsLink("palette", "Appearance", "Theme, typography, and layout") { navigate("appearance") } }
            SettingsSection("Storage & support") {
                if (LocalClientFeatures.current.files) SettingsLink("database", "Data and storage", "Media, downloads, and cache") { navigate("storage") }
                DiagnosticsLink(state.issue)
                SettingsLink("info", "About", "Version, licenses, and support") { navigate("about") }
            }
        }
    }
}

/// Counts and stage names only, so a report can be shared to explain a device that
/// will not send or receive without disclosing anything that was said.
@Composable
private fun DiagnosticsLink(issue: String?) {
    val access = LocalServiceAccess.current
    val clipboard = LocalClipboardManager.current
    val scope = rememberCoroutineScope()
    var state by remember { mutableStateOf("Copy a report for troubleshooting") }
    SettingsLink("bug_report", "Diagnostics", state) {
        val request = access ?: return@SettingsLink
        scope.launch {
            val report = runCatching {
                // The request layer already unwraps the reply to its value.
                Json.parseToJsonElement(request("{\"command\":\"diagnostics\"}").json)
                    .jsonObject["report"]
                    ?.jsonPrimitive
                    ?.content
            }.getOrNull()
            state = if (report == null) {
                "Could not read the report. Try again."
            } else {
                // The stage that is failing is the first thing anyone reading this needs.
                val stage = issue?.let { "\n$it" }.orEmpty()
                clipboard.setText(AnnotatedString(report + stage))
                "Copied. Paste it wherever you are reporting the problem."
            }
        }
    }
}

@Composable
internal fun SettingsSection(title: String, content: @Composable ColumnScope.() -> Unit) {
    Column(Modifier.fillMaxWidth()) {
        SettingsSectionLabel(title)
        content()
    }
}

@Composable
internal fun SettingsSectionLabel(title: String) {
    Text(title, Modifier.padding(start = 12.dp, top = 20.dp, bottom = 8.dp),
        style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
}

@Composable
internal fun SettingsNote(text: String) {
    Text(text, Modifier.padding(horizontal = 12.dp), style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant)
}

@Composable
internal fun SettingsLink(icon: String, title: String, detail: String, click: () -> Unit) {
    Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(18.dp)).clickable(role = Role.Button, onClick = click)
        .heightIn(min = 72.dp).padding(horizontal = 12.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp)) {
        CompositionLocalProvider(LocalContentColor provides MaterialTheme.colorScheme.onSurfaceVariant) { Glyph(icon, 24) }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(title, style = MaterialTheme.typography.titleMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        CompositionLocalProvider(LocalContentColor provides MaterialTheme.colorScheme.onSurfaceVariant) { Glyph("chevron_right", 20) }
    }
}

@Composable
internal fun SettingsToggle(title: String, detail: String, checked: Boolean, enabled: Boolean = true, description: String = title, update: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(18.dp)).toggleable(checked, enabled = enabled, role = Role.Switch, onValueChange = update).semantics { contentDescription = description }
        .heightIn(min = 76.dp).padding(horizontal = 12.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp)) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(title, style = MaterialTheme.typography.titleMedium)
            if (detail.isNotBlank()) Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Switch(checked, onCheckedChange = null, enabled = enabled)
    }
}

@Composable
internal fun SettingsChoice(title: String, options: List<Pair<String, String>>, selected: String, enabled: Boolean = true, update: (String) -> Unit) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Text(title, Modifier.padding(bottom = 8.dp), style = MaterialTheme.typography.titleLarge)
        options.forEach { (value, label) ->
            Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).selectable(value == selected, enabled = enabled, role = Role.RadioButton) { update(value) }
                .heightIn(min = 48.dp).padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                RadioButton(value == selected, onClick = null, enabled = enabled)
                Text(label, style = MaterialTheme.typography.bodyMedium)
            }
        }
    }
}

@Composable
internal fun SettingsValue(label: String, value: String) {
    Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(16.dp)) {
        Text(label, Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1, overflow = TextOverflow.Ellipsis)
        Text(value, style = MaterialTheme.typography.bodyMedium)
    }
}


@Composable
internal fun SettingsDetailLayout(title: String, back: () -> Unit, continuous: Boolean = false, content: @Composable ColumnScope.() -> Unit) {
    val insets = if (LocalPageHeader.current) LocalHomeContentPadding.current else PaddingValues(bottom = 24.dp)
    Column(Modifier.fillMaxSize()) {
        Header(title, back)
        Column(Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()).padding(insets),
            horizontalAlignment = Alignment.CenterHorizontally) {
            Column(Modifier.widthIn(max = 680.dp).fillMaxWidth().padding(horizontal = 16.dp, vertical = if (continuous) 0.dp else 8.dp),
                verticalArrangement = Arrangement.spacedBy(if (continuous) 0.dp else 16.dp), content = content)
        }
    }
}
