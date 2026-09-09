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

data class ChatDevice(val id: String, val fingerprint: String, val verified: Boolean, val blocked: Boolean, val changed: Boolean)
data class ChatSummary(val id: String, val address: String, val preview: String, val time: String, val verified: Boolean, val devices: List<ChatDevice>) {
    val name get() = address.removePrefix("@").substringBefore(':')
}
data class ChatMessage(val id: String, val author: String, val text: String, val mine: Boolean, val time: String,
    val delivery: String, val pinned: Boolean, val reactions: List<String>, val myReactions: List<String>, val reply: String?, val readByMe: Boolean)
data class MessengerState(val phase: String = "loading", val address: String = "", val fingerprint: String = "", val device: String = "",
    val chats: List<ChatSummary> = emptyList(), val selected: String? = null, val messages: List<ChatMessage> = emptyList(),
    val more: Boolean = false, val busy: Boolean = false, val issue: String? = null, val sent: Long = 0,
    val loginAddress: String = "", val loginMethods: LoginMethods? = null, val discovering: Boolean = false, val discoveryIssue: String? = null)
data class LoginMethods(val server: String, val sso: Boolean, val password: Boolean, val invitation: Boolean)

@Composable
fun SigilApp(palette: (Int, Boolean) -> String, analyze: (String) -> String, state: MessengerState, command: (String, Map<String, Any?>) -> Unit,
    read: (String) -> String? = { null }, write: (String, String) -> Unit = { _, _ -> }, dynamicAccent: Int? = null,
    onBackAvailable: (Boolean, () -> Unit) -> Unit = { _, _ -> }) {
    var appearance by remember { mutableStateOf(decodeAppearance(read("appearance"))) }
    var page by rememberSaveable { mutableStateOf("") }
    var chatAppearance by remember { mutableStateOf(ChatTheme()) }
    val drafts = remember { mutableMapOf<String, TextFieldState>() }
    val chat = state.chats.find { it.id == state.selected }
    LaunchedEffect(chat?.id) { chatAppearance = decodeChat(read("chat.${chat?.id}")) }
    val back = { if (page.isNotEmpty()) page = "" else command("close", emptyMap()) }
    SideEffect { onBackAvailable(page.isNotEmpty() || state.selected != null, back) }
    SigilTheme(appearance, if (chat != null) chatAppearance else null, dynamicAccent, palette) {
        Surface(color = MaterialTheme.colorScheme.background) {
            Column(Modifier.fillMaxSize().safeDrawingPadding().imePadding()) {
                if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth().height(2.dp))
                state.issue?.let { issue ->
                    Surface(color = MaterialTheme.colorScheme.surfaceVariant) {
                        Row(Modifier.fillMaxWidth().padding(start = 20.dp), verticalAlignment = Alignment.CenterVertically) {
                            Text(issue, Modifier.weight(1f), style = MaterialTheme.typography.bodySmall)
                            Symbol("close", "Dismiss notice") { command("dismiss", emptyMap()) }
                        }
                    }
                }
                when {
                    state.phase == "loading" -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
                    state.phase == "unavailable" -> Box(Modifier.fillMaxSize().padding(32.dp), contentAlignment = Alignment.Center) { Text("Connected messaging is currently available in the Android development build.") }
                    state.phase != "connected" -> SignIn(state, command)
                    page == "appearance" -> AppearancePage(appearance, dynamicAccent != null, { page = "settings" }) {
                        appearance = it; write("appearance", it.encode())
                    }
                    page == "chatAppearance" -> ChatAppearance(chatAppearance, { page = "" }) {
                        chatAppearance = it; write("chat.${chat?.id}", it.encode())
                    }
                    page == "settings" -> Column(Modifier.verticalScroll(rememberScrollState())) {
                        Header("Settings", back)
                        Row(Modifier.padding(24.dp), verticalAlignment = Alignment.CenterVertically) {
                            Avatar(state.address.removePrefix("@"), 56)
                            Column(Modifier.padding(start = 16.dp)) {
                                Text(state.address.substringBefore(':').removePrefix("@"), style = MaterialTheme.typography.headlineSmall)
                                Text(state.address, style = MaterialTheme.typography.bodyMedium)
                            }
                        }
                        SettingRow("palette", "Appearance", "Typeface, colors and display mode") { page = "appearance" }
                        SettingRow("devices", "Devices", "This device's verification fingerprint") { page = "device" }
                        Text("Development build · messaging integration", Modifier.padding(24.dp), style = MaterialTheme.typography.bodySmall)
                    }
                    page == "device" -> Column(Modifier.verticalScroll(rememberScrollState()).padding(bottom = 24.dp)) {
                        Header("This device", { page = "settings" })
                        Column(Modifier.padding(24.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                            Text("Compare this fingerprint with your contact through a trusted channel before approving this device.")
                            SelectionContainer { Text(state.fingerprint.chunked(4).joinToString(" "), fontFamily = LocalCodeFont.current) }
                            Text("Your messaging keys are stored on this device, protected by Android's hardware-backed key store.", style = MaterialTheme.typography.bodyMedium)
                        }
                    }
                    page == "new" -> NewConversation(state, { page = "" }, command) { page = "" }
                    chat != null -> ConversationPage(chat, state, drafts.getOrPut(chat.id) { TextFieldState() }, chatAppearance.gradient, back, { page = "chatAppearance" }, analyze, command)
                    else -> Inbox(state, { command("open", mapOf("peer" to it)) }, { page = "new" }, { page = "settings" })
                }
            }
        }
    }
}

@Composable
private fun Symbol(name: String, label: String, action: () -> Unit) {
    IconButton(action, Modifier.semantics { contentDescription = label }) {
        Text(name, fontFamily = FontFamily(Font(Res.font.material_symbols)), fontSize = 24.sp, modifier = Modifier.clearAndSetSemantics { })
    }
}
@Composable
private fun Header(title: String, back: (() -> Unit)? = null, action: @Composable RowScope.() -> Unit = {}) {
    Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
        if (back != null) Symbol("arrow_back", "Back", back)
        Text(title, Modifier.weight(1f).padding(start = 8.dp), style = MaterialTheme.typography.headlineMedium)
        action()
    }
}
@Composable
private fun Avatar(name: String, size: Int = 48) {
    Surface(Modifier.size(size.dp), shape = CircleShape, color = MaterialTheme.colorScheme.surfaceVariant) {
        Box(contentAlignment = Alignment.Center) { Text(name.take(1).uppercase(), style = MaterialTheme.typography.titleLarge) }
    }
}
@Composable
private fun SettingRow(icon: String, title: String, detail: String, click: () -> Unit) {
    Row(Modifier.fillMaxWidth().clickable(onClick = click).padding(horizontal = 20.dp, vertical = 16.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(icon, fontFamily = FontFamily(Font(Res.font.material_symbols)), fontSize = 24.sp, modifier = Modifier.clearAndSetSemantics { })
        Column(Modifier.weight(1f).padding(horizontal = 16.dp)) { Text(title, style = MaterialTheme.typography.titleMedium); Text(detail, style = MaterialTheme.typography.bodySmall) }
        Text("›")
    }
}
@Composable
private fun SignIn(state: MessengerState, command: (String, Map<String, Any?>) -> Unit) {
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
private fun Inbox(state: MessengerState, open: (String) -> Unit, create: () -> Unit, settings: () -> Unit) {
    var searching by rememberSaveable { mutableStateOf(false) }
    var query by rememberSaveable { mutableStateOf("") }
    Column(Modifier.fillMaxSize()) {
        Header("Sigil") { Symbol("search", "Search conversations") { searching = !searching } }
        if (searching) OutlinedTextField(query, { query = it }, Modifier.fillMaxWidth().padding(horizontal = 20.dp), label = { Text("Search conversations") }, singleLine = true)
        Box(Modifier.weight(1f)) {
            LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 88.dp)) {
                items(state.chats.filter { it.address.contains(query, true) || it.preview.contains(query, true) }, key = { it.id }) { chat ->
                    Row(Modifier.fillMaxWidth().clickable { open(chat.id) }.padding(horizontal = 20.dp, vertical = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                        Avatar(chat.name)
                        Column(Modifier.weight(1f).padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            Text(chat.name, style = MaterialTheme.typography.titleLarge)
                            Text(if (!chat.verified) "Verify devices to start" else chat.preview.ifEmpty { "Start a conversation" }, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyMedium)
                        }
                        Text(chat.time, style = MaterialTheme.typography.labelSmall)
                    }
                }
            }
            if (state.chats.isEmpty()) Column(Modifier.align(Alignment.Center).padding(32.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Text("Your correspondence starts here.", style = MaterialTheme.typography.headlineSmall)
                Spacer(Modifier.height(12.dp)); Text("Add someone by their Sigil address.")
            }
            FloatingActionButton(create, Modifier.align(Alignment.BottomEnd).padding(24.dp), shape = RoundedCornerShape(18.dp), containerColor = MaterialTheme.colorScheme.inverseSurface, contentColor = MaterialTheme.colorScheme.inverseOnSurface) {
                Text("edit_square", fontFamily = FontFamily(Font(Res.font.material_symbols)), fontSize = 26.sp, modifier = Modifier.semantics { contentDescription = "New conversation" })
            }
        }
        Row(Modifier.fillMaxWidth().padding(vertical = 8.dp), horizontalArrangement = Arrangement.SpaceEvenly) {
            TextButton(create) { Text("New conversation") }
            TextButton(settings) { Text("Settings") }
        }
    }
}
@Composable
private fun NewConversation(state: MessengerState, back: () -> Unit, command: (String, Map<String, Any?>) -> Unit, opened: () -> Unit) {
    var address by rememberSaveable { mutableStateOf("") }
    Column(Modifier.fillMaxSize()) {
        Header("New conversation", back)
        Column(Modifier.padding(24.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            OutlinedTextField(address, { address = it }, Modifier.fillMaxWidth(), label = { Text("Sigil address") }, placeholder = { Text("@someone:example.com") }, singleLine = true,
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search), keyboardActions = KeyboardActions(onSearch = { if (!state.busy) command("find", mapOf("address" to address.trim())) }))
            Button({ command("find", mapOf("address" to address.trim())) }, enabled = !state.busy && address.isNotBlank()) { Text("Find person") }
            state.chats.filter { it.address == address.trim() }.forEach { chat ->
                SettingRow("person", chat.name, chat.address) { command("open", mapOf("peer" to chat.id)); opened() }
            }
            Text("Your contact must allow account discovery. Compare device fingerprints before your first message.", style = MaterialTheme.typography.bodyMedium)
        }
    }
}
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun ConversationPage(chat: ChatSummary, state: MessengerState, draft: TextFieldState, gradient: Boolean, back: () -> Unit, appearance: () -> Unit, analyze: (String) -> String, command: (String, Map<String, Any?>) -> Unit) {
    var reply by remember(chat.id) { mutableStateOf<ChatMessage?>(null) }
    var selected by remember(chat.id) { mutableStateOf<ChatMessage?>(null) }
    var verify by remember(chat.id) { mutableStateOf(false) }
    var submitted by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(state.sent) { submitted?.let { if (draft.text.toString() == it) draft.clearText(); submitted = null; reply = null } }
    val scheme = MaterialTheme.colorScheme
    Column(Modifier.fillMaxSize().then(if (gradient) Modifier.background(Brush.verticalGradient(listOf(scheme.background, scheme.primaryContainer))) else Modifier)) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Symbol("arrow_back", "Back", back); Avatar(chat.name, 40)
            Column(Modifier.weight(1f).padding(start = 12.dp)) {
                Text(chat.name, style = MaterialTheme.typography.titleLarge)
                Text(if (chat.verified) "Verified devices" else "Device verification needed", style = MaterialTheme.typography.labelSmall)
            }
            Symbol("verified_user", "Verify devices") { verify = true }
            Symbol("tune", "Conversation appearance", appearance)
        }
        HorizontalDivider(color = scheme.outlineVariant)
        if (!chat.verified) TextButton({ verify = true }, Modifier.align(Alignment.CenterHorizontally)) { Text("Compare and approve devices") }
        LazyColumn(Modifier.weight(1f).fillMaxWidth(), reverseLayout = true, contentPadding = PaddingValues(horizontal = 20.dp, vertical = 16.dp)) {
            items(state.messages, key = { it.author + it.id }) { message ->
                val index = state.messages.indexOf(message)
                val older = state.messages.getOrNull(index + 1)
                val newer = state.messages.getOrNull(index - 1)
                val grouped = older?.mine == message.mine
                Column(Modifier.fillMaxWidth().padding(top = if (grouped) 3.dp else 16.dp)) {
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = if (message.mine) Arrangement.End else Arrangement.Start) {
                        Column(Modifier.widthIn(max = 300.dp), horizontalAlignment = if (message.mine) Alignment.End else Alignment.Start) {
                            Surface(Modifier.combinedClickable(onClick = {}, onLongClick = { selected = message }),
                                shape = RoundedCornerShape(topStart = if (!message.mine && grouped) 6.dp else 20.dp, topEnd = if (message.mine && grouped) 6.dp else 20.dp, bottomEnd = 20.dp, bottomStart = 20.dp),
                                color = if (message.mine) scheme.inverseSurface else scheme.surfaceVariant,
                                contentColor = if (message.mine) scheme.inverseOnSurface else scheme.onSurfaceVariant) {
                                Column(Modifier.padding(horizontal = 16.dp, vertical = 11.dp)) {
                                    message.reply?.let { Text(it, style = MaterialTheme.typography.bodySmall, maxLines = 2, overflow = TextOverflow.Ellipsis); Spacer(Modifier.height(6.dp)) }
                                    MessageText(message.text, analyze)
                                }
                            }
                            if (message.reactions.isNotEmpty() || message.pinned) Surface(shape = RoundedCornerShape(10.dp), color = scheme.surface) {
                                Text((if (message.pinned) "⌖ " else "") + message.reactions.distinct().joinToString(" "), Modifier.padding(horizontal = 6.dp, vertical = 2.dp), style = MaterialTheme.typography.bodySmall)
                            }
                            if (newer?.mine != message.mine || index == 0) Row(Modifier.padding(top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                                Text(listOf(message.time, message.delivery).filter { it.isNotBlank() }.joinToString(" · "), style = MaterialTheme.typography.labelSmall)
                                if (message.mine && message.delivery == "Read") { Spacer(Modifier.width(6.dp)); Avatar(chat.name, 18) }
                            }
                        }
                    }
                }
                if (!message.mine && !message.readByMe && chat.verified) LaunchedEffect(message.id) {
                    command("read", mapOf("peer" to chat.id, "author" to message.author, "message" to message.id))
                }
            }
            if (state.more) item { TextButton({ command("older", emptyMap()) }, Modifier.fillMaxWidth(), enabled = !state.busy) { Text("Earlier messages") } }
        }
        reply?.let { message -> Row(Modifier.fillMaxWidth().padding(start = 20.dp), verticalAlignment = Alignment.CenterVertically) {
            Text("Replying to ${message.text}", Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodySmall)
            Symbol("close", "Cancel reply") { reply = null }
        } }
        Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 10.dp), verticalAlignment = Alignment.Bottom) {
            Composer(draft, analyze, Modifier.weight(1f), showTools = false)
            Spacer(Modifier.width(8.dp))
            FilledIconButton({
                submitted = draft.text.toString()
                command("post", mapOf("peer" to chat.id, "text" to draft.text.toString(), "reply_author" to reply?.author, "reply_message" to reply?.id))
            }, Modifier.size(52.dp), enabled = chat.verified && draft.text.isNotBlank() && !state.busy, shape = RoundedCornerShape(16.dp),
                colors = IconButtonDefaults.filledIconButtonColors(containerColor = scheme.inverseSurface, contentColor = scheme.inverseOnSurface)) {
                Text("arrow_upward", fontFamily = FontFamily(Font(Res.font.material_symbols)), fontSize = 25.sp, modifier = Modifier.semantics { contentDescription = "Send message" })
            }
        }
    }
    if (verify) AlertDialog(onDismissRequest = { verify = false }, title = { Text("Verify ${chat.name}'s devices") },
        text = { Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Text("Compare each fingerprint with your contact through a trusted channel. They must also approve your device.")
            chat.devices.forEach { device ->
                SelectionContainer { Text(device.fingerprint.chunked(4).joinToString(" "), fontFamily = LocalCodeFont.current, style = MaterialTheme.typography.bodySmall) }
                if (device.changed || device.blocked) Text("This device changed or is blocked. Identity recovery must be reviewed before it can be used.")
                else TextButton({ command("confirm", mapOf("peer" to device.id, "fingerprint" to device.fingerprint)) }, enabled = !state.busy) { Text(if (device.verified) "Re-apply sender permission" else "Fingerprints match · approve") }
            }
        } }, confirmButton = { TextButton({ verify = false }) { Text("Done") } })
    selected?.let { message -> AlertDialog(onDismissRequest = { selected = null }, title = { Text("Message") }, text = {
        Column {
            TextButton({ reply = message; selected = null }) { Text("Reply") }
            TextButton({ command("react", mapOf("peer" to chat.id, "author" to message.author, "message" to message.id, "emoji" to "❤️", "active" to ("❤️" !in message.myReactions))); selected = null }) { Text(if ("❤️" in message.myReactions) "Remove ❤️" else "React ❤️") }
            TextButton({ command("pin", mapOf("peer" to chat.id, "author" to message.author, "message" to message.id, "active" to !message.pinned)); selected = null }) { Text(if (message.pinned) "Unpin" else "Pin message") }
        }
    }, confirmButton = { TextButton({ selected = null }) { Text("Close") } }) }
}

@Composable
private fun MessageText(source: String, analyze: (String) -> String) {
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
private fun AppearancePage(value: Appearance, dynamicAvailable: Boolean, back: () -> Unit, update: (Appearance) -> Unit) {
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(bottom = 24.dp)) {
        Header("Appearance", back)
        Column(Modifier.widthIn(max = 680.dp).padding(horizontal = 24.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Text("Your ink. Your paper.", style = MaterialTheme.typography.headlineLarge)
            Text("Appearance on this device. Account-wide appearance sync is not connected yet.")
            Choices("Typeface", listOf("Newsreader", "Google Sans Flex"), value.font) { update(value.copy(font = it)) }
            Choices("Appearance mode", listOf("System", "Light", "Dark"), value.mode) { update(value.copy(mode = it)) }
            if (dynamicAvailable) Toggle("Use Android wallpaper colors", value.dynamic) { update(value.copy(dynamic = it)) }
            AccentPicker(value.accent) { update(value.copy(accent = it, dynamic = false)) }
            Preview()
            TextButton({ update(Appearance()) }) { Text("Reset app appearance") }
        }
    }
}

@Composable
private fun ChatAppearance(value: ChatTheme, back: () -> Unit, update: (ChatTheme) -> Unit) {
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(bottom = 24.dp)) {
        Header("Chat appearance", back)
        Column(Modifier.widthIn(max = 680.dp).padding(horizontal = 24.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Text("Only for you", style = MaterialTheme.typography.headlineLarge)
            Text("Your accent colors the whole conversation. Other participants keep their own theme.")
            Text(if (value.accent == null) "Accent follows the app" else "Custom conversation accent", style = MaterialTheme.typography.bodyMedium)
            AccentPicker(value.accent) { update(value.copy(accent = it)) }
            TextButton({ update(value.copy(accent = null)) }) { Text("Follow app accent") }
            Toggle("Gradient background", value.gradient) { update(value.copy(gradient = it)) }
            Preview()
            TextButton({ update(ChatTheme()) }) { Text("Reset conversation appearance") }
        }
    }
}

@Composable
private fun Choices(label: String, choices: List<String>, selected: String, update: (String) -> Unit) {
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
private fun Modifier.selectableChoice(selected: Boolean, action: () -> Unit) = this
    .semantics { this.selected = selected; role = Role.RadioButton }.clickable(onClick = action).heightIn(min = 48.dp)

@Composable
private fun Toggle(label: String, checked: Boolean, update: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(label, Modifier.weight(1f))
        Switch(checked, update, Modifier.semantics { contentDescription = label })
    }
}

@Composable
private fun AccentPicker(value: Int?, update: (Int) -> Unit) {
    var text by remember(value) { mutableStateOf(value?.let(::accentText) ?: "") }
    var advanced by remember { mutableStateOf(false) }
    val parsed = parseAccent(text)
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text("Accent color", style = MaterialTheme.typography.titleMedium)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf(0x555555, 0x48658c, 0x8b4861, 0x806333, 0x586951).forEach { color ->
                Box(Modifier.size(48.dp).background(androidx.compose.ui.graphics.Color(0xff000000L or color.toLong()), CircleShape)
                    .semantics { contentDescription = "Accent ${accentText(color)}"; selected = value == color }
                    .clickable { update(color) }, contentAlignment = Alignment.Center) {
                    if (value == color) Text("✓", color = androidx.compose.ui.graphics.Color.White)
                }
            }
        }
        TextButton({ advanced = !advanced }) { Text(if (advanced) "Hide advanced color" else "Advanced color") }
        if (advanced) {
        OutlinedTextField(text, { text = it.take(7) }, label = { Text("Hex color") }, prefix = { Text("#") },
            singleLine = true, isError = text.isNotEmpty() && parsed == null,
            supportingText = { Text("Six hexadecimal digits, for example 48658C") })
        TextButton({ parsed?.let(update) }, enabled = parsed != null) { Text("Apply accent") }
        }
    }
}

@Composable
private fun Preview() {
    Column(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.background, RoundedCornerShape(20.dp)).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text("A little correspondence", style = MaterialTheme.typography.titleLarge)
        Surface(shape = RoundedCornerShape(14.dp), color = MaterialTheme.colorScheme.surface) { Text("Room for a thought.", Modifier.padding(14.dp)) }
        Surface(Modifier.align(Alignment.End), shape = RoundedCornerShape(14.dp), color = MaterialTheme.colorScheme.primaryContainer,
            contentColor = MaterialTheme.colorScheme.onPrimaryContainer) { Text("And a thoughtful reply.", Modifier.padding(14.dp)) }
        Text("let thought = \"hello\";", fontFamily = LocalCodeFont.current, style = MaterialTheme.typography.bodyMedium)
    }
}
