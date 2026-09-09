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
import androidx.compose.ui.graphics.Brush
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
import org.jetbrains.compose.resources.Font
import org.jetbrains.compose.resources.painterResource
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.expandVertically
import androidx.compose.animation.shrinkVertically
import androidx.compose.ui.text.input.PasswordVisualTransformation
import kotlinx.coroutines.delay
import sigil.shared.generated.resources.*

@Composable
internal fun Symbol(name: String, label: String, action: () -> Unit) {
    IconButton(action, Modifier.semantics { contentDescription = label }) {
        Glyph(name)
    }
}
@Composable
internal fun Header(title: String, back: (() -> Unit)? = null, action: @Composable RowScope.() -> Unit = {}) {
    Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
        if (back != null) Symbol("arrow_back", "Back", back)
        Text(title, Modifier.weight(1f).padding(start = 8.dp), style = MaterialTheme.typography.headlineMedium)
        action()
    }
}
@Composable
internal fun Avatar(name: String, size: Int = 48) {
    Surface(Modifier.size(size.dp), shape = CircleShape, color = MaterialTheme.colorScheme.surfaceVariant) {
        Box(contentAlignment = Alignment.Center) { Text(name.take(1).uppercase(), fontSize = (size * .43f).sp) }
    }
}
@Composable
internal fun SettingRow(icon: String, title: String, detail: String, click: () -> Unit) {
    Row(Modifier.fillMaxWidth().clickable(onClick = click).padding(horizontal = 20.dp, vertical = 16.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(icon, fontFamily = FontFamily(Font(Res.font.material_symbols)), fontSize = 24.sp, modifier = Modifier.clearAndSetSemantics { })
        Column(Modifier.weight(1f).padding(horizontal = 16.dp)) { Text(title, style = MaterialTheme.typography.titleMedium); Text(detail, style = MaterialTheme.typography.bodySmall) }
        Text("›")
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
        if (state.phase == "new") TextButton({ command("device_link", mapOf("action" to "join")) }, enabled = !state.busy) { Text("Link to an existing device") }
        Spacer(Modifier.height(8.dp))
        OutlinedTextField(state.loginAddress, { command("server_changed", mapOf("server" to it)) }, Modifier.fillMaxWidth().testTag("server-address"),
            label = { Text("Server address") }, singleLine = true, enabled = !state.busy && state.phase == "new",
            shape = RoundedCornerShape(14.dp), keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done, keyboardType = KeyboardType.Uri, capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false),
            keyboardActions = KeyboardActions(onDone = { if (state.loginAddress.isNotBlank()) command("discover", mapOf("server" to state.loginAddress)) }))
        if (state.discovering) CircularProgressIndicator(Modifier.size(22.dp), strokeWidth = 2.dp)
        state.discoveryIssue?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
        AnimatedVisibility(methods != null && state.phase == "new", enter = fadeIn() + expandVertically(), exit = fadeOut() + shrinkVertically()) {
            Column(verticalArrangement = Arrangement.spacedBy(16.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                if (methods?.sso == true) Button({ command("oidc", mapOf("server" to methods.server, "username" to null, "label" to "Android", "replace_devices" to false)) },
                    enabled = !state.busy, modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(14.dp)) { Text("Sign in with SSO") }
                if (methods?.password == true && !passwordForm) OutlinedButton({ method = "password" }, enabled = !state.busy, modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(14.dp)) { Text("Sign in with password") }
                if (methods?.invitation == true && method != "invitation") TextButton({ method = "invitation" }, enabled = !state.busy) { Text("Use an invitation") }
                if (methods != null && !methods.sso && !methods.password && !methods.invitation) Text("This server has no sign-in methods enabled.", style = MaterialTheme.typography.bodySmall)
            }
        }
        AnimatedVisibility(passwordForm) {
            Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
                OutlinedTextField(username, { username = it }, Modifier.fillMaxWidth(), label = { Text("Username") }, singleLine = true, enabled = !state.busy)
                OutlinedTextField(password, { password = it }, Modifier.fillMaxWidth(), label = { Text("Password") }, singleLine = true, enabled = !state.busy,
                    visualTransformation = PasswordVisualTransformation(), keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done), keyboardActions = KeyboardActions(onDone = { submitPassword() }))
                Button(submitPassword, enabled = ready, modifier = Modifier.fillMaxWidth()) { Text("Sign in") }
            }
        }
        if (method == "invitation" && methods?.invitation == true) {
            OutlinedTextField(invitation, { invitation = it }, Modifier.fillMaxWidth(), label = { Text("Invitation") }, singleLine = true, enabled = !state.busy)
            Button({ command("enroll", mapOf("server" to methods.server, "invitation" to invitation.trim(), "label" to "Android")) }, enabled = !state.busy && invitation.isNotBlank()) { Text("Continue") }
        }
        if (state.phase == "username") {
            Text("Choose your Sigil username", style = MaterialTheme.typography.headlineSmall)
            Text("Your SSO account is verified. Choose an available name for this server.", style = MaterialTheme.typography.bodyMedium)
            OutlinedTextField(username, { username = it }, Modifier.fillMaxWidth(), label = { Text("Username") }, singleLine = true, enabled = !state.busy)
            Button({ command("username", mapOf("username" to username.trim())) }, enabled = !state.busy && username.isNotBlank()) { Text("Continue") }
        }
        if (state.phase !in listOf("new", "password", "username")) {
            Text("Finish signing in with your server.", style = MaterialTheme.typography.bodyMedium)
            Button({ command("resume", emptyMap()) }, enabled = !state.busy) { Text("Continue sign-in") }
        }
    }
}
@Composable
internal fun MessageText(source: String, analyze: (String) -> String) {
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
@Composable
internal fun AppearancePage(value: Appearance, dynamicAvailable: Boolean, back: () -> Unit, collections: Boolean = false, setCollections: (Boolean) -> Unit = {}, collectionLabels: Boolean = true, setCollectionLabels: (Boolean) -> Unit = {}, followAccount: Boolean = true, setFollowAccount: (Boolean) -> Unit = {}, update: (Appearance) -> Unit) {
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(bottom = 24.dp)) {
        Header("Appearance", back)
        Column(Modifier.widthIn(max = 680.dp).padding(horizontal = 24.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Text("Your ink. Your paper.", style = MaterialTheme.typography.headlineLarge)
            Text(if (followAccount) "Your appearance follows you across linked devices." else "Appearance for this device.")
            Choices("Typeface", listOf("Newsreader", "Google Sans Flex"), value.font) { update(value.copy(font = it)) }
            Choices("Appearance mode", listOf("System", "Light", "Dark"), value.mode) { update(value.copy(mode = it)) }
            if (dynamicAvailable) Toggle("Use Android wallpaper colors", value.dynamic) { update(value.copy(dynamic = it)) }
            AccentPicker(value.accent) { update(value.copy(accent = it, dynamic = false)) }
            Preview()
            Text("Layout", style = MaterialTheme.typography.titleLarge)
            Toggle("Collections", collections, setCollections)
            if (collections) Toggle("Show collection names", collectionLabels, setCollectionLabels)
            var advanced by remember { mutableStateOf(false) }
            TextButton({ advanced = !advanced }) { Text(if (advanced) "Hide advanced" else "Advanced") }
            if (advanced) Toggle("Follow account appearance on this device", followAccount, setFollowAccount)
            TextButton({ update(Appearance()) }) { Text("Reset app appearance") }
        }
    }
}

@Composable
internal fun ChatAppearance(value: ChatTheme, peer: String, command: Command, back: () -> Unit, update: (ChatTheme) -> Unit) {
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(bottom = 24.dp)) {
        Header("Chat appearance", back)
        Column(Modifier.widthIn(max = 680.dp).padding(horizontal = 24.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Text("Only for you", style = MaterialTheme.typography.headlineLarge)
            Text("Your accent colors the whole conversation. Other participants keep their own theme.")
            Text(if (value.accent == null) "Accent follows the app" else "Custom conversation accent", style = MaterialTheme.typography.bodyMedium)
            AccentPicker(value.accent) { update(value.copy(accent = it)) }
            TextButton({ update(value.copy(accent = null)) }) { Text("Follow app accent") }
            Toggle("Gradient background", value.gradient) { update(value.copy(gradient = it)) }
            Text("Background image", style = MaterialTheme.typography.titleMedium)
            Text("Your image stays on this device. Accent and gradient settings follow your account.", style = MaterialTheme.typography.bodySmall)
            Row { TextButton({ command("attachment_pick", mapOf("peer" to peer, "kind" to "Wallpaper")) }) { Text("Choose image") }; TextButton({ command("wallpaper_remove", mapOf("peer" to peer)) }) { Text("Remove image") } }
            Box(Modifier.clip(RoundedCornerShape(20.dp))) { LocalWallpaper.current(peer, Modifier.matchParentSize()); Preview(false) }
            TextButton({ update(ChatTheme()); command("wallpaper_remove", mapOf("peer" to peer)) }) { Text("Reset conversation appearance") }
        }
    }
}

@Composable
internal fun Choices(label: String, choices: List<String>, selected: String, update: (String) -> Unit) {
    Column {
        Text(label, style = MaterialTheme.typography.titleMedium)
        choices.forEach { choice ->
            Row(Modifier.fillMaxWidth().selectableChoice(choice == selected) { update(choice) }, verticalAlignment = Alignment.CenterVertically) {
                RadioButton(choice == selected, null)
                Text(choice, Modifier.padding(12.dp))
            }
        }
    }
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
internal fun Preview(background: Boolean = true) {
    Column(Modifier.fillMaxWidth().then(if (background) Modifier.background(MaterialTheme.colorScheme.background, RoundedCornerShape(20.dp)) else Modifier).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text("A little correspondence", style = MaterialTheme.typography.titleLarge)
        Surface(shape = RoundedCornerShape(14.dp), color = MaterialTheme.colorScheme.surface) { Text("Room for a thought.", Modifier.padding(14.dp)) }
        Surface(Modifier.align(Alignment.End), shape = RoundedCornerShape(14.dp), color = MaterialTheme.colorScheme.primaryContainer,
            contentColor = MaterialTheme.colorScheme.onPrimaryContainer) { Text("And a thoughtful reply.", Modifier.padding(14.dp)) }
        Text("let thought = \"hello\";", fontFamily = LocalCodeFont.current, style = MaterialTheme.typography.bodyMedium)
    }
}
