package org.sigil.compose

import android.Manifest
import android.content.pm.PackageManager
import android.location.*
import android.os.*
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.*
import androidx.compose.ui.unit.dp
import androidx.lifecycle.*
import androidx.lifecycle.compose.LocalLifecycleOwner
import kotlinx.coroutines.*
import kotlin.math.roundToInt
import org.sigil.*

@Composable
internal fun PlacePanel(close: () -> Unit, photo: String = "", initialMode: String = "once", caption: String = "", mapContent: (@Composable ((Double, Double) -> Unit) -> Unit)? = null, send: suspend (Map<String, Any?>) -> Boolean) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val scope = rememberCoroutineScope()
    var point by remember { mutableStateOf<Pair<Double, Double>?>(null) }
    var mapCenter by remember { mutableStateOf<Pair<Double, Double>?>(null) }
    var sample by remember { mutableStateOf<Location?>(null) }
    var mode by rememberSaveable { mutableStateOf(initialMode.takeIf {it in listOf("once","live","pin")} ?: "once") }
    var duration by rememberSaveable { mutableStateOf("fifteen_minutes") }
    var locating by remember { mutableStateOf(false) }
    var sending by remember { mutableStateOf(false) }
    var issue by remember { mutableStateOf<String?>(null) }
    var mapFailed by remember { mutableStateOf(false) }
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
        issue = if (allowed) null else "Allow notifications to keep sharing in the background."
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
                    sample = value; if(mode=="pin")mapCenter=value.latitude to value.longitude else point=value.latitude to value.longitude; locating = false; issue = null
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
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
      Column(Modifier.fillMaxWidth().then(naturalPanelHeight()).padding(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Box(Modifier.fillMaxWidth().height(200.dp).clip(RoundedCornerShape(20.dp)).background(MaterialTheme.colorScheme.surfaceContainer)) {
                if (mapContent != null) mapContent { lat, lon -> if (mode == "pin") {point = lat to lon;mapCenter=null} }
                else if (!mapFailed) ServerMap(Modifier.fillMaxSize(), point?.first ?: 0.0, point?.second ?: 0.0, selected = point != null, center = mapCenter,
                    marker = if (mode != "pin") { { LocationAvatar("You", photo, false) } } else null,
                    chosen = { lat, lon -> if (mode == "pin") {point = lat to lon;mapCenter=null} }, failure = { mapFailed = true })
                else Column(Modifier.align(Alignment.Center), horizontalAlignment = Alignment.CenterHorizontally) {
                    Text("Map unavailable", style = MaterialTheme.typography.bodySmall)
                    SigilTextButton({ mapFailed = false }) { Text("Retry") }
                }
                Surface(Modifier.align(Alignment.TopStart).padding(4.dp), shape = RoundedCornerShape(24.dp), color = MaterialTheme.colorScheme.surface.copy(alpha = .94f), contentColor = MaterialTheme.colorScheme.onSurface) {
                    SigilIconButton(close) { Glyph("chevron_left", 24, "Back to attachments") }
                }
                LocationMapChip(when(mode) { "pin" -> "Drop a pin"; "live" -> "Real-time location"; else -> "One-time location" },
                    Modifier.align(Alignment.TopStart).padding(start = 60.dp, end = 60.dp, top = 12.dp))
                Surface(Modifier.align(Alignment.TopEnd).padding(8.dp), shape = RoundedCornerShape(24.dp), color = MaterialTheme.colorScheme.surface.copy(alpha = .94f), contentColor = MaterialTheme.colorScheme.onSurface) {
                    SigilIconButton(::locate, enabled = !locating && !sending) {
                        if (locating) CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                        else Glyph("my_location", 24, if (mode=="pin" || point == null) "Use my location" else "Refresh location")
                    }
                }
            }
            if (mode == "live") {
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    listOf("fifteen_minutes" to "15 min", "hour" to "1 hour", "eight_hours" to "8 hours").forEach { (id, name) ->
                        FilterChip(duration == id, { duration = id }, label = { Text(name) }, shape = RoundedCornerShape(14.dp))
                    }
                }
                if (issue != null && !NativeLocations.visibleNotifications(context)) SigilTextButton({
                    context.startActivity(android.content.Intent(android.provider.Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(android.provider.Settings.EXTRA_APP_PACKAGE, context.packageName))
                }) { Text("Notification settings") }
            }
        issue?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
        BuilderConfirm(if(sending)"Preparing…" else if(mode=="live")"Share live location" else "Send place",enabled=point!=null && !locating && !sending) {
            if (caption.toByteArray().size > 256) { issue = "Shorten the location caption."; return@BuilderConfirm }
            val chosen = point ?: return@BuilderConfirm
            if (mode == "live" && !NativeLocations.visibleNotifications(context)) {
                if (Build.VERSION.SDK_INT >= 33 && context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) notifications.launch(Manifest.permission.POST_NOTIFICATIONS)
                else issue = "Enable Sigil notifications in Android settings before sharing live location."
                return@BuilderConfirm
            }
            if (mode != "pin" && sample?.let { SystemClock.elapsedRealtimeNanos() - it.elapsedRealtimeNanos in 0..60_000_000_000L } != true) { issue = "Refresh your location before sharing."; return@BuilderConfirm }
            sending = true
            scope.launch {
                try {
                    val sent = send(mapOf("latitude_e6" to (chosen.first * 1_000_000).roundToInt(), "longitude_e6" to (chosen.second * 1_000_000).roundToInt(),
                        "accuracy_cm" to if (mode == "pin") null else sample?.takeIf { it.hasAccuracy() }?.let { (it.accuracy * 100).roundToInt().coerceAtLeast(0) },
                        "sampled_at" to if (mode == "pin") System.currentTimeMillis() / 1000 else sample!!.time / 1000,
                        "label" to caption.ifBlank { if (mode == "pin") "Dropped pin" else "My location" }, "pin" to (mode == "pin"), "live" to duration.takeIf { mode == "live" }))
                    if (!sent) issue = "Could not share this place. You can try again."
                } catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { issue = "Could not share this place. You can try again." }
                finally { sending = false }
            }
        }
    }
  }
}
