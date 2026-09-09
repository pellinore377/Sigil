package org.sigil.compose

import android.Manifest
import android.content.pm.PackageManager
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.Bundle
import android.os.Looper
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import kotlinx.coroutines.delay
import kotlin.math.roundToInt

@Composable
internal fun PlaceSheet(close: () -> Unit, send: (Map<String, Any?>) -> Unit) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    var point by remember { mutableStateOf<Pair<Double, Double>?>(null) }
    var sample by remember { mutableStateOf<Location?>(null) }
    var label by remember { mutableStateOf("") }
    var locating by remember { mutableStateOf(false) }
    var issue by remember { mutableStateOf<String?>(null) }
    DisposableEffect(lifecycle) {
        val observer = LifecycleEventObserver { _, event -> if (event == Lifecycle.Event.ON_STOP) locating = false }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer) }
    }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { result ->
        locating = result.values.any { it }
        if (!locating) issue = "Allow location access or select a place on the map."
    }
    LaunchedEffect(locating) { if (locating) { delay(20000); locating = false; issue = "No location fix yet. Try again or select a place on the map." } }
    DisposableEffect(locating) {
        val manager = context.getSystemService(LocationManager::class.java)
        val listener = object : LocationListener {
            override fun onLocationChanged(value: Location) { if (locating) { sample = value; point = value.latitude to value.longitude; locating = false; issue = null } }
            override fun onStatusChanged(provider: String?, status: Int, extras: Bundle?) {}
            override fun onProviderEnabled(provider: String) {}
            override fun onProviderDisabled(provider: String) {}
        }
        if (locating) try {
            val providers = listOf(LocationManager.GPS_PROVIDER, LocationManager.NETWORK_PROVIDER).filter { manager.isProviderEnabled(it) }
            if (providers.isEmpty()) { locating = false; issue = "Enable location services or select a place on the map." }
            providers.forEach { manager.requestLocationUpdates(it, 1000L, 0f, listener, Looper.getMainLooper()) }
        } catch (_: SecurityException) { locating = false; issue = "Location permission is needed." }
        onDispose { manager.removeUpdates(listener) }
    }
    Dialog(close, DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) {
            Column(Modifier.fillMaxSize().systemBarsPadding().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) { TextButton(close) { Text("Cancel") }; Text("Share a place", style = MaterialTheme.typography.titleLarge) }
                Box(Modifier.weight(1f).fillMaxWidth()) {
                    ServerMap(Modifier.fillMaxSize(), point?.first ?: 0.0, point?.second ?: 0.0, chosen = { lat, lon -> point = lat to lon; sample = null }, failure = { issue = "Maps are unavailable. You can still share your current location." })
                }
                issue?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
                OutlinedTextField(label, { label = it.take(256) }, Modifier.fillMaxWidth(), label = { Text("Place name · optional") })
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                    TextButton({
                        if (context.checkSelfPermission(Manifest.permission.ACCESS_COARSE_LOCATION) == PackageManager.PERMISSION_GRANTED) locating = true
                        else permission.launch(arrayOf(Manifest.permission.ACCESS_FINE_LOCATION, Manifest.permission.ACCESS_COARSE_LOCATION))
                    }, enabled = !locating) { Text(if (locating) "Locating…" else "Use my location") }
                    Button({ point?.let { (lat, lon) -> send(mapOf("latitude_e6" to (lat * 1_000_000).roundToInt(), "longitude_e6" to (lon * 1_000_000).roundToInt(), "accuracy_cm" to sample?.let { (it.accuracy * 100).roundToInt().coerceAtLeast(0) }, "sampled_at" to (sample?.time?.div(1000) ?: System.currentTimeMillis() / 1000), "label" to label.ifBlank { "Shared place" }, "pin" to (sample == null))) } }, enabled = point != null && !locating) { Text("Send place") }
                }
            }
        }
    }
}

@Composable
internal fun LocationCard(part: org.sigil.MessagePart) {
    var opened by remember(part.id) { mutableStateOf(false) }
    TextButton({ opened = true }) { org.sigil.Glyph("location_on", 22); Text("Open map") }
    if (opened) Dialog({ opened = false }, DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize()) { Column(Modifier.systemBarsPadding().padding(16.dp)) {
            Text(part.text, style = MaterialTheme.typography.titleLarge)
            ServerMap(Modifier.weight(1f).fillMaxWidth(), part.latitude, part.longitude)
            TextButton({ opened = false }) { Text("Close") }
        } }
    }
}
