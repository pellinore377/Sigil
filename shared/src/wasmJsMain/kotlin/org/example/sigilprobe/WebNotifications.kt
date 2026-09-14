package org.sigil
import androidx.compose.runtime.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

@Composable internal fun WebNotificationSettings(state:MessengerState,command:Command) {
    LaunchedEffect(Unit){command("notification_settings",emptyMap())}
    Text("This browser",style=MaterialTheme.typography.titleLarge)
    Text("Receive a private reminder to open Sigil, even when its tab is closed. Your messages and contact names stay out of notifications.")
    Text("Your browser's push service delivers these reminders. Your server must enable Web Push. Browsers can suspend delivery when fully quit or restricted by the operating system.",style=MaterialTheme.typography.bodySmall)
    state.push?.let {push->
        Text(push.status,style=MaterialTheme.typography.titleMedium)
        Column(Modifier.fillMaxWidth(),verticalArrangement=Arrangement.spacedBy(8.dp)) {
            push.distributors.forEach {SigilButton({command("push_select",mapOf("distributor" to it.id))},enabled=!state.busy){Text(if(push.enabled)"Reconnect notifications" else "Enable notifications")}}
            if(push.enabled)SigilOutlinedButton({command("push_disable",emptyMap())},enabled=!state.busy){Text("Disable notifications")}
        }
    }
    Text("When disabled, this browser syncs while Sigil is open. Site permissions and sounds are managed in your browser settings.",style=MaterialTheme.typography.bodySmall)
    SigilTextButton({command("notification_settings",emptyMap())},enabled=!state.busy){Text("Refresh status")}
}
