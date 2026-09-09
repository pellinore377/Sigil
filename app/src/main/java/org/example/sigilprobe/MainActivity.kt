package org.sigil.compose

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.result.contract.ActivityResultContracts
import androidx.browser.auth.AuthTabIntent
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.runtime.*
import androidx.compose.ui.graphics.toArgb
import androidx.lifecycle.ViewModelProvider
import org.sigil.NativeCore
import org.sigil.SigilApp

class MainActivity : ComponentActivity() {
    private lateinit var messenger: Messenger
    private var pickerPeer: Map<String, Any?>? = null
    private var projectionCall: String? = null
    private val projection = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        messenger.calls.projectionResult(projectionCall, result.data.takeIf { result.resultCode == RESULT_OK }); projectionCall = null
    }
    private val filePicker = registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        val peer = pickerPeer
        pickerPeer = null
        if (uri != null && peer != null) messenger.importFile(peer, uri)
    }
    private val microphone = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted -> messenger.microphoneResult(granted) }
    private val notifications = registerForActivityResult(ActivityResultContracts.RequestPermission()) { messenger.notificationPermissionResult() }
    private val callPermissions = registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { result -> messenger.calls.permissionResult(result.values.all { it }) }
    private val signIn = AuthTabIntent.registerActivityResultLauncher(this) { result ->
        if (result.resultCode == AuthTabIntent.RESULT_OK) messenger.callback(result.resultUri)
    }
    internal fun openSignIn(uri: Uri) = AuthTabIntent.Builder().build().launch(signIn, uri, "sigil")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        messenger = ViewModelProvider(this)[Messenger::class.java]
        savedInstanceState?.getBundle("attachment_target")?.let { saved -> pickerPeer = saved.keySet().associateWith { saved.getString(it) } }
        messenger.callback(intent.data)
        intent.data = null
        val preferences = getSharedPreferences("appearance", MODE_PRIVATE)
        setSigilContent {
            var cameraPeer by remember { mutableStateOf<Map<String, Any?>?>(null) }
            var placePeer by remember { mutableStateOf<Map<String, Any?>?>(null) }
            var backAvailable by remember { mutableStateOf(false) }
            var goBack by remember { mutableStateOf<() -> Unit>({}) }
            BackHandler(backAvailable) { goBack() }
            val dynamicAccent = if (Build.VERSION.SDK_INT >= 31) dynamicLightColorScheme(this).primary.toArgb() and 0xffffff else null
            LaunchedEffect(messenger.authorizationUrl) {
                messenger.authorizationUrl?.let { url ->
                    try { openSignIn(Uri.parse(url)) }
                    catch (_: android.content.ActivityNotFoundException) { }
                    finally { messenger.browserOpened() }
                }
            }
            LaunchedEffect(messenger.picker) {
                messenger.picker?.let { (peer, kind) ->
                    if (kind == "Camera") cameraPeer = peer
                    else if (kind == "Place") placePeer = peer
                    else {
                        pickerPeer = when (kind) { "Wallpaper" -> peer + ("wallpaper" to "true"); "Profile photo" -> peer + ("profile_photo" to "true"); else -> peer }
                        filePicker.launch(when (kind) { "Wallpaper", "Profile photo" -> arrayOf("image/*"); "Photos" -> arrayOf("image/*", "video/*"); else -> arrayOf("*/*") })
                    }
                    messenger.pickerOpened()
                }
            }
            LaunchedEffect(messenger.microphoneRequest) { if (messenger.microphoneRequest != null) microphone.launch(android.Manifest.permission.RECORD_AUDIO) }
            LaunchedEffect(messenger.notificationPermission) { if (messenger.notificationPermission && Build.VERSION.SDK_INT >= 33) notifications.launch(android.Manifest.permission.POST_NOTIFICATIONS) }
            LaunchedEffect(messenger.calls.permissions) { messenger.calls.permissions?.let { (_, fields) -> callPermissions.launch(if (fields["video"] == true) arrayOf(android.Manifest.permission.RECORD_AUDIO, android.Manifest.permission.CAMERA) else arrayOf(android.Manifest.permission.RECORD_AUDIO)) } }
            LaunchedEffect(messenger.calls.projectionRequest) { messenger.calls.projectionRequest?.let { id -> projectionCall = id; projection.launch(getSystemService(android.media.projection.MediaProjectionManager::class.java).createScreenCaptureIntent()) } }
            CompositionLocalProvider(org.sigil.LocalProfilePhoto provides { reference, modifier -> ProfilePhoto(reference, messenger.photoRevision, modifier) }, org.sigil.LocalWallpaper provides { peer, modifier -> Wallpaper(peer, messenger.wallpaperRevision, modifier) }, org.sigil.LocalCallVideo provides { member, screen, modifier -> CallVideoView(messenger.calls, member, screen, modifier) }, org.sigil.LocalAttachmentContent provides { message -> AndroidAttachment(message) }, org.sigil.LocalLocationContent provides { part -> LocationCard(part) }) {
            SigilApp(NativeCore::palette, NativeCore::analyze, messenger.state, messenger::command,
                read = { preferences.getString(it, null) }, write = { key, value -> preferences.edit().putString(key, value).apply() },
                dynamicAccent = dynamicAccent, onBackAvailable = { available, action -> backAvailable = available; goBack = action },
                overlay = {
                    if (messenger.signOutStage.isNotEmpty()) SignOutDialog(messenger.signOutStage, messenger.signOutBusy, messenger.signOutIssue, messenger::signOut)
                    messenger.deviceLink?.let { flow -> DeviceLinkDialog(flow, messenger.state.busy, messenger.state.issue) { action, qr -> messenger.command("device_link", mapOf("action" to action, "qr" to qr)) } }
                    messenger.recoveryKey?.let { secret -> RecoveryDialog(secret, messenger.state.busy, messenger::dismissRecovery) { messenger.command("recovery_enable", mapOf("secret" to secret)) } }
                    if (messenger.restoringRecovery) RestoreRecoveryDialog(messenger.state.busy, messenger.state.issue, messenger::dismissRestoreRecovery) { secret -> messenger.command("recovery_restore", mapOf("secret" to secret, "accept_unanchored" to true)) }
                    if (messenger.recoveringAccount) AccountRecoveryDialog(messenger.state.loginMethods?.sso == true || messenger.state.phase == "oidc", messenger.state.busy, messenger.state.issue, messenger::dismissAccountRecovery) { method, invitation -> messenger.command("recover_account", mapOf("server" to (messenger.state.loginMethods?.server ?: messenger.state.loginAddress), "method" to method, "invitation" to invitation, "confirm_replacement" to true)) }
                    cameraPeer?.let { peer -> CameraSheet({ cameraPeer = null }) { bytes -> messenger.importPhoto(peer, bytes); cameraPeer = null } }
                    placePeer?.let { peer -> PlaceSheet({ placePeer = null }) { fields -> messenger.command("place", fields + peer); placePeer = null } }
                })
            }
        }
    }
    override fun onNewIntent(intent: Intent) { super.onNewIntent(intent); messenger.callback(intent.data); intent.data = null }
    override fun onSaveInstanceState(outState: Bundle) {
        pickerPeer?.let { target -> outState.putBundle("attachment_target", Bundle().apply { target.forEach { (key, value) -> putString(key, value as? String) } }) }
        super.onSaveInstanceState(outState)
    }
    override fun onStart() { super.onStart(); messenger.foreground(true) }
    override fun onUserInteraction() { super.onUserInteraction(); NativeSync.interaction() }
    override fun onStop() { messenger.foreground(false); super.onStop() }
}
