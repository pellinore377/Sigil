package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

@Composable
internal fun AccountAccessSection(access: AccountAccess?, busy: Boolean, command: Command) {
    Text("Account access", style = MaterialTheme.typography.titleMedium)
    Text("Sign-in methods are managed by your server administrator.", style = MaterialTheme.typography.bodySmall)
    if (access == null) {
        TextButton({ command("account_access", emptyMap()) }, enabled = !busy) { Text("Check account access") }
        return
    }
    if (access.issuer != null) Text(access.issuer, style = MaterialTheme.typography.bodySmall)
    when {
        access.linked && access.retiring -> {
            Text("Your administrator is preparing to turn off this SSO provider.")
            Text("Arrange an administrator-issued sign-in invitation before acknowledging this change. Keep your recovery key and encrypted backup for restoring history on a replacement device.")
            if (access.acknowledged) Text("You acknowledged this sign-in change.")
            else {
                var arranged by remember(access.configuration, access.transition) { mutableStateOf(false) }
                Row { Checkbox(arranged, { arranged = it }, enabled = !busy); Text("I have arranged account access with my administrator.", Modifier.padding(top = 12.dp)) }
                Button({ command("acknowledge_access", mapOf("configuration_revision" to access.configuration, "transition_revision" to access.transition)) }, enabled = arranged && !busy) { Text("Acknowledge sign-in change") }
            }
        }
        access.linked -> Text("SSO is linked to this account.")
        access.linkPending -> {
            Text("Finish linking in your browser. Your existing Sigil account will be kept.")
            Row {
                TextButton({ command("oidc_account", mapOf("action" to "resume")) }, enabled = !busy) { Text("Continue linking") }
                TextButton({ command("oidc_account", mapOf("action" to "cancel")) }, enabled = !busy) { Text("Cancel linking") }
            }
        }
        access.issuer != null && !access.retiring -> {
            Text("Link SSO to sign in to this existing account.")
            Button({ command("oidc_account", mapOf("action" to "start")) }, enabled = !busy) { Text("Link SSO account") }
        }
        else -> Text("SSO linking is unavailable on this server.")
    }
    TextButton({ command("account_access", emptyMap()) }, enabled = !busy) { Text("Refresh account access") }
}
