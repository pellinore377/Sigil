package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

@Composable
internal fun SettingsPage(state: MessengerState, navigate: (String) -> Unit) {
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(LocalHomeContentPadding.current),
        horizontalAlignment = Alignment.CenterHorizontally) {
        Column(Modifier.widthIn(max = 680.dp).fillMaxWidth().padding(horizontal = 16.dp).testTag("settings-content")) {
            Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(20.dp)).clickable { navigate("profile") }
                .padding(horizontal = 8.dp, vertical = 20.dp), verticalAlignment = Alignment.CenterVertically) {
                Avatar(state.profileName.ifEmpty { state.address.removePrefix("@") }, 60, state.profileAvatar)
                Column(Modifier.weight(1f).padding(start = 16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
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
                SettingsLink("info", "About", "Version, licenses, and support") { navigate("about") }
            }
            Spacer(Modifier.height(24.dp))
        }
    }
}

@Composable
internal fun SettingsSection(title: String, content: @Composable ColumnScope.() -> Unit) {
    Column(Modifier.fillMaxWidth()) {
        Text(title, Modifier.padding(start = 12.dp, top = 20.dp, bottom = 8.dp),
            style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        content()
    }
}

@Composable
internal fun SettingsLink(icon: String, title: String, detail: String, click: () -> Unit) {
    Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(18.dp)).clickable(role = Role.Button, onClick = click)
        .heightIn(min = 72.dp).padding(horizontal = 12.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(15.dp)) {
        CompositionLocalProvider(LocalContentColor provides MaterialTheme.colorScheme.onSurfaceVariant) { Glyph(icon, 24) }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
            Text(title, style = MaterialTheme.typography.titleMedium)
            Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        CompositionLocalProvider(LocalContentColor provides MaterialTheme.colorScheme.onSurfaceVariant) { Glyph("chevron_right", 20) }
    }
}

@Composable
internal fun SettingsToggle(title: String, detail: String, checked: Boolean, enabled: Boolean = true, update: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(18.dp)).toggleable(checked, enabled = enabled, role = Role.Switch, onValueChange = update).semantics { contentDescription = title }
        .heightIn(min = 76.dp).padding(horizontal = 12.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp)) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
            Text(title, style = MaterialTheme.typography.titleMedium)
            if (detail.isNotBlank()) Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Switch(checked, onCheckedChange = null, enabled = enabled)
    }
}

@Composable
internal fun SettingsValue(label: String, value: String) {
    Row(Modifier.fillMaxWidth().padding(vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(16.dp)) {
        Text(label, Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
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
