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
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
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
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInWindow
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
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
            if (request.status.toInt() == 204) c.resume(JsonNull)
            else if (request.status.toInt() in 200..299 && parsed != null) c.resume(parsed)
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
    var accountOpen by remember { mutableStateOf(false) }
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
                    if (status?.flag("authenticated") == true) HeaderAccount(checkNotNull(status), busy,
                        { accountOpen = true; appearanceOpen = false },
                        { run { api("/auth/v0/admin/logout", "POST"); accountOpen = false; appearanceOpen = false } })
                    else SigilTextButton(onClick = { appearanceOpen = true }) { Text("Appearance") }
                }
                HorizontalDivider(Modifier.padding(top = 18.dp, bottom = 32.dp))
                if (error.isNotEmpty()) Text(error, color = MaterialTheme.colorScheme.error, modifier = Modifier.widthIn(max = 680.dp).padding(bottom = 24.dp))
                val current = status
                if (appearanceOpen) AdminAppearance(appearance, { appearanceOpen = false }) { appearance = it; window.localStorage.setItem("appearance", it.encode()) }
                else if (current == null) { Text(if (busy) "Opening your server…" else "Your server is unavailable."); if (!busy) SigilTextButton(onClick = { run {} }) { Text("Retry") } }
                else if (!current.flag("claimed")) ClaimPage(busy, run)
                else if (!current.flag("authenticated")) LoginPage(current, busy, run)
                else if (!current.flag("complete")) IdentityPage(current, busy, run)
                else if (accountOpen) AccountPage(current, busy, run, { accountOpen = false }, { appearanceOpen = true })
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
    SigilButton(action, Modifier.heightIn(min = 48.dp), enabled = enabled, shape = RoundedCornerShape(10.dp)) { Text(label) }
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
    var step by remember { mutableStateOf(if (status.flag("oidc_enabled")) "choose" else "provider") }
    var username by remember(status.text("suggested_username")) { mutableStateOf(status.text("suggested_username")) }
    var displayName by remember(status.text("suggested_display_name")) { mutableStateOf(status.text("suggested_display_name")) }
    val ready = !busy && username.isNotBlank()
    val submit: () -> Unit = { if (ready) run { api("/auth/v0/admin/finish", "POST", obj("username" to str(username), "display_name" to str(displayName.trim()))) } }
    Page("Your administrator account.", "Choose how you’ll sign in to administer this server.") {
        Text("2 / 3   ·   Your identity", style = MaterialTheme.typography.labelLarge)
        if (step != "local" && !status.flag("oidc_linked")) {
            if (step == "provider") {
                OidcForm(status, busy, run, onSaved = { step = "choose" })
                SigilTextButton(enabled = !busy, onClick = { step = if (status.flag("oidc_enabled")) "choose" else "local" }) {
                    Text(if (status.flag("oidc_enabled")) "Back to account options" else "Continue with a local administrator")
                }
            } else {
                Text("Identity provider saved.", style = MaterialTheme.typography.headlineSmall)
                Text("Choose your administrator’s sign-in method. You can change it later in Authentication settings.")
                BoxWithConstraints(Modifier.fillMaxWidth()) {
                    val choices: @Composable (Modifier) -> Unit = { modifier ->
                        IdentityChoice("Link admin account", "Sign in with your provider to link your identity and profile. Your administrator password stays available until you disable password login.", modifier, !busy) {
                            run { val result = api("/auth/v0/admin/oidc", "POST"); window.location.assign(result.text("authorization_url")) }
                        }
                        IdentityChoice("Create local admin account", "Choose a Sigil username and use the password you already set. Your identity provider remains configured without linking this administrator.", modifier, !busy) { step = "local" }
                    }
                    if (maxWidth >= 560.dp) Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) { choices(Modifier.weight(1f)) }
                    else Column(verticalArrangement = Arrangement.spacedBy(16.dp)) { choices(Modifier.fillMaxWidth()) }
                }
                SigilTextButton(enabled = !busy, onClick = { step = "provider" }) { Text("Edit identity provider") }
            }
        } else {
            if (status.flag("oidc_linked")) Text("Your identity provider is linked. Confirm your Sigil username.")
            Field("Username", username, { username = it }, enabled = !busy)
            Text("@$username:${status.text("server_name")}")
            Field("Display name · optional", displayName, { displayName = it }, enabled = !busy, onSubmit = submit)
            Text("You can change your display name later. Your Sigil address stays the same.", style = MaterialTheme.typography.bodySmall)
            Action("Open my dashboard", ready, submit)
            if (!status.flag("oidc_linked")) SigilTextButton(enabled = !busy, onClick = { step = if (status.flag("oidc_enabled")) "choose" else "provider" }) { Text("Back") }
        }
    }
}
@Composable
private fun IdentityChoice(title: String, description: String, modifier: Modifier, enabled: Boolean, action: () -> Unit) {
    OutlinedCard(modifier) {
        Column(Modifier.padding(20.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Text(title, style = MaterialTheme.typography.headlineSmall)
            Text(description)
            Action(title, enabled, action)
        }
    }
}
@Composable
private fun CopyableCallback(url: String, label: String = "Copy callback URL") {
    var feedback by remember(url) { mutableStateOf("") }
    val scope = rememberCoroutineScope()
    Text(url, Modifier.clickable(onClickLabel = label, role = androidx.compose.ui.semantics.Role.Button) {
        scope.launch(start = CoroutineStart.UNDISPATCHED) {
            try { writeClipboard(url); feedback = "Copied." }
            catch (e: CancellationException) { throw e }
            catch (_: Exception) { feedback = "Could not copy. Clipboard access was not granted." }
        }
    }.padding(vertical = 8.dp), style = MaterialTheme.typography.bodyMedium)
    Text(feedback.ifEmpty { "Click to copy." }, style = MaterialTheme.typography.bodySmall)
}
@Composable
private fun OidcForm(status: JsonElement, busy: Boolean, run: (suspend () -> Unit) -> Unit, onSaved: (() -> Unit)? = null) {
    var unlinkPassword by remember { mutableStateOf("") }
    var issuer by remember { mutableStateOf("") }; var client by remember { mutableStateOf("") }; var secret by remember { mutableStateOf("") }
    var configuration by remember { mutableStateOf<JsonElement?>(null) }
    var transition by remember { mutableStateOf<JsonElement?>(null) }
    var editing by remember(status.flag("oidc_enabled")) { mutableStateOf(onSaved != null || !status.flag("oidc_enabled")) }
    LaunchedEffect(status.flag("oidc_enabled")) { run {
        val saved = api("/admin/v0/oidc")
        configuration = saved; issuer = saved.text("issuer"); client = saved.text("client_id")
        if (onSaved == null) transition = api("/admin/v0/oidc/transition")
    } }
    val ready = !busy && configuration != null && issuer.isNotBlank() && client.isNotBlank()
    val submit: () -> Unit = { if (ready) run {
        val old = checkNotNull(configuration)
        val exceptions = if (issuer.trim() == old.text("issuer")) old.jsonObject["exceptions"] ?: JsonArray(emptyList()) else JsonArray(emptyList())
        configuration = api("/admin/v0/oidc", "PUT", obj("expected_revision" to old.jsonObject.getValue("revision"), "confirm" to JsonPrimitive(true), "provider" to obj("issuer" to str(issuer.trim()), "client_id" to str(client.trim()), "client_secret" to if (secret.isEmpty()) JsonNull else str(secret), "exceptions" to exceptions)))
        secret = ""
        if (onSaved == null) transition = api("/admin/v0/oidc/transition")
        if (onSaved != null) onSaved() else editing = false
    } }
    if (editing) {
        Text("Connect Pocket ID or another OpenID Connect provider.")
        Text("Add this callback URL to your provider:", style = MaterialTheme.typography.bodySmall)
        CopyableCallback("${status.text("public_origin")}/auth/v0/oidc/callback")
        Field("Issuer URL", issuer, { issuer = it }, enabled = !busy)
        Field("Client ID", client, { client = it }, enabled = !busy)
        Field("Client secret · empty for a public client", secret, { secret = it }, secret = true, enabled = !busy, onSubmit = submit)
        if (configuration?.flag("secret_configured") == true) Text("Re-enter the client secret when saving changes.", style = MaterialTheme.typography.bodySmall)
        Action("Save identity provider", ready, submit)
    } else {
        Text("Identity provider saved.", style = MaterialTheme.typography.headlineSmall)
        Text(configuration?.text("issuer").orEmpty())
        Text(if (configuration?.flag("secret_configured") == true) "Client secret saved. It is never displayed again." else "Public client · no client secret.", style = MaterialTheme.typography.bodySmall)
        SigilTextButton(enabled = !busy, onClick = { editing = true }) { Text("Edit identity provider") }
    }
    if (onSaved == null && status.flag("oidc_enabled") && !editing) Action(if (status.flag("oidc_linked")) "Verify identity again" else "Link my administrator identity", !busy) { run {
        val result = api("/auth/v0/admin/oidc", "POST"); window.location.assign(result.text("authorization_url"))
    } }
    if (status.flag("oidc_linked")) {
        Text("To link a different identity, enable password login and unlink this one first.", style = MaterialTheme.typography.bodySmall)
        val canUnlink = !busy && status.flag("password_login") && unlinkPassword.isNotEmpty()
        val unlink: () -> Unit = { if (canUnlink) run {
            api("/auth/v0/admin/oidc/unlink", "POST", obj("password" to str(unlinkPassword))); unlinkPassword = ""
        } }
        Field("Administrator password to unlink", unlinkPassword, { unlinkPassword = it }, secret = true, enabled = !busy, onSubmit = unlink)
        SigilTextButton(enabled = canUnlink, onClick = unlink) { Text("Unlink administrator identity") }
    }
    if (onSaved == null && status.flag("oidc_enabled")) transition?.let { current ->
        HorizontalDivider()
        Text("Changing sign-in access", style = MaterialTheme.typography.headlineSmall)
        Text("Existing users link their provider while signed in to their existing Sigil account. Matching names or email addresses never merge accounts automatically.")
        Text("Before disabling OIDC or replacing its issuer or client ID, prepare the transition. New OIDC registrations pause; existing sign-ins and account linking continue.")
        Text("Users keep their account and can link another device from an existing one. If they lose access, an administrator can issue account access from Users. This does not recover encryption keys or message history.")
        Text("${current.text("linked_accounts")} linked accounts · ${current.text("awaiting_acknowledgement")} awaiting acknowledgement")
        if (!status.flag("password_login")) Text("Enable administrator password login below before preparing a transition.")
        if (current.flag("retiring")) {
            Text("Transition in progress. Each affected user must acknowledge administrator-assisted access from a signed-in device before OIDC can be removed or replaced.")
            for (user in current.jsonObject.getValue("pending").jsonArray) Text("${user.text("username")} · ${user.text("active_devices")} active devices", style = MaterialTheme.typography.bodySmall)
            if (current.text("next_after").isNotEmpty()) SigilTextButton(enabled = !busy, onClick = { run {
                val page = api("/admin/v0/oidc/transition?after=${current.text("next_after")}")
                transition = JsonObject(page.jsonObject.toMutableMap().apply { put("pending", JsonArray(current.jsonObject.getValue("pending").jsonArray + page.jsonObject.getValue("pending").jsonArray)) })
            } }) { Text("Load more affected users") }
        }
        SigilTextButton(enabled = !busy && status.flag("password_login"), onClick = { run {
            transition = api("/admin/v0/oidc/transition", "PUT", obj("configuration_revision" to current.jsonObject.getValue("configuration_revision"), "revision" to current.jsonObject.getValue("revision"), "retiring" to JsonPrimitive(!current.flag("retiring")), "confirm" to JsonPrimitive(true)))
        } }) { Text(if (current.flag("retiring")) "Cancel transition" else "Prepare OIDC transition") }
        SigilTextButton(enabled = !busy, onClick = { run { transition = api("/admin/v0/oidc/transition"); configuration = api("/admin/v0/oidc") } }) { Text("Refresh access review") }
        var disabling by remember { mutableStateOf(false) }
        SigilTextButton(enabled = !busy && status.flag("password_login") && current.text("awaiting_acknowledgement") == "0", onClick = { disabling = true }) { Text("Disable OIDC") }
        if (disabling) Confirmation(title = { Text("Disable identity-provider sign-in?") }, text = { Text("Existing devices remain signed in. New access will require device linking or an administrator-issued account invitation. Your administrator password remains available.") }, confirmButton = { SigilTextButton(enabled = !busy, onClick = { run {
            api("/admin/v0/oidc", "PUT", obj("expected_revision" to current.jsonObject.getValue("configuration_revision"), "provider" to JsonNull, "confirm" to JsonPrimitive(true))); disabling = false
        } }) { Text("Disable OIDC") } }, dismissButton = { SigilTextButton(enabled = !busy, onClick = { disabling = false }) { Text("Cancel") } })
    }
}
@Composable
private fun Dashboard(status: JsonElement, busy: Boolean, run: (suspend () -> Unit) -> Unit) {
    var page by remember { mutableStateOf("Overview") }
    Column(Modifier.widthIn(max = 1100.dp).fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(24.dp)) {
        Text("Your server, at a glance.", style = MaterialTheme.typography.displaySmall)
        Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            for (tab in listOf("Overview", "Users", "Groups", "Authentication", "Server")) AdminTab(tab, page == tab, !busy) { page = tab }
        }
        when (page) {
            "Overview" -> Overview(run)
            "Users" -> Users(busy, run)
            "Groups" -> GroupRecords(busy, run)
            "Authentication" -> Column(Modifier.widthIn(max = 680.dp), verticalArrangement = Arrangement.spacedBy(18.dp)) {
                Text("Sign-in methods", style = MaterialTheme.typography.headlineMedium)
                OidcForm(status, busy, run)
                UserPasswordPolicy(busy, run)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Switch(status.flag("password_login"), modifier = Modifier.semantics { contentDescription = "Allow administrator password login" }, enabled = !busy, onCheckedChange = { enabled -> run { api("/auth/v0/admin/password-login", "POST", obj("enabled" to JsonPrimitive(enabled))) } })
                    Text("Allow administrator password login", Modifier.padding(start = 12.dp))
                }
            }
            "Server" -> ServerSettings(busy, run)
        }
    }
}
@Composable
private fun UserPasswordPolicy(busy: Boolean, run: (suspend () -> Unit) -> Unit) {
    var policy by remember { mutableStateOf<JsonElement?>(null) }
    LaunchedEffect(Unit) { run { policy = api("/admin/v0/password-login") } }
    Row(verticalAlignment = Alignment.CenterVertically) {
        Switch(policy?.flag("enabled") == true, enabled = !busy && policy != null,
            modifier = Modifier.semantics { contentDescription = "Allow user password sign-in" }, onCheckedChange = { enabled -> run {
                policy = api("/admin/v0/password-login", "PUT", obj("revision" to policy!!.jsonObject.getValue("revision"), "enabled" to JsonPrimitive(enabled)))
            } })
        Text("Allow user password sign-in", Modifier.padding(start = 12.dp))
    }
    Text("Users with a password can sign in even when SSO is enabled. Set passwords under Users → Account access. Disabling this leaves existing devices signed in.", style = MaterialTheme.typography.bodySmall)
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
    var access by remember { mutableStateOf<JsonElement?>(null) }
    var invitation by remember { mutableStateOf<JsonElement?>(null) }
    var users by remember { mutableStateOf<List<JsonElement>>(emptyList()) }; var next by remember { mutableStateOf("") }; var selected by remember { mutableStateOf<JsonElement?>(null) }
    suspend fun refresh() { val result = api("/admin/v0/accounts"); users = result.jsonObject.getValue("accounts").jsonArray; next = result.text("next_after") }
    LaunchedEffect(Unit) { run { refresh() } }
    if (deleting == null && selected == null && access == null) {
    Text("People on your server", style = MaterialTheme.typography.headlineMedium)
    if (users.isEmpty()) Text("No users to display.")
    for (user in users) {
        Row(Modifier.fillMaxWidth().padding(vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) { Text(user.text("username"), style = MaterialTheme.typography.titleMedium); Text(if (user.flag("disabled")) "Disabled" else user.text("role"), style = MaterialTheme.typography.bodySmall) }
            if (user.flag("deleted")) Text("Deleted") else {
                SigilTextButton(enabled = !busy && !user.flag("disabled"), onClick = { access = user }) { Text("Account access") }
                SigilTextButton(enabled = !busy, onClick = { selected = user }) { Text(if (user.flag("disabled")) "Enable" else "Disable") }
                SigilTextButton(enabled = !busy, onClick = { deleting = user }) { Text("Delete") }
            }
        }
        HorizontalDivider()
    }
    if (next.isNotEmpty()) SigilTextButton(enabled = !busy, onClick = { run { val result = api("/admin/v0/accounts?after=$next"); users = users + result.jsonObject.getValue("accounts").jsonArray; next = result.text("next_after") } }) { Text("Load more") }
    }
    access?.let { user ->
        Text("Account access for ${user.text("username")}", style = MaterialTheme.typography.headlineMedium)
        var password by remember(user.text("id")) { mutableStateOf("") }
        var saved by remember(user.text("id")) { mutableStateOf(false) }
        Field("Sign-in password · at least 15 characters", password, { password = it; saved = false }, secret = true, enabled = !busy)
        Action("Set sign-in password", !busy && password.length >= 15) { run {
            api("/admin/v0/accounts/${user.text("id")}/password", "PUT", obj("password" to str(password)))
            password = ""; saved = true
        } }
        if (saved) Text("Password saved. User password sign-in must also be enabled in Authentication.")
        Text("Verify the person's identity before sharing an invitation. It grants access to this existing account and revokes its current devices when redeemed. It does not recover encryption keys or message history.")
        val issued = invitation
        if (issued == null) {
            Action("Issue one-hour access invitation", !busy) { run { invitation = api("/admin/v0/accounts/${user.text("id")}/reauthorization-invitations", "POST", obj("expires_in_seconds" to JsonPrimitive(3600))) } }
        } else {
            Text("Share privately. This single-use invitation expires one hour after issue.")
            CopyableCallback(issued.text("secret"), "Copy account access invitation")
            SigilTextButton(enabled = !busy, onClick = { run { api("/admin/v0/invitations/${issued.text("id")}", "DELETE"); invitation = null; access = null } }) { Text("Revoke invitation") }
        }
        SigilTextButton(enabled = !busy, onClick = { invitation = null; access = null }) { Text(if (issued == null) "Back to users" else "Done") }
    }
    deleting?.let { user -> Confirmation(title = { Text("Delete ${user.text("username")}? ") }, text = { Text("Permanently disable this account and schedule removal of its stored messages, attachments and recovery data. The username stays reserved to prevent impersonation. Copies already delivered to other devices remain.") }, confirmButton = { SigilTextButton(enabled = !busy, onClick = { run {
        api("/admin/v0/accounts/${user.text("id")}/delete", "POST", obj("expected_revision" to user.jsonObject.getValue("revision"), "confirm" to JsonPrimitive(true))); deleting = null; refresh()
    } }) { Text("Delete account") } }, dismissButton = { SigilTextButton(enabled = !busy, onClick = { deleting = null }) { Text("Cancel") } }) }
    selected?.let { user -> Confirmation(title = { Text("${if (user.flag("disabled")) "Enable" else "Disable"} ${user.text("username")}? ") }, text = { Text("Disabling an account revokes its devices. Existing downloaded messages remain on their recipients’ devices.") }, confirmButton = { SigilTextButton(enabled = !busy, onClick = { run {
        api("/admin/v0/accounts/${user.text("id")}", "PUT", obj("expected_revision" to user.jsonObject.getValue("revision"), "role" to user.jsonObject.getValue("role"), "disabled" to JsonPrimitive(!user.flag("disabled")), "quota_bytes" to user.jsonObject.getValue("quota_bytes"), "confirm" to JsonPrimitive(true)))
        selected = null; refresh()
    } }) { Text("Confirm") } }, dismissButton = { SigilTextButton(enabled = !busy, onClick = { selected = null }) { Text("Cancel") } }) }
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
            SigilTextButton(enabled = !busy && !group.flag("blocked"), onClick = { selected = group }) { Text("Delete") }
        }
        HorizontalDivider()
    }
    if (groups.size >= 100 && groups.size % 100 == 0) SigilTextButton(enabled = !busy, onClick = { run { groups = groups + api("/admin/v0/group-records?after=${groups.last().text("id")}").jsonArray } }) { Text("Load more") }
    }
    selected?.let { group -> Confirmation(title = { Text("Delete this group record?") }, text = { Text("This permanently removes its encrypted membership and control records from this server and blocks the same group reference from returning. Downloaded messages and copies on other servers remain. This cannot be undone.") }, confirmButton = { SigilTextButton(enabled = !busy, onClick = { run {
        api("/admin/v0/group-records/${group.text("id")}/delete", "POST", obj("expected_revision" to group.jsonObject.getValue("revision"), "confirm" to JsonPrimitive(true))); selected = null; refresh()
    } }) { Text("Delete group record") } }, dismissButton = { SigilTextButton(enabled = !busy, onClick = { selected = null }) { Text("Cancel") } }) }
}

@OptIn(ExperimentalEncodingApi::class)
@Composable
private fun AdminAvatar(status: JsonElement, modifier: Modifier) {
    var image by remember(status.text("username"), status.flag("oidc_linked")) { mutableStateOf<ImageBitmap?>(null) }
    LaunchedEffect(status.text("username"), status.flag("oidc_linked")) {
        runCatching {
            val encoded = api("/auth/v0/admin/avatar").text("image")
            if (encoded.isNotEmpty()) {
                val decoded = org.jetbrains.skia.Image.makeFromEncoded(Base64.decode(encoded))
                if (decoded.width in 1..2048 && decoded.height in 1..2048) image = decoded.toComposeImageBitmap()
            }
        }
    }
    Box(modifier.clip(androidx.compose.foundation.shape.CircleShape).background(MaterialTheme.colorScheme.secondaryContainer), contentAlignment = Alignment.Center) {
        val picture = image
        if (picture != null) Image(picture, null, Modifier.fillMaxSize())
        else Text((status.text("display_name").ifBlank { status.text("username") }.firstOrNull()?.toString() ?: "S").uppercase(), style = MaterialTheme.typography.titleMedium)
    }
}
@Composable
private fun HeaderAccount(status: JsonElement, busy: Boolean, account: () -> Unit, logout: () -> Unit) {
    var open by remember { mutableStateOf(false) }
    var anchor by remember { mutableStateOf(Offset.Zero) }
    val button = remember { FocusRequester() }
    var selected by remember { mutableStateOf(0) }
    SigilIconButton(onClick = { selected = if (status.flag("complete")) 0 else 1; open = !open }, enabled = !busy,
        modifier = Modifier.focusRequester(button).onGloballyPositioned { anchor = it.positionInWindow() + Offset(it.size.width.toFloat(), it.size.height.toFloat()) }
            .semantics { contentDescription = "Your account menu" }) {
        AdminAvatar(status, Modifier.size(40.dp))
    }
    if (open) {
        MenuKeys { key -> when (key) {
            "Escape" -> { open = false; button.requestFocus(); true }
            "ArrowDown", "ArrowUp" -> { selected = if (status.flag("complete")) 1 - selected else 1; true }
            "Enter", " " -> { if (!busy) { open = false; if (selected == 0) account() else logout() }; true }
            "Tab" -> { open = false; false }
            else -> false
        } }
        AdminMenu(anchor, { open = false }, "Your account menu") {
            Text(status.text("display_name").ifBlank { status.text("username").ifBlank { "Your account" } }, Modifier.padding(horizontal = 16.dp, vertical = 8.dp), style = MaterialTheme.typography.titleMedium)
            listOf("Account", "Sign out").forEachIndexed { index, label ->
                DropdownMenuItem(text = { Text(label) }, enabled = !busy && (index == 1 || status.flag("complete")),
                    onClick = { open = false; if (index == 0) account() else logout() },
                    modifier = Modifier.background(if (selected == index) MaterialTheme.colorScheme.surfaceVariant else androidx.compose.ui.graphics.Color.Transparent))
            }
        }
    }
}
@Composable
private fun AccountPage(status: JsonElement, busy: Boolean, run: (suspend () -> Unit) -> Unit, close: () -> Unit, appearance: () -> Unit) {
    var profile by remember { mutableStateOf<JsonElement?>(null) }
    var name by remember { mutableStateOf("") }
    var saved by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) { run { profile = api("/auth/v0/admin/profile"); name = checkNotNull(profile).text("display_name") } }
    val ready = !busy && profile != null && name.trim() != profile?.text("display_name")
    val submit: () -> Unit = { if (ready) run {
        profile = api("/auth/v0/admin/profile", "PUT", obj("revision" to checkNotNull(profile).jsonObject.getValue("revision"), "display_name" to str(name.trim())))
        name = checkNotNull(profile).text("display_name"); saved = true
    } }
    Page("Your account.", "Personal settings for your Sigil account.") {
        Text("@${status.text("username")}:${status.text("server_name")}", style = MaterialTheme.typography.bodyLarge)
        Field("Display name", name, { name = it; saved = false }, enabled = !busy && profile != null, onSubmit = submit)
        Text("Your display name is separate from your permanent Sigil address. Leave it empty to use your username.", style = MaterialTheme.typography.bodySmall)
        Action("Save profile", ready, submit)
        if (saved) Text("Profile saved.")
        HorizontalDivider()
        SigilTextButton(onClick = appearance) { Text("Appearance") }
        Text("Appearance is currently saved only in this browser.", style = MaterialTheme.typography.bodySmall)
        ChangePassword(busy, run)
        SigilTextButton(onClick = close) { Text("Back to Administration") }
    }
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
    Page("Make it feel like you.", "Appearance for this browser. Preview uses sample messages.") {
            Text("Typography")
            for (font in listOf("Newsreader", "Google Sans Flex")) Row(verticalAlignment = Alignment.CenterVertically) { RadioButton(appearance.font == font, { change(appearance.copy(font = font)) }, modifier = Modifier.semantics { contentDescription = font }); Text(font) }
            Text("Appearance")
            for (mode in listOf("System", "Light", "Dark")) Row(verticalAlignment = Alignment.CenterVertically) { RadioButton(appearance.mode == mode, { change(appearance.copy(mode = mode)) }, modifier = Modifier.semantics { contentDescription = mode }); Text(mode) }
            Field("Accent color · hex", accent, { accent = it; parseAccent(it)?.let { color -> change(appearance.copy(accent = color)) } }, onSubmit = close)
            SigilTextButton(onClick = { change(Appearance()); accent = accentText(Appearance().accent) }) { Text("Restore Sigil defaults") }
        TimelinePreview()
        Action("Done", true, close)
    }
}
@Composable
private fun TimelinePreview() {
    OutlinedCard(Modifier.fillMaxWidth()) {
        Column(Modifier.fillMaxWidth().padding(20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("Preview · sample conversation", style = MaterialTheme.typography.labelLarge)
            Surface(shape = RoundedCornerShape(14.dp), color = MaterialTheme.colorScheme.surfaceContainerHighest) {
                Text("Shall we meet at the bookshop?", Modifier.padding(14.dp))
            }
            Surface(Modifier.align(Alignment.End), shape = RoundedCornerShape(14.dp), color = MaterialTheme.colorScheme.primaryContainer) {
                Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("Sounds good. See you at six.")
                    Text("https://example.com", color = MaterialTheme.colorScheme.primary)
                }
            }
            Text("♥ 1", style = MaterialTheme.typography.labelMedium)
            Surface(shape = RoundedCornerShape(10.dp), color = MaterialTheme.colorScheme.surfaceContainerHighest) {
                Text("let greeting = \"Hello, Sigil\";", Modifier.padding(14.dp), fontFamily = LocalCodeFont.current, style = MaterialTheme.typography.bodySmall)
            }
        }
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
