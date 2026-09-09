package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.contentDescription
import kotlinx.browser.window
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.serialization.json.*
import org.w3c.xhr.XMLHttpRequest
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import org.jetbrains.compose.resources.painterResource
import sigil.shared.generated.resources.*
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.toComposeImageBitmap
import androidx.compose.ui.draw.clip
import kotlin.io.encoding.Base64
import kotlin.io.encoding.ExperimentalEncodingApi

private fun obj(vararg pairs: Pair<String, JsonElement>) = JsonObject(pairs.toMap())
private fun str(value: String) = JsonPrimitive(value)
private fun JsonElement.text(key: String) = jsonObject[key]?.jsonPrimitive?.contentOrNull.orEmpty()
private fun JsonElement.flag(key: String) = jsonObject[key]?.jsonPrimitive?.booleanOrNull == true
private suspend fun api(path: String, method: String = "GET", body: JsonElement? = null): JsonElement = suspendCancellableCoroutine { c ->
    val request = XMLHttpRequest()
    request.open(method, path)
    request.timeout = 30000
    request.setRequestHeader("X-Sigil-Admin", "1")
    if (body != null) request.setRequestHeader("Content-Type", "application/json")
    request.onload = {
        if (c.isActive) {
            val parsed = runCatching { Json.parseToJsonElement(request.responseText) }.getOrNull()
            if (request.status.toInt() in 200..299 && parsed != null) c.resume(parsed)
            else c.resumeWithException(IllegalStateException(parsed?.text("message")?.takeIf { it.isNotEmpty() } ?: "Request failed (${request.status}). Please try again."))
        }
    }
    request.onerror = { if (c.isActive) c.resumeWithException(IllegalStateException("Could not reach your server.")) }
    request.ontimeout = { if (c.isActive) c.resumeWithException(IllegalStateException("The request timed out. Check the server before retrying.")) }
    c.invokeOnCancellation { request.abort() }
    if (body == null) request.send() else request.send(body.toString())
}

@Composable
fun AdminApp() {
    var appearance by remember { mutableStateOf(decodeAppearance(window.localStorage.getItem("appearance"))) }
    var appearanceOpen by remember { mutableStateOf(false) }
    var status by remember { mutableStateOf<JsonElement?>(null) }
    var error by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val run: (suspend () -> Unit) -> Unit = { operation ->
        if (!busy) scope.launch {
            busy = true; error = ""
            try { operation(); status = api("/setup/v0/status") }
            catch (e: Exception) { error = e.message ?: "Something went wrong. Please try again." }
            finally { busy = false }
        }
    }
    LaunchedEffect(Unit) { run {} }
    SigilTheme(appearance, palette = ::rustPalette) {
        Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
            AdminMenuHost {
            Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 32.dp, vertical = 24.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Row(Modifier.fillMaxWidth().widthIn(max = 1180.dp), verticalAlignment = Alignment.CenterVertically) {
                    Image(painterResource(if (appearance.mode == "Dark" || (appearance.mode == "System" && isSystemInDarkTheme())) Res.drawable.sigil_dark else Res.drawable.sigil_light), null, Modifier.height(46.dp).width(28.dp))
                    Spacer(Modifier.width(12.dp))
                    Text("Sigil", style = MaterialTheme.typography.headlineLarge)
                    Spacer(Modifier.weight(1f))
                    TextButton(onClick = { appearanceOpen = true }) { Text("Appearance") }
                    if (status?.flag("authenticated") == true) TextButton(enabled = !busy, onClick = { run { api("/auth/v0/admin/logout", "POST") } }) { Text("Sign out") }
                }
                HorizontalDivider(Modifier.padding(top = 18.dp, bottom = 32.dp))
                if (error.isNotEmpty()) Text(error, color = MaterialTheme.colorScheme.error, modifier = Modifier.widthIn(max = 680.dp).padding(bottom = 24.dp))
                val current = status
                if (appearanceOpen) AdminAppearance(appearance, { appearanceOpen = false }) { appearance = it; window.localStorage.setItem("appearance", it.encode()) }
                else if (current == null) { Text(if (busy) "Opening your server…" else "Your server is unavailable."); if (!busy) TextButton(onClick = { run {} }) { Text("Retry") } }
                else if (!current.flag("claimed")) ClaimPage(busy, run)
                else if (!current.flag("authenticated")) LoginPage(current, busy, run)
                else if (!current.flag("complete")) IdentityPage(current, busy, run)
                else Dashboard(current, busy, run)
                if (busy) LinearProgressIndicator(Modifier.widthIn(max = 680.dp).fillMaxWidth().padding(top = 20.dp))
            }
            }
        }
    }
}
@Composable
private fun Page(title: String, subtitle: String, content: @Composable ColumnScope.() -> Unit) {
    Column(Modifier.widthIn(max = 680.dp).fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(18.dp)) {
        Text(title, style = MaterialTheme.typography.displaySmall)
        Text(subtitle, color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.bodyLarge)
        Spacer(Modifier.height(8.dp))
        content()
    }
}
@Composable
private fun Action(label: String, enabled: Boolean, action: () -> Unit) {
    Button(action, Modifier.heightIn(min = 48.dp), enabled = enabled, shape = RoundedCornerShape(10.dp)) { Text(label) }
}
@Composable
private fun ClaimPage(busy: Boolean, run: (suspend () -> Unit) -> Unit) {
    var code by remember { mutableStateOf("") }; var password by remember { mutableStateOf("") }; var confirm by remember { mutableStateOf("") }
    var server by remember { mutableStateOf(window.location.hostname) }
    val ready = !busy && code.isNotBlank() && password.isNotBlank() && password == confirm
    val submit: () -> Unit = { if (ready) run {
        api("/setup/v0/claim", "POST", obj("code" to str(code), "password" to str(password), "server_name" to str(server), "public_origin" to str(window.location.origin)))
        code = ""; password = ""; confirm = ""
    } }
    Page("A place for your correspondence.", "Welcome to your Sigil server. Let’s make it yours.") {
        Text("1 / 3   ·   Secure your server", style = MaterialTheme.typography.labelLarge)
        Field("One-time code from container logs", code, { code = it }, secret = true, enabled = !busy)
        Field("Identity domain", server, { server = it }, enabled = !busy)
        Text("Your address will look like @you:$server. This domain cannot change after setup.", style = MaterialTheme.typography.bodySmall)
        Field("Administrator password · at least 15 characters", password, { password = it }, secret = true, enabled = !busy)
        Field("Repeat password", confirm, { confirm = it }, secret = true, enabled = !busy, onSubmit = submit)
        Action("Secure my server", ready, submit)
    }
}
@Composable
private fun LoginPage(status: JsonElement, busy: Boolean, run: (suspend () -> Unit) -> Unit) {
    var username by remember { mutableStateOf("") }; var password by remember { mutableStateOf("") }
    val ready = !busy && password.isNotEmpty()
    val submit: () -> Unit = { if (ready) run { api("/auth/v0/admin/login", "POST", obj("username" to str(username), "password" to str(password))); password = "" } }
    Page("Welcome back.", "Sign in to look after your Sigil server.") {
        if (status.flag("oidc_login")) Action("Sign in with your identity provider", !busy) { run { val result = api("/auth/v0/admin/oidc", "POST"); window.location.assign(result.text("authorization_url")) } }
        if (status.flag("password_login")) {
            if (status.flag("complete")) Field("Administrator username", username, { username = it }, enabled = !busy)
            Field("Password", password, { password = it }, secret = true, enabled = !busy, onSubmit = submit)
            Action("Sign in", ready, submit)
        }
    }
}
@Composable
private fun IdentityPage(status: JsonElement, busy: Boolean, run: (suspend () -> Unit) -> Unit) {
    var local by remember { mutableStateOf(false) }
    var username by remember(status.text("suggested_username")) { mutableStateOf(status.text("suggested_username")) }
    val ready = !busy && username.isNotBlank()
    val submit: () -> Unit = { if (ready) run { api("/auth/v0/admin/finish", "POST", obj("username" to str(username))) } }
    Page("Your administrator account.", "Choose how you’ll sign in to administer this server.") {
        Text("2 / 3   ·   Your identity", style = MaterialTheme.typography.labelLarge)
        if (!local && !status.flag("oidc_linked")) {
            OidcForm(status, busy, run)
            TextButton(enabled = !busy, onClick = { local = true }) { Text("Continue with a local administrator") }
        } else {
            if (status.flag("oidc_linked")) Text("Your identity provider is linked. Confirm your Sigil username.")
            Field("Username", username, { username = it }, enabled = !busy, onSubmit = submit)
            Text("@$username:${status.text("server_name")}")
            Action("Open my dashboard", ready, submit)
        }
    }
}
@Composable
private fun OidcForm(status: JsonElement, busy: Boolean, run: (suspend () -> Unit) -> Unit) {
    var unlinkPassword by remember { mutableStateOf("") }
    var issuer by remember { mutableStateOf("") }; var client by remember { mutableStateOf("") }; var secret by remember { mutableStateOf("") }
    var configuration by remember { mutableStateOf<JsonElement?>(null) }
    LaunchedEffect(Unit) { run {
        val saved = api("/admin/v0/oidc")
        configuration = saved; issuer = saved.text("issuer"); client = saved.text("client_id")
    } }
    val ready = !busy && configuration != null && issuer.isNotBlank() && client.isNotBlank()
    val submit: () -> Unit = { if (ready) run {
        val old = checkNotNull(configuration)
        val exceptions = if (issuer.trim() == old.text("issuer")) old.jsonObject["exceptions"] ?: JsonArray(emptyList()) else JsonArray(emptyList())
        configuration = api("/admin/v0/oidc", "PUT", obj("expected_revision" to old.jsonObject.getValue("revision"), "confirm" to JsonPrimitive(true), "provider" to obj("issuer" to str(issuer.trim()), "client_id" to str(client.trim()), "client_secret" to if (secret.isEmpty()) JsonNull else str(secret), "exceptions" to exceptions)))
        secret = ""
    } }
    Text("Connect Pocket ID or another OpenID Connect provider.")
    Text("Add this callback URL to your provider:", style = MaterialTheme.typography.bodySmall)
    Field("Callback URL", "${status.text("public_origin")}/auth/v0/oidc/callback", {}, readOnly = true)
    Field("Issuer URL", issuer, { issuer = it }, enabled = !busy)
    Field("Client ID", client, { client = it }, enabled = !busy)
    Field("Client secret · empty for a public client", secret, { secret = it }, secret = true, enabled = !busy, onSubmit = submit)
    if (configuration?.flag("secret_configured") == true) Text("Re-enter the client secret when saving changes.", style = MaterialTheme.typography.bodySmall)
    Action("Save identity provider", ready, submit)
    if (status.flag("oidc_enabled")) Action(if (status.flag("oidc_linked")) "Verify identity again" else "Link my administrator identity", !busy) { run {
        val result = api("/auth/v0/admin/oidc", "POST"); window.location.assign(result.text("authorization_url"))
    } }
    if (status.flag("oidc_linked")) {
        Text("To link a different identity, enable password login and unlink this one first.", style = MaterialTheme.typography.bodySmall)
        val canUnlink = !busy && status.flag("password_login") && unlinkPassword.isNotEmpty()
        val unlink: () -> Unit = { if (canUnlink) run {
            api("/auth/v0/admin/oidc/unlink", "POST", obj("password" to str(unlinkPassword))); unlinkPassword = ""
        } }
        Field("Administrator password to unlink", unlinkPassword, { unlinkPassword = it }, secret = true, enabled = !busy, onSubmit = unlink)
        TextButton(enabled = canUnlink, onClick = unlink) { Text("Unlink administrator identity") }
    }
}
@Composable
private fun Dashboard(status: JsonElement, busy: Boolean, run: (suspend () -> Unit) -> Unit) {
    var page by remember { mutableStateOf("Overview") }
    Column(Modifier.widthIn(max = 1100.dp).fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(24.dp)) {
        Text("Your server, at a glance.", style = MaterialTheme.typography.displaySmall)
        AdminAvatar()
        Text("@${status.text("username")}:${status.text("server_name")}", color = MaterialTheme.colorScheme.onSurfaceVariant)
        Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            for (tab in listOf("Overview", "Users", "Groups", "Authentication", "Server")) FilterChip(page == tab, { page = tab }, { Text(tab) }, enabled = !busy)
        }
        when (page) {
            "Overview" -> Overview(run)
            "Users" -> Users(busy, run)
            "Groups" -> GroupRecords(busy, run)
            "Authentication" -> Column(Modifier.widthIn(max = 680.dp), verticalArrangement = Arrangement.spacedBy(18.dp)) {
                Text("Sign-in methods", style = MaterialTheme.typography.headlineMedium)
                OidcForm(status, busy, run)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Switch(status.flag("password_login"), modifier = Modifier.semantics { contentDescription = "Allow administrator password login" }, enabled = !busy, onCheckedChange = { enabled -> run { api("/auth/v0/admin/password-login", "POST", obj("enabled" to JsonPrimitive(enabled))) } })
                    Text("Allow administrator password login", Modifier.padding(start = 12.dp))
                }
                if (status.flag("oidc_enabled")) TextButton(enabled = !busy && status.flag("password_login"), onClick = { run {
                    val old = api("/admin/v0/oidc"); api("/admin/v0/oidc", "PUT", obj("expected_revision" to old.jsonObject.getValue("revision"), "provider" to JsonNull, "confirm" to JsonPrimitive(true)))
                } }) { Text("Disable OIDC") }
                ChangePassword(busy, run)
            }
            "Server" -> ServerSettings(busy, run)
        }
    }
}
@Composable
private fun Overview(run: (suspend () -> Unit) -> Unit) {
    var diagnostics by remember { mutableStateOf<JsonElement?>(null) }
    LaunchedEffect(Unit) { run { diagnostics = api("/admin/v0/diagnostics") } }
    Text("Encrypted correspondence", style = MaterialTheme.typography.headlineMedium)
    Text("This server stores encrypted correspondence. Administration cannot read messages or reveal private group membership.")
    diagnostics?.let { data ->
        for ((key, label) in listOf("accounts" to "Active accounts", "devices" to "Active devices", "mailbox_pending" to "Messages waiting for delivery", "federation_pending" to "Federated deliveries pending", "federation_failed_peers" to "Servers needing attention", "push_pending" to "Notifications pending", "attachment_bytes" to "Encrypted attachments", "recovery_bytes" to "Encrypted recovery data", "database_bytes" to "Database storage", "version" to "Server version")) {
            val value = data.text(key)
            val displayed = if (key.endsWith("_bytes")) value.toLongOrNull()?.let { if (it >= 1048576) "${it / 1048576} MiB" else "${it / 1024} KiB" } ?: value else value
            Row(Modifier.fillMaxWidth().padding(vertical = 6.dp), horizontalArrangement = Arrangement.SpaceBetween) { Text(label, Modifier.weight(1f)); Text(displayed) }
        }
    }
}
@Composable
private fun Users(busy: Boolean, run: (suspend () -> Unit) -> Unit) {
    var deleting by remember { mutableStateOf<JsonElement?>(null) }
    var users by remember { mutableStateOf<List<JsonElement>>(emptyList()) }; var next by remember { mutableStateOf("") }; var selected by remember { mutableStateOf<JsonElement?>(null) }
    suspend fun refresh() { val result = api("/admin/v0/accounts"); users = result.jsonObject.getValue("accounts").jsonArray; next = result.text("next_after") }
    LaunchedEffect(Unit) { run { refresh() } }
    if (deleting == null && selected == null) {
    Text("People on your server", style = MaterialTheme.typography.headlineMedium)
    if (users.isEmpty()) Text("No users to display.")
    for (user in users) {
        Row(Modifier.fillMaxWidth().padding(vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) { Text(user.text("username"), style = MaterialTheme.typography.titleMedium); Text(if (user.flag("disabled")) "Disabled" else user.text("role"), style = MaterialTheme.typography.bodySmall) }
            if (user.flag("deleted")) Text("Deleted") else {
                TextButton(enabled = !busy, onClick = { selected = user }) { Text(if (user.flag("disabled")) "Enable" else "Disable") }
                TextButton(enabled = !busy, onClick = { deleting = user }) { Text("Delete") }
            }
        }
        HorizontalDivider()
    }
    if (next.isNotEmpty()) TextButton(enabled = !busy, onClick = { run { val result = api("/admin/v0/accounts?after=$next"); users = users + result.jsonObject.getValue("accounts").jsonArray; next = result.text("next_after") } }) { Text("Load more") }
    }
    deleting?.let { user -> Confirmation(title = { Text("Delete ${user.text("username")}? ") }, text = { Text("Permanently disable this account and schedule removal of its stored messages, attachments and recovery data. The username stays reserved to prevent impersonation. Copies already delivered to other devices remain.") }, confirmButton = { TextButton(enabled = !busy, onClick = { run {
        api("/admin/v0/accounts/${user.text("id")}/delete", "POST", obj("expected_revision" to user.jsonObject.getValue("revision"), "confirm" to JsonPrimitive(true))); deleting = null; refresh()
    } }) { Text("Delete account") } }, dismissButton = { TextButton(enabled = !busy, onClick = { deleting = null }) { Text("Cancel") } }) }
    selected?.let { user -> Confirmation(title = { Text("${if (user.flag("disabled")) "Enable" else "Disable"} ${user.text("username")}? ") }, text = { Text("Disabling an account revokes its devices. Existing downloaded messages remain on their recipients’ devices.") }, confirmButton = { TextButton(enabled = !busy, onClick = { run {
        api("/admin/v0/accounts/${user.text("id")}", "PUT", obj("expected_revision" to user.jsonObject.getValue("revision"), "role" to user.jsonObject.getValue("role"), "disabled" to JsonPrimitive(!user.flag("disabled")), "quota_bytes" to user.jsonObject.getValue("quota_bytes"), "confirm" to JsonPrimitive(true)))
        selected = null; refresh()
    } }) { Text("Confirm") } }, dismissButton = { TextButton(enabled = !busy, onClick = { selected = null }) { Text("Cancel") } }) }
}
@Composable
private fun ServerSettings(busy: Boolean, run: (suspend () -> Unit) -> Unit) {
    var policy by remember { mutableStateOf<JsonObject?>(null) }
    var limit by remember { mutableStateOf("") }; var daily by remember { mutableStateOf("") }; var registration by remember { mutableStateOf("closed") }
    LaunchedEffect(Unit) { run { val p = api("/admin/v0/policy"); policy = p.jsonObject; limit = p.text("max_accounts"); daily = p.text("registrations_per_day"); registration = p.text("registration") } }
    val ready = !busy && policy != null && limit.toIntOrNull() != null && daily.toIntOrNull() != null
    val submit: () -> Unit = { if (ready) run {
        val updated = JsonObject(policy!!.toMutableMap().apply { put("max_accounts", JsonPrimitive(limit.toInt())); put("registrations_per_day", JsonPrimitive(daily.toInt())); put("registration", str(registration)) })
        policy = api("/admin/v0/policy", "PUT", updated).jsonObject
    } }
    Column(Modifier.widthIn(max = 680.dp), verticalArrangement = Arrangement.spacedBy(18.dp)) {
        Text("A considered welcome.", style = MaterialTheme.typography.headlineMedium)
        Text("Choose who can join, and how quickly your server can grow.")
        for ((value, label) in listOf("closed" to "Registration closed", "invitations" to "Invitation only", "oidc" to "Identity provider")) Row(verticalAlignment = Alignment.CenterVertically) { RadioButton(registration == value, { registration = value }, modifier = Modifier.semantics { contentDescription = label }, enabled = !busy); Text(label) }
        Field("Maximum accounts", limit, { limit = it }, enabled = !busy)
        Field("New accounts per day", daily, { daily = it }, enabled = !busy, onSubmit = submit)
        Action("Save server settings", ready, submit)
    }
}

@Composable
private fun GroupRecords(busy: Boolean, run: (suspend () -> Unit) -> Unit) {
    var groups by remember { mutableStateOf<List<JsonElement>>(emptyList()) }
    var selected by remember { mutableStateOf<JsonElement?>(null) }
    suspend fun refresh() { groups = api("/admin/v0/group-records").jsonArray }
    LaunchedEffect(Unit) { run { refresh() } }
    if (selected == null) {
    Text("Encrypted group records", style = MaterialTheme.typography.headlineMedium)
    Text("Group names and participant identities are private. These references identify records stored on this server. Direct conversations have no server-visible chat directory.")
    if (groups.isEmpty()) Text("No group records on this server.")
    for (group in groups) {
        Row(Modifier.fillMaxWidth().padding(vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                androidx.compose.foundation.text.selection.SelectionContainer { Text(group.text("id"), style = MaterialTheme.typography.bodySmall, fontFamily = LocalCodeFont.current) }
                Text(if (group.flag("blocked")) "Deleted · recreation blocked" else "Active", style = MaterialTheme.typography.bodySmall)
            }
            TextButton(enabled = !busy && !group.flag("blocked"), onClick = { selected = group }) { Text("Delete") }
        }
        HorizontalDivider()
    }
    if (groups.size >= 100 && groups.size % 100 == 0) TextButton(enabled = !busy, onClick = { run { groups = groups + api("/admin/v0/group-records?after=${groups.last().text("id")}").jsonArray } }) { Text("Load more") }
    }
    selected?.let { group -> Confirmation(title = { Text("Delete this group record?") }, text = { Text("This permanently removes its encrypted membership and control records from this server and blocks the same group reference from returning. Downloaded messages and copies on other servers remain. This cannot be undone.") }, confirmButton = { TextButton(enabled = !busy, onClick = { run {
        api("/admin/v0/group-records/${group.text("id")}/delete", "POST", obj("expected_revision" to group.jsonObject.getValue("revision"), "confirm" to JsonPrimitive(true))); selected = null; refresh()
    } }) { Text("Delete group record") } }, dismissButton = { TextButton(enabled = !busy, onClick = { selected = null }) { Text("Cancel") } }) }
}

@OptIn(ExperimentalEncodingApi::class)
@Composable
private fun AdminAvatar() {
    var image by remember { mutableStateOf<ImageBitmap?>(null) }
    LaunchedEffect(Unit) {
        runCatching {
            val encoded = api("/auth/v0/admin/avatar").text("image")
            if (encoded.isNotEmpty()) {
                val decoded = org.jetbrains.skia.Image.makeFromEncoded(Base64.decode(encoded))
                if (decoded.width in 1..2048 && decoded.height in 1..2048) image = decoded.toComposeImageBitmap()
            }
        }
    }
    image?.let { Image(it, "Administrator profile picture", Modifier.size(64.dp).clip(androidx.compose.foundation.shape.CircleShape)) }
}
@Composable
private fun ChangePassword(busy: Boolean, run: (suspend () -> Unit) -> Unit) {
    var current by remember { mutableStateOf("") }; var replacement by remember { mutableStateOf("") }; var confirm by remember { mutableStateOf("") }
    var saved by remember { mutableStateOf(false) }
    val ready = !busy && current.isNotEmpty() && replacement.isNotEmpty() && replacement == confirm
    val submit: () -> Unit = { if (ready) run {
        api("/auth/v0/admin/password", "POST", obj("current" to str(current), "replacement" to str(replacement)))
        current = ""; replacement = ""; confirm = ""; saved = true
    } }
    HorizontalDivider()
    Text("Change administrator password", style = MaterialTheme.typography.headlineSmall)
    Field("Current password", current, { current = it; saved = false }, secret = true, enabled = !busy)
    Field("New password · at least 15 characters", replacement, { replacement = it; saved = false }, secret = true, enabled = !busy)
    Field("Repeat new password", confirm, { confirm = it }, secret = true, enabled = !busy, onSubmit = submit)
    Action("Update password", ready, submit)
    if (saved) Text("Password updated. Other browser sessions have been signed out.")
}

@Composable
private fun AdminAppearance(appearance: Appearance, close: () -> Unit, change: (Appearance) -> Unit) {
    var accent by remember { mutableStateOf(accentText(appearance.accent)) }
    Page("Make it feel like you.", "Choose your typography, appearance and accent.") {
            Text("Typography")
            for (font in listOf("Newsreader", "Google Sans Flex")) Row(verticalAlignment = Alignment.CenterVertically) { RadioButton(appearance.font == font, { change(appearance.copy(font = font)) }, modifier = Modifier.semantics { contentDescription = font }); Text(font) }
            Text("Appearance")
            for (mode in listOf("System", "Light", "Dark")) Row(verticalAlignment = Alignment.CenterVertically) { RadioButton(appearance.mode == mode, { change(appearance.copy(mode = mode)) }, modifier = Modifier.semantics { contentDescription = mode }); Text(mode) }
            Field("Accent color · hex", accent, { accent = it; parseAccent(it)?.let { color -> change(appearance.copy(accent = color)) } }, onSubmit = close)
            TextButton(onClick = { change(Appearance()); accent = accentText(Appearance().accent) }) { Text("Restore Sigil defaults") }
        Action("Done", true, close)
    }
}

@Composable
private fun Confirmation(title: @Composable () -> Unit, text: @Composable () -> Unit, confirmButton: @Composable () -> Unit, dismissButton: @Composable () -> Unit) {
    Column(Modifier.widthIn(max = 680.dp).fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(18.dp)) {
        ProvideTextStyle(MaterialTheme.typography.headlineSmall, title)
        text()
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) { dismissButton(); confirmButton() }
    }
}
