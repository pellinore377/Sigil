package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
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
import androidx.compose.ui.draw.clip
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
        Text(title, Modifier.weight(1f).padding(start = 8.dp), style = MaterialTheme.typography.headlineMedium)
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
internal fun SettingRow(icon: String, title: String, detail: String, click: () -> Unit) {
    Row(Modifier.fillMaxWidth().clickable(onClick = click).padding(horizontal = 20.dp, vertical = 16.dp), verticalAlignment = Alignment.CenterVertically) {
        Glyph(icon)
        Column(Modifier.weight(1f).padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) { Text(title, style = MaterialTheme.typography.titleMedium); Text(detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
        Glyph("chevron_right", 20)
    }
}
@Composable
internal fun SignIn(state: MessengerState, command: (String, Map<String, Any?>) -> Unit) {
    var method by remember(state.loginAddress) { mutableStateOf("") }
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
    val ready = !state.busy && username.isNotBlank() && password.isNotEmpty()
    val submitPassword = { if (ready) command("password", mapOf("server" to (methods?.server ?: state.loginAddress), "username" to username.trim(), "password" to password)) }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(start = 32.dp, end = 32.dp, top = 48.dp, bottom = 144.dp),
        horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(24.dp, Alignment.CenterVertically)) {
        Icon(painterResource(Res.drawable.sigil_mark), null, Modifier.height(100.dp).width(60.dp), tint = MaterialTheme.colorScheme.onBackground)
        Text("Sigil", style = MaterialTheme.typography.displayLarge)
        Spacer(Modifier.height(8.dp))
        OutlinedTextField(state.loginAddress, { command("server_changed", mapOf("server" to it)) }, Modifier.fillMaxWidth().testTag("server-address"),
            label = { Text("Server address") }, singleLine = true, enabled = !state.busy && state.phase == "new",
            shape = RoundedCornerShape(14.dp), keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done, keyboardType = KeyboardType.Uri, capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false),
            keyboardActions = KeyboardActions(onDone = { if (state.loginAddress.isNotBlank()) command("discover", mapOf("server" to state.loginAddress)) }))
        if (state.discovering) CircularProgressIndicator(Modifier.size(22.dp), strokeWidth = 2.dp)
        state.discoveryIssue?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
        AnimatedVisibility(methods != null && state.phase == "new", enter = fadeIn() + expandVertically(), exit = fadeOut() + shrinkVertically()) {
            Column(verticalArrangement = Arrangement.spacedBy(16.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                if (methods?.sso == true) SigilButton({ command("oidc", mapOf("server" to methods.server, "username" to null, "label" to "Android", "replace_devices" to false)) },
                    enabled = !state.busy, modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(14.dp)) { Text("Sign in with SSO") }
                if (methods?.password == true && !passwordForm) SigilOutlinedButton({ method = "password" }, enabled = !state.busy, modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(14.dp)) { Text("Sign in with password") }
                if (methods?.invitation == true && method != "invitation") SigilTextButton({ method = "invitation" }, enabled = !state.busy) { Text("Use an invitation") }
                if (methods != null && !methods.sso && !methods.password && !methods.invitation) Text("This server has no sign-in methods enabled.", style = MaterialTheme.typography.bodySmall)
            }
        }
        AnimatedVisibility(passwordForm) {
            Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
                OutlinedTextField(username, { username = it }, Modifier.fillMaxWidth(), label = { Text("Username") }, singleLine = true, enabled = !state.busy)
                OutlinedTextField(password, { password = it }, Modifier.fillMaxWidth(), label = { Text("Password") }, singleLine = true, enabled = !state.busy,
                    visualTransformation = PasswordVisualTransformation(), keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done), keyboardActions = KeyboardActions(onDone = { submitPassword() }))
                SigilButton(submitPassword, enabled = ready, modifier = Modifier.fillMaxWidth()) { Text("Sign in") }
            }
        }
        if (method == "invitation" && methods?.invitation == true) {
            OutlinedTextField(invitation, { invitation = it }, Modifier.fillMaxWidth(), label = { Text("Invitation") }, singleLine = true, enabled = !state.busy)
            SigilButton({ command("enroll", mapOf("server" to methods.server, "invitation" to invitation.trim(), "label" to "Android")) }, enabled = !state.busy && invitation.isNotBlank()) { Text("Continue") }
        }
        if (state.phase == "username") {
            Text("Choose your Sigil username", style = MaterialTheme.typography.headlineSmall)
            Text("Your SSO account is verified. Choose an available name for this server.", style = MaterialTheme.typography.bodyMedium)
            OutlinedTextField(username, { username = it }, Modifier.fillMaxWidth(), label = { Text("Username") }, singleLine = true, enabled = !state.busy)
            SigilButton({ command("username", mapOf("username" to username.trim())) }, enabled = !state.busy && username.isNotBlank()) { Text("Continue") }
        }
        if (state.phase !in listOf("new", "password", "username")) {
            Text("Finish signing in with your server.", style = MaterialTheme.typography.bodyMedium)
            SigilButton({ command("resume", emptyMap()) }, enabled = !state.busy) { Text("Continue sign-in") }
        }
        SigilTextButton({ command("device_link", mapOf("action" to "join")) }, enabled = !state.busy) { Text("Link to an existing device") }
        if (methods != null || state.phase != "new") SigilTextButton({ command("recovery_account_open", emptyMap()) }, enabled = !state.busy) { Text("Recover a lost account") }
        if (state.phase != "new") SigilTextButton({ command("cancel_login", emptyMap()) }, enabled = !state.busy) { Text("Back to sign-in choices") }
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
    AnimatedVisibility(visible, enter = expandVertically(motionPolicy.tween(MotionMillis), expandFrom = Alignment.Top) + slideInHorizontally(motionPolicy.tween(MotionMillis)) { it } + fadeIn(motionPolicy.tween(MotionMillis)),
        exit = shrinkVertically(motionPolicy.tween(MotionMillis), shrinkTowards = Alignment.Top) + slideOutHorizontally(motionPolicy.tween(MotionMillis)) { it } + fadeOut(motionPolicy.tween(120))) { Column(content = content) }
}
