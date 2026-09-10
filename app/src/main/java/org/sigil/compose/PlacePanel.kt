package org.sigil.compose

import android.Manifest
import android.content.pm.PackageManager
import android.location.*
import android.os.*
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.*
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.*
import androidx.compose.ui.text.input.*
import androidx.compose.ui.unit.dp
import androidx.lifecycle.*
import androidx.lifecycle.compose.LocalLifecycleOwner
import kotlinx.coroutines.*
import kotlin.math.roundToInt
import org.sigil.*

@Composable
internal fun PlacePanel(close: () -> Unit, photo: String = "", send: suspend (Map<String, Any?>) -> Boolean) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val scope = rememberCoroutineScope()
    val motion = LocalMotion.current
    var point by remember { mutableStateOf<Pair<Double, Double>?>(null) }
    var sample by remember { mutableStateOf<Location?>(null) }
    var label by rememberSaveable { mutableStateOf("") }
    var mode by rememberSaveable { mutableStateOf("once") }
    var duration by rememberSaveable { mutableStateOf("fifteen_minutes") }
    var coordinates by rememberSaveable { mutableStateOf(false) }
    var latitude by rememberSaveable { mutableStateOf("") }
    var longitude by rememberSaveable { mutableStateOf("") }
    var locating by remember { mutableStateOf(false) }
    var sending by remember { mutableStateOf(false) }
    var issue by remember { mutableStateOf<String?>(null) }
    var mapFailed by remember { mutableStateOf(false) }
    val keyboard = LocalSoftwareKeyboardController.current
    val focus = LocalFocusManager.current
    DisposableEffect(lifecycle) {
        val observer = LifecycleEventObserver { _, event -> if (event == Lifecycle.Event.ON_STOP) locating = false }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer) }
    }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { result ->
        locating = result[Manifest.permission.ACCESS_COARSE_LOCATION] == true || result[Manifest.permission.ACCESS_FINE_LOCATION] == true
        if (!locating) issue = "Allow location access, or drop a pin without sharing your location."
    }
    val notifications = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { allowed ->
        issue = if (allowed) "Notifications enabled. Tap Share live location when ready." else "Live sharing needs notifications so you can stop it while Sigil is closed."
    }
    fun locate() {
        issue = null
        if (NativeLocations.permitted(context)) locating = true
        else permission.launch(arrayOf(Manifest.permission.ACCESS_FINE_LOCATION, Manifest.permission.ACCESS_COARSE_LOCATION))
    }
    LaunchedEffect(locating) { if (locating) { delay(20000); locating = false; issue = "No location fix yet. Try again, or drop a pin." } }
    DisposableEffect(locating) {
        val manager = context.getSystemService(LocationManager::class.java)
        val listener = object : LocationListener {
            override fun onLocationChanged(value: Location) {
                if (locating && SystemClock.elapsedRealtimeNanos() - value.elapsedRealtimeNanos in 0..60_000_000_000L) {
                    sample = value; point = value.latitude to value.longitude; locating = false; issue = null
                }
            }
            override fun onStatusChanged(provider: String?, status: Int, extras: Bundle?) {}
            override fun onProviderEnabled(provider: String) {}
            override fun onProviderDisabled(provider: String) {}
        }
        if (locating) try {
            val providers = locationProviders(context, manager)
            if (providers.isEmpty()) { locating = false; issue = "Enable location services, or drop a pin." }
            providers.forEach { manager.requestLocationUpdates(it, 1000L, 0f, listener, Looper.getMainLooper()) }
        } catch (_: SecurityException) { locating = false; issue = "Location permission is needed." }
        onDispose { manager.removeUpdates(listener) }
    }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            SigilIconButton(close) { Glyph("chevron_left", 24, "Back to attachments") }
            Text("Share a place", Modifier.weight(1f), style = MaterialTheme.typography.titleMedium)
        }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf("once" to "Once", "live" to "Live", "pin" to "Pin").forEach { (id, name) ->
                FilterChip(mode == id, { mode = id; issue = null; if (id != "pin") { point = sample?.let { it.latitude to it.longitude } } }, label = { Text(name) }, shape = RoundedCornerShape(14.dp))
            }
        }
        Text(when (mode) { "live" -> "Share updates from this device until the time runs out. You can stop at any time."; "pin" -> "Tap the map to choose a place. Your device’s location is not shared."; else -> "Send your current location once. It will not update." }, style = MaterialTheme.typography.bodySmall)
        if (!mapFailed) Box(Modifier.fillMaxWidth().height(164.dp).clip(RoundedCornerShape(20.dp))) {
            ServerMap(Modifier.fillMaxSize(), point?.first ?: 0.0, point?.second ?: 0.0, selected = point != null, marker = if (mode != "pin") { { LocationAvatar("You", photo, false) } } else null, chosen = { lat, lon ->
                mode = "pin"; point = lat to lon
            }, failure = { mapFailed = true })
        } else Text("Map unavailable. You can still send your location or enter pin coordinates.", style = MaterialTheme.typography.bodySmall)
        if (mode != "pin") SigilOutlinedButton(::locate, enabled = !locating && !sending) {
            Glyph("my_location", 22); Spacer(Modifier.width(8.dp)); Text(if (locating) "Finding location…" else if (sample == null) "Use my location" else "Refresh location")
        }
        if (mode == "live") Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Share for", style = MaterialTheme.typography.labelMedium)
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                listOf("fifteen_minutes" to "15 min", "hour" to "1 hour", "eight_hours" to "8 hours").forEach { (id, name) -> FilterChip(duration == id, { duration = id }, label = { Text(name) }, shape = RoundedCornerShape(14.dp)) }
            }
            SigilTextButton({ context.startActivity(android.content.Intent(android.provider.Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(android.provider.Settings.EXTRA_APP_PACKAGE, context.packageName)) }) { Text("Notification settings") }
        }
        if (mode == "pin") {
            SigilTextButton({ coordinates = !coordinates }) { Text(if (coordinates) "Hide coordinates" else "Enter coordinates") }
            AnimatedVisibility(coordinates, enter = expandVertically(motion.tween()) + fadeIn(motion.tween()), exit = shrinkVertically(motion.tween()) + fadeOut(motion.tween())) {
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    OutlinedTextField(latitude, { latitude = it.take(24) }, Modifier.fillMaxWidth(), shape = RoundedCornerShape(16.dp), singleLine = true, label = { Text("Latitude") }, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal))
                    OutlinedTextField(longitude, { longitude = it.take(24) }, Modifier.fillMaxWidth(), shape = RoundedCornerShape(16.dp), singleLine = true, label = { Text("Longitude") }, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal))
                    SigilOutlinedButton({
                        val lat = latitude.toDoubleOrNull(); val lon = longitude.toDoubleOrNull()
                        if (lat != null && lon != null && lat.isFinite() && lon.isFinite() && lat in -90.0..90.0 && lon in -180.0..180.0) { point = lat to lon; issue = null; focus.clearFocus(); keyboard?.hide() }
                        else issue = "Use latitude −90 to 90 and longitude −180 to 180."
                    }) { Text("Set pin") }
                }
            }
        }
        point?.let { (lat, lon) -> Text(String.format(java.util.Locale.ROOT, "%.5f, %.5f", lat, lon), style = MaterialTheme.typography.bodySmall) }
        if (mode != "pin") sample?.takeIf { it.hasAccuracy() }?.let { Text("Accuracy about ${it.accuracy.roundToInt().coerceAtLeast(1)} m", style = MaterialTheme.typography.bodySmall) }
        OutlinedTextField(label, { if (it.toByteArray().size <= 256) label = it }, Modifier.fillMaxWidth(), shape = RoundedCornerShape(16.dp), label = { Text("Caption · optional") }, maxLines = 3)
        issue?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
        SigilButton({
            val chosen = point ?: return@SigilButton
            if (mode == "live" && !NativeLocations.visibleNotifications(context)) {
                if (Build.VERSION.SDK_INT >= 33 && context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) notifications.launch(Manifest.permission.POST_NOTIFICATIONS)
                else issue = "Enable Sigil notifications in Android settings before sharing live location."
                return@SigilButton
            }
            if (mode != "pin" && sample?.let { SystemClock.elapsedRealtimeNanos() - it.elapsedRealtimeNanos in 0..60_000_000_000L } != true) { issue = "Refresh your location before sharing."; return@SigilButton }
            sending = true
            scope.launch {
                try {
                    val sent = send(mapOf("latitude_e6" to (chosen.first * 1_000_000).roundToInt(), "longitude_e6" to (chosen.second * 1_000_000).roundToInt(),
                        "accuracy_cm" to if (mode == "pin") null else sample?.takeIf { it.hasAccuracy() }?.let { (it.accuracy * 100).roundToInt().coerceAtLeast(0) },
                        "sampled_at" to if (mode == "pin") System.currentTimeMillis() / 1000 else sample!!.time / 1000,
                        "label" to label.ifBlank { if (mode == "pin") "Dropped pin" else "My location" }, "pin" to (mode == "pin"), "live" to duration.takeIf { mode == "live" }))
                    if (!sent) issue = "Could not share this place. You can try again."
                } catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { issue = "Could not share this place. You can try again." }
                finally { sending = false }
            }
        }, Modifier.fillMaxWidth(), enabled = point != null && !locating && !sending) { Glyph("send", 22); Spacer(Modifier.width(8.dp)); Text(if (sending) "Preparing…" else if (mode == "live") "Share live location" else "Send place") }
    }
}
