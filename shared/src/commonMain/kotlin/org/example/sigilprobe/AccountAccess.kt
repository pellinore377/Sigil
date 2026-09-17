package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp

@Composable
internal fun AccountAccessSection(access: AccountAccess?, busy: Boolean, command: Command) {
    SettingsSectionLabel("Account access")
    SettingsNote("Sign-in methods are managed by your server administrator.")
    if (access == null) {
        SigilTextButton({ command("account_access", emptyMap()) }, enabled = !busy) { Text("Check account access") }
        return
    }
    if (access.issuer != null) SettingsNote(access.issuer)
    when {
        access.linked && access.retiring -> {
            SettingsNote("Your administrator is preparing to turn off this SSO provider.")
            SettingsNote("Arrange an administrator-issued sign-in invitation before acknowledging this change. Keep your recovery key and encrypted backup for restoring history on a replacement device.")
            if (access.acknowledged) SettingsNote("You acknowledged this sign-in change.")
            else {
                var arranged by remember(access.configuration, access.transition) { mutableStateOf(false) }
                Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(18.dp)).toggleable(arranged, enabled = !busy, role = Role.Checkbox) { arranged = it }
                    .heightIn(min = 48.dp).padding(horizontal = 12.dp, vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    Checkbox(arranged, null, enabled = !busy)
                    Text("I have arranged account access with my administrator.", style = MaterialTheme.typography.bodyMedium)
                }
                SigilButton({ command("acknowledge_access", mapOf("configuration_revision" to access.configuration, "transition_revision" to access.transition)) }, enabled = arranged && !busy) { Text("Acknowledge sign-in change") }
            }
        }
        access.linked -> SettingsNote("SSO is linked to this account.")
        access.linkPending -> {
            SettingsNote("Finish linking in your browser. Your existing Sigil account will be kept.")
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                SigilTextButton({ command("oidc_account", mapOf("action" to "resume")) }, enabled = !busy) { Text("Continue linking") }
                SigilTextButton({ command("oidc_account", mapOf("action" to "cancel")) }, enabled = !busy) { Text("Cancel linking") }
            }
        }
        access.issuer != null && !access.retiring -> {
            SettingsNote("Link SSO to sign in to this existing account.")
            SigilButton({ command("oidc_account", mapOf("action" to "start")) }, enabled = !busy) { Text("Link SSO account") }
        }
        else -> SettingsNote("SSO linking is unavailable on this server.")
    }
    SigilTextButton({ command("account_access", emptyMap()) }, enabled = !busy) { Text("Refresh account access") }
}
