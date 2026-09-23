package org.sigil

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import org.jetbrains.compose.resources.painterResource
import sigil.shared.generated.resources.Res
import sigil.shared.generated.resources.sigil_mark

// The onboarding card: the mark and the word above one tonal card that refills as the steps go by, with quiet actions beneath it.
@Composable internal fun OnboardingCard(foot: @Composable ColumnScope.() -> Unit = {}, content: @Composable ColumnScope.() -> Unit) {
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(top = 48.dp, bottom = 40.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(24.dp, Alignment.CenterVertically)) {
        Column(Modifier.widthIn(max = 560.dp).fillMaxWidth().padding(horizontal = 24.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(20.dp)) {
            Icon(painterResource(Res.drawable.sigil_mark), null, Modifier.height(84.dp).width(50.dp), tint = MaterialTheme.colorScheme.onBackground)
            Text("Sigil", style = MaterialTheme.typography.displayMedium)
            Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(24.dp)).background(MaterialTheme.colorScheme.onSurface.copy(alpha = .045f)).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp), content = content)
            Column(Modifier.fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally, content = foot)
        }
    }
}

// Fields sit quietly on the card: a tonal fill, no outline.
@Composable internal fun quietFieldColors(): TextFieldColors {
    val scheme = MaterialTheme.colorScheme
    val none = androidx.compose.ui.graphics.Color.Transparent
    return TextFieldDefaults.colors(focusedContainerColor = scheme.onSurface.copy(alpha = .06f), unfocusedContainerColor = scheme.onSurface.copy(alpha = .06f), disabledContainerColor = scheme.onSurface.copy(alpha = .04f),
        focusedIndicatorColor = none, unfocusedIndicatorColor = none, disabledIndicatorColor = none)
}

@Composable internal fun OnboardingTitle(text: String) = Text(text, style = MaterialTheme.typography.headlineSmall)

// A status line with a small dot: the server found, a name available, a device recognised.
@Composable internal fun OnboardingStatus(text: String, ok: Boolean = true, trailing: (@Composable () -> Unit)? = null) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Box(Modifier.size(8.dp).background(if (ok) MaterialTheme.colorScheme.tertiary else MaterialTheme.colorScheme.error, CircleShape))
        Text(text, Modifier.weight(1f), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        trailing?.invoke()
    }
}

// A way to sign in, as a row with a squircle key and a line saying what it means.
@Composable internal fun OnboardingOption(icon: String, title: String, detail: String, enabled: Boolean = true, click: () -> Unit) {
    Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).clickable(enabled = enabled, role = Role.Button, onClick = click).padding(vertical = 8.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Surface(Modifier.size(40.dp), shape = RoundedCornerShape(13.dp), color = MaterialTheme.colorScheme.surfaceVariant) { Box(contentAlignment = Alignment.Center) { Glyph(icon, 22) } }
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleMedium)
            Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        CompositionLocalProvider(LocalContentColor provides MaterialTheme.colorScheme.onSurfaceVariant) { Glyph("chevron_right", 20) }
    }
}

// Once, for a new account without a passkey: a way back in on a new phone or computer.
@Composable internal fun ProtectAccount(state: MessengerState, command: Command, done: () -> Unit) {
    OnboardingCard(foot = { SigilTextButton(done, enabled = !state.busy) { Text("Not now") } }) {
        OnboardingTitle("Protect your account")
        Text("Create a passkey so you can get your conversations back on a new phone or computer.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        state.issue?.let { OnboardingStatus(it, ok = false) }
        SigilButton({ command("passkey_create", emptyMap()) }, Modifier.fillMaxWidth(), enabled = !state.busy) { Text("Create passkey") }
    }
}

// The last step: what contacts may see, chosen once here rather than found later in settings.
@Composable internal fun WelcomePermissions(state: MessengerState, command: Command, done: () -> Unit) {
    OnboardingCard(foot = { SigilButton(done, Modifier.fillMaxWidth(), enabled = !state.busy) { Text("Start messaging") } }) {
        // The toggles carry their own inset, so the label and the line above them share it.
        SettingsGroupLabel("Before you begin")
        Text("What your contacts can see about you. Each of these can be changed later in Privacy.", Modifier.padding(horizontal = 12.dp), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        SettingsToggle("Read receipts", "Let contacts see when you read messages", state.readReceipts, !state.busy) { command("organize", mapOf("peer" to null, "value" to mapOf("ReadReceipts" to it))) }
        SettingsToggle("Typing indicators", "Show when you are writing", state.typingIndicators, !state.busy) { command("organize", mapOf("peer" to null, "value" to mapOf("TypingIndicators" to it))) }
        SettingsToggle("Share activity status", "Let contacts see your activity", state.presenceSharing, !state.busy) { command("organize", mapOf("peer" to null, "value" to mapOf("PresenceSharing" to it))) }
        state.allowRequests?.let { enabled -> SettingsToggle("Allow message requests", "Let people request a conversation", enabled, !state.busy) { command("contact_policy", mapOf("enabled" to it)) } }
    }
}

// The whole flow with made-up state, for looking at, reachable from About: every screen in turn, nothing sent anywhere.
@Composable internal fun OnboardingPreview(close: () -> Unit) {
    var step by remember { mutableIntStateOf(0) }
    val server = "chat.example.org"
    val methods = LoginMethods(server, sso = true, password = true, invitation = true)
    val steps = listOf("Server", "Server found, SSO only", "All methods", "Password", "Invitation", "Waiting for SSO", "Your name", "Welcome back", "Recovery code", "Protect your account", "Before you begin")
    Box(Modifier.fillMaxSize().background(LocalGlobalBackground.current)) {
        CompositionLocalProvider(LocalClientFeatures provides LocalClientFeatures.current.copy(passkeys = true)) {
            when (steps[step]) {
                "Server" -> SignIn(MessengerState(phase = "new", loginAddress = "", ui = emptyMap()), { _, _ -> })
                "Server found, SSO only" -> SignIn(MessengerState(phase = "new", loginAddress = server, loginMethods = LoginMethods(server, sso = true, password = false, invitation = false)), { _, _ -> })
                "All methods" -> SignIn(MessengerState(phase = "new", loginAddress = server, loginMethods = methods), { _, _ -> })
                "Password" -> SignIn(MessengerState(phase = "new", loginAddress = server, loginMethods = methods), { _, _ -> }, initialMethod = "password")
                "Invitation" -> SignIn(MessengerState(phase = "new", loginAddress = server, loginMethods = methods), { _, _ -> }, initialMethod = "invitation")
                "Waiting for SSO" -> SignIn(MessengerState(phase = "oidc", loginAddress = server, loginMethods = methods), { _, _ -> })
                "Your name" -> SignIn(MessengerState(phase = "username", loginAddress = server, loginMethods = methods), { _, _ -> })
                "Welcome back" -> SignIn(MessengerState(phase = "recover", loginAddress = server, recoverAddress = "@sam:$server", recoverPasskeys = 1), { _, _ -> })
                "Recovery code" -> SignIn(MessengerState(phase = "recover", loginAddress = server, recoverAddress = "@sam:$server", recoverPasskeys = 1), { _, _ -> }, initialMethod = "code")
                "Protect your account" -> ProtectAccount(MessengerState(phase = "connected", accountRecovery = AccountRecovery(emptyList(), true)), { _, _ -> }) {}
                else -> WelcomePermissions(MessengerState(phase = "connected", readReceipts = true, typingIndicators = true, presenceSharing = false, allowRequests = true), { _, _ -> }) {}
            }
        }
        // The preview's own controls: step name, back and next, and a way out.
        Row(Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(12.dp).clip(RoundedCornerShape(18.dp)).background(MaterialTheme.colorScheme.surfaceContainerHigh.copy(alpha = .94f)).padding(horizontal = 6.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            Symbol("close", "Leave preview", close)
            Symbol("chevron_left", "Previous screen") { if (step > 0) step-- }
            Text("${step + 1} of ${steps.size} · ${steps[step]}", Modifier.weight(1f), style = MaterialTheme.typography.labelMedium, textAlign = TextAlign.Center)
            Symbol("chevron_right", "Next screen") { if (step < steps.lastIndex) step++ }
        }
    }
}
