package org.sigil.compose

import android.Manifest
import android.content.pm.PackageManager
import android.os.SystemClock
import android.util.Size
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.*
import androidx.camera.core.resolutionselector.*
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size as DrawSize
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.*
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import org.json.JSONObject
import org.sigil.storage.NativeStorage
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

@Composable
internal fun DeviceLinkDialog(flow: JSONObject, busy: Boolean, issue: String?, command: (String, String?) -> Unit) {
    val stage = flow.getString("stage")
    var scanning by remember(stage) { mutableStateOf(stage == "scan_offer") }
    var matched by remember(stage) { mutableStateOf(false) }
    val canCancel = flow.optBoolean("can_cancel", true)
    val close = { if (!busy) command(if (stage == "done") "close" else if (canCancel) "cancel" else "pause", null) }
    Dialog(close, DialogProperties(usePlatformDefaultWidth = false, securePolicy = SecureFlagPolicy.SecureOn)) {
        Surface(Modifier.fillMaxSize()) {
            Column(Modifier.fillMaxSize().systemBarsPadding().verticalScroll(rememberScrollState()).padding(24.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(20.dp)) {
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    Text("Link a device", Modifier.weight(1f), style = MaterialTheme.typography.headlineMedium)
                    TextButton(close, enabled = !busy) { Text(if (stage == "done") "Done" else if (canCancel) "Cancel" else "Finish later") }
                }
                Text(when (stage) {
                    "show_offer" -> "On your existing device, open Settings → Devices → Link a new device. Scan this code with that device."
                    "scan_offer" -> "Scan the code shown by your new device. Keep both devices with you throughout setup."
                    "show_proposal" -> "Now scan this code with your new device. Compare the symbols shown on both screens."
                    "confirm_join", "confirm_sponsor" -> "Check that these symbols match on both devices. Only approve a device you have with you."
                    "show_response" -> "Scan this final code with your existing device and approve the link there. Then finish here."
                    "authorize" -> "Your approval is saved. Retry to finish registering the device."
                    "cancelling" -> "Cancellation is pending. Retry to make sure the server cancels this link."
                    "done" -> "Your device is linked."
                    else -> "Preparing a secure link…"
                })
                if (scanning && !busy) LinkScanner { qr -> scanning = false; command("scan", qr) }
                else if (flow.has("cells")) {
                    val width = flow.getInt("width"); val cells = flow.getString("cells")
                    Canvas(Modifier.fillMaxWidth().aspectRatio(1f).semantics { contentDescription = "Device linking QR code" }) {
                        drawRect(Color.White)
                        val unit = kotlin.math.floor(size.minDimension / (width + 8))
                        val origin = Offset((size.width - unit * width) / 2, (size.height - unit * width) / 2)
                        cells.forEachIndexed { index, cell -> if (cell == '1') drawRect(Color.Black, origin + Offset(index % width * unit, index / width * unit), DrawSize(unit, unit)) }
                    }
                }
                flow.optJSONArray("emoji")?.let { emoji ->
                    Text((0 until emoji.length()).joinToString(" ") { emoji.getString(it) }, style = MaterialTheme.typography.headlineMedium)
                }
                if (flow.has("account")) Text(flow.getString("account"), style = MaterialTheme.typography.titleMedium)
                issue?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                if (busy) CircularProgressIndicator(Modifier.size(24.dp))
                when (stage) {
                    "scan_offer", "show_offer", "show_proposal" -> if (!scanning) Button({ scanning = true }, enabled = !busy) { Text("Scan the other device") }
                    "confirm_join", "confirm_sponsor" -> {
                        Row(verticalAlignment = Alignment.CenterVertically) { Checkbox(matched, { matched = it }, enabled = !busy); Text("The symbols match on both devices.", Modifier.weight(1f)) }
                        Button({ command("confirm", null) }, enabled = matched && !busy) { Text("Approve this device") }
                    }
                    "show_response" -> Button({ command("finish", null) }, enabled = !busy) { Text("Finish linking") }
                    "prepare_offer", "authorize", "cancelling" -> Button({ command("retry", null) }, enabled = !busy) { Text("Retry") }
                    "done" -> Button(close, enabled = !busy) { Text("Continue") }
                }
                if (scanning && stage != "scan_offer") TextButton({ scanning = false }) { Text("Show my code") }
            }
        }
    }
}

@Composable
private fun LinkScanner(found: (String) -> Unit) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current
    val onFound by rememberUpdatedState(found)
    var granted by remember { mutableStateOf(context.checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted = it }
    var issue by remember { mutableStateOf<String?>(null) }
    val preview = remember { PreviewView(context).apply { implementationMode = PreviewView.ImplementationMode.COMPATIBLE } }
    DisposableEffect(granted, lifecycle) {
        val active = AtomicBoolean(true)
        val delivered = AtomicBoolean(false)
        val executor = Executors.newSingleThreadExecutor()
        val main = ContextCompat.getMainExecutor(context)
        var provider: ProcessCameraProvider? = null
        var feed: Preview? = null
        var analysis: ImageAnalysis? = null
        if (granted) {
            val future = ProcessCameraProvider.getInstance(context)
            future.addListener({ if (active.get()) try {
                provider = future.get()
                feed = Preview.Builder().build().also { it.surfaceProvider = preview.surfaceProvider }
                analysis = ImageAnalysis.Builder().setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                    .setResolutionSelector(ResolutionSelector.Builder().setResolutionStrategy(ResolutionStrategy(Size(1280, 960), ResolutionStrategy.FALLBACK_RULE_CLOSEST_LOWER)).build()).build()
                var last = 0L
                analysis!!.setAnalyzer(executor) { image ->
                    try {
                        if (active.get() && !delivered.get() && SystemClock.elapsedRealtime() - last >= 300 && image.width in 64..1280 && image.height in 64..1280) {
                            last = SystemClock.elapsedRealtime()
                            val plane = image.planes[0]; val buffer = plane.buffer; val start = buffer.position()
                            val pixels = ByteArray(image.width * image.height)
                            try {
                                for (y in 0 until image.height) for (x in 0 until image.width) pixels[y * image.width + x] = buffer.get(start + y * plane.rowStride + x * plane.pixelStride)
                                val qr = NativeStorage.scanLinkQr(image.width, image.height, pixels)
                                if (qr != null && delivered.compareAndSet(false, true)) main.execute { if (active.get()) onFound(qr) }
                            } finally { pixels.fill(0) }
                        }
                    } catch (_: Exception) { main.execute { if (active.get()) issue = "Could not read this frame. Hold the code steady." } }
                    finally { image.close() }
                }
                provider!!.bindToLifecycle(lifecycle, CameraSelector.DEFAULT_BACK_CAMERA, feed!!, analysis!!)
            } catch (_: Exception) { issue = "Could not open the camera. Close any active video call and try again." } }, main)
        }
        onDispose { active.set(false); analysis?.clearAnalyzer(); feed?.let { provider?.unbind(it) }; analysis?.let { provider?.unbind(it) }; executor.shutdown() }
    }
    if (granted) AndroidView({ preview }, Modifier.fillMaxWidth().aspectRatio(1f))
    else Button({ permission.launch(Manifest.permission.CAMERA) }) { Text("Allow camera to scan") }
    issue?.let { Text(it) }
}
