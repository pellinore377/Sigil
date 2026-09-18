package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.foundation.text.input.clearText
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.drawscope.clipRect
import org.jetbrains.compose.resources.Font
import org.jetbrains.compose.resources.painterResource
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.expandVertically
import androidx.compose.animation.shrinkVertically
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.core.tween
import androidx.compose.ui.text.input.PasswordVisualTransformation
import kotlinx.coroutines.delay
import sigil.shared.generated.resources.*

@Composable
internal fun Symbol(name: String, label: String, action: () -> Unit) {
    SigilIconButton(action, Modifier.semantics { contentDescription = label }) {
        Glyph(name)
    }
}
internal fun Modifier.headerShadow(shape: Shape = RectangleShape, elevation: androidx.compose.ui.unit.Dp = 2.dp) = drawWithContent {
    clipRect(top = 0f, bottom = size.height + 12.dp.toPx()) { this@drawWithContent.drawContent() }
}.shadow(elevation, shape, clip = false)
@Composable
internal fun pageHeaderHeight() = with(androidx.compose.ui.platform.LocalDensity.current) { maxOf(76.dp, MaterialTheme.typography.displaySmall.lineHeight.toDp() + 24.dp, MaterialTheme.typography.titleLarge.lineHeight.toDp() + MaterialTheme.typography.bodyMedium.lineHeight.toDp() + 16.dp) }
internal val LocalPageHeader = staticCompositionLocalOf { false }
@Composable
internal fun Header(title: String, back: (() -> Unit)? = null, action: @Composable RowScope.() -> Unit = {}) {
    if (LocalPageHeader.current) return
    Surface(Modifier.headerShadow(), color = MaterialTheme.colorScheme.surface) {
    Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
        if (back != null) Symbol("arrow_back", "Back", back)
        MainHeaderTitle(title, Modifier.weight(1f).padding(start = 8.dp))
        action()
    }
    }
}
@Composable
internal fun Avatar(name: String, size: Int = 48, photo: String = "") {
    val initialSize = with(androidx.compose.ui.platform.LocalDensity.current) { (size * .43f).dp.toSp() }
    Surface(Modifier.size(size.dp).clearAndSetSemantics { contentDescription = name }, shape = CircleShape, color = MaterialTheme.colorScheme.surfaceVariant) {
        Box(contentAlignment = Alignment.Center) { Text(name.take(1).uppercase(), fontSize = initialSize, lineHeight = initialSize, maxLines = 1); if (photo.isNotEmpty()) LocalProfilePhoto.current(photo, Modifier.matchParentSize()) }
    }
}
val LocalProfilePhoto = staticCompositionLocalOf<@Composable (String, Modifier) -> Unit> { { _, _ -> } }
@Composable
internal fun SettingRow(icon: String, title: String, detail: String, click: () -> Unit) = SettingsLink(icon, title, detail, click)
@Composable
internal fun SignIn(state: MessengerState, command: (String, Map<String, Any?>) -> Unit, initialMethod: String = "") {
    var method by remember(state.loginAddress) { mutableStateOf(initialMethod) }
    var username by remember { mutableStateOf("") }
    var password by remember(state.loginAddress) { mutableStateOf("") }
    var invitation by remember { mutableStateOf("") }
    LaunchedEffect(state.loginAddress, state.phase) {
        if (state.phase == "new" && state.loginAddress.isNotBlank()) {
            delay(650)
            command("discover", mapOf("server" to state.loginAddress))
        }
    }
    val methods = state.loginMethods
    val passwordForm = method == "password" || state.phase == "password"
    val invitationForm = method == "invitation" && methods?.invitation == true
    val ready = !state.busy && username.isNotBlank() && password.isNotEmpty()
    val submitPassword = { if (ready) command("password", mapOf("server" to (methods?.server ?: state.loginAddress), "username" to username.trim(), "password" to password)) }
    val scheme = MaterialTheme.colorScheme
    // Fields sit quietly on the card: a tonal fill, no outline.
    val fieldShape = RoundedCornerShape(14.dp)
    val quiet = TextFieldDefaults.colors(focusedContainerColor = scheme.onSurface.copy(alpha = .06f), unfocusedContainerColor = scheme.onSurface.copy(alpha = .06f), disabledContainerColor = scheme.onSurface.copy(alpha = .04f),
        focusedIndicatorColor = androidx.compose.ui.graphics.Color.Transparent, unfocusedIndicatorColor = androidx.compose.ui.graphics.Color.Transparent, disabledIndicatorColor = androidx.compose.ui.graphics.Color.Transparent)
    val inForm = passwordForm || invitationForm || state.phase != "new"
    val recovery = LocalClientFeatures.current.recovery && (methods != null || state.phase != "new")
    OnboardingCard(foot = {
        if (!inForm) {
            SigilTextButton({ command("device_link", mapOf("action" to "join", "server" to state.loginAddress)) }, enabled = !state.busy && methods != null) { Text("Link to an existing device") }
            if (recovery) SigilTextButton({ command("recovery_account_open", emptyMap()) }, enabled = !state.busy) { Text("Recover a lost account") }
        } else if (state.phase == "new") SigilTextButton({ method = "" }, enabled = !state.busy) { Text("Other ways to sign in") }
        else SigilTextButton({ command("cancel_login", emptyMap()) }, enabled = !state.busy) { Text("Back to sign-in choices") }
    }) {
        if (!inForm) {
            SettingsGroupLabel("Server", inset = 0.dp)
            TextField(state.loginAddress, { command("server_changed", mapOf("server" to it)) }, Modifier.fillMaxWidth().testTag("server-address"),
                placeholder = { Text("Server address") }, singleLine = true, enabled = !state.busy && state.phase == "new", shape = fieldShape, colors = quiet,
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done, keyboardType = KeyboardType.Uri, capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false),
                keyboardActions = KeyboardActions(onDone = { if (state.loginAddress.isNotBlank()) command("discover", mapOf("server" to state.loginAddress)) }))
            val offered = listOfNotNull(if (methods?.sso == true) "single sign-on" else null, if (methods?.password == true) "password" else null, if (methods?.invitation == true) "invitations" else null)
            when {
                state.discovering -> Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) { CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp); Text("Looking for the server…", style = MaterialTheme.typography.bodySmall, color = scheme.onSurfaceVariant) }
                state.discoveryIssue != null -> OnboardingStatus(state.discoveryIssue, ok = false)
                methods != null -> OnboardingStatus(if (offered.isEmpty()) "Sigil server · no sign-in methods enabled" else "Sigil server · " + offered.joinToString(", "))
            }
            if (methods != null) {
                // Only what the server offers: one method ends the card in a single key, several become rows.
                val available = listOfNotNull(if (methods.sso) "sso" else null, if (methods.password) "password" else null, if (methods.invitation) "invitation" else null)
                if (available.size == 1) when (available.single()) {
                    "sso" -> SigilButton({ command("oidc", mapOf("server" to methods.server, "username" to null, "label" to "Android", "replace_devices" to false)) }, Modifier.fillMaxWidth(), enabled = !state.busy) { Text("Sign in with SSO") }
                    "password" -> SigilButton({ method = "password" }, Modifier.fillMaxWidth(), enabled = !state.busy) { Text("Sign in with password") }
                    else -> SigilButton({ method = "invitation" }, Modifier.fillMaxWidth(), enabled = !state.busy) { Text("Use an invitation") }
                } else if (available.isNotEmpty()) {
                    SettingsGroupLabel("Sign in", inset = 0.dp)
                    if (methods.sso) OnboardingOption("login", "Single sign-on", "The account your server gave you", !state.busy) { command("oidc", mapOf("server" to methods.server, "username" to null, "label" to "Android", "replace_devices" to false)) }
                    if (methods.password) OnboardingOption("key", "Password", "A username and password on this server", !state.busy) { method = "password" }
                    if (methods.invitation) OnboardingOption("mail", "Invitation", "A code someone sent you", !state.busy) { method = "invitation" }
                } else Text("This server has no sign-in methods enabled.", style = MaterialTheme.typography.bodySmall, color = scheme.onSurfaceVariant)
            }
        } else {
            OnboardingStatus(methods?.server ?: state.loginAddress, trailing = if (state.phase == "new") ({ SigilTextButton({ method = "" }, enabled = !state.busy) { Text("Change") } }) else null)
            when {
                state.phase == "username" -> {
                    SettingsGroupLabel("Your name on this server", inset = 0.dp)
                    Text("Your account is verified. People will find you by this name.", style = MaterialTheme.typography.bodySmall, color = scheme.onSurfaceVariant)
                    TextField(username, { username = it }, Modifier.fillMaxWidth(), placeholder = { Text("Username") }, singleLine = true, enabled = !state.busy, shape = fieldShape, colors = quiet)
                    SigilButton({ command("username", mapOf("username" to username.trim())) }, Modifier.fillMaxWidth(), enabled = !state.busy && username.isNotBlank()) { Text("Continue") }
                }
                passwordForm && state.phase in listOf("new", "password") -> {
                    SettingsGroupLabel("Password", inset = 0.dp)
                    TextField(username, { username = it }, Modifier.fillMaxWidth(), placeholder = { Text("Username") }, singleLine = true, enabled = !state.busy, shape = fieldShape, colors = quiet)
                    TextField(password, { password = it }, Modifier.fillMaxWidth(), placeholder = { Text("Password") }, singleLine = true, enabled = !state.busy, shape = fieldShape, colors = quiet,
                        visualTransformation = PasswordVisualTransformation(), keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done), keyboardActions = KeyboardActions(onDone = { submitPassword() }))
                    SigilButton(submitPassword, Modifier.fillMaxWidth(), enabled = ready) { Text("Sign in") }
                }
                invitationForm && state.phase == "new" -> {
                    SettingsGroupLabel("Invitation", inset = 0.dp)
                    TextField(invitation, { invitation = it }, Modifier.fillMaxWidth(), placeholder = { Text("Invitation code") }, singleLine = true, enabled = !state.busy, shape = fieldShape, colors = quiet)
                    Text("Paste the code you were sent. It signs this device in and creates your account.", style = MaterialTheme.typography.bodySmall, color = scheme.onSurfaceVariant)
                    SigilButton({ command("enroll", mapOf("server" to methods!!.server, "invitation" to invitation.trim(), "label" to "Android")) }, Modifier.fillMaxWidth(), enabled = !state.busy && invitation.isNotBlank()) { Text("Continue") }
                }
                else -> {
                    Text("Finish signing in with your server.", style = MaterialTheme.typography.bodyMedium)
                    SigilButton({ command("resume", emptyMap()) }, Modifier.fillMaxWidth(), enabled = !state.busy) { Text("Continue sign-in") }
                }
            }
        }
    }
}
@Composable
fun MessageText(source: String, analyze: (String) -> String) {
    val codeFont = LocalCodeFont.current
    val text = remember(source, codeFont) {
        val formats = spans(analyze(source))
        val projection = EditorProjection(source, formats)
        buildAnnotatedString {
            append(projection.text)
            formats.forEach {
                val style = when (it.style) {
                    1 -> SpanStyle(fontWeight = FontWeight.Bold)
                    2 -> SpanStyle(fontStyle = FontStyle.Italic)
                    3 -> SpanStyle(textDecoration = TextDecoration.LineThrough)
                    else -> SpanStyle(fontFamily = codeFont)
                }
                addStyle(style, projection.offsets[it.start], projection.offsets[it.end])
            }
        }
    }
    Text(text, style = MaterialTheme.typography.bodyLarge)
}
internal fun Modifier.selectableChoice(selected: Boolean, action: () -> Unit) = this
    .semantics { this.selected = selected; role = Role.RadioButton }.clickable(onClick = action).heightIn(min = 48.dp)

@Composable
internal fun Toggle(label: String, checked: Boolean, update: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(label, Modifier.weight(1f))
        Switch(checked, update, Modifier.semantics { contentDescription = label })
    }
}
@Composable
internal fun Expandable(visible: Boolean, content: @Composable ColumnScope.() -> Unit) {
    val motionPolicy = LocalMotion.current
    AnimatedVisibility(visible, enter = expandVertically(motionPolicy.enter(MotionMillis), expandFrom = Alignment.Top) + slideInHorizontally(motionPolicy.enter(MotionMillis)) { it } + fadeIn(motionPolicy.enter(MotionMillis)),
        exit = shrinkVertically(motionPolicy.exit(MotionQuick), shrinkTowards = Alignment.Top) + slideOutHorizontally(motionPolicy.exit(MotionQuick)) { it } + fadeOut(motionPolicy.exit(MotionExit)), label = "Expandable section") { Column(content = content) }
}
