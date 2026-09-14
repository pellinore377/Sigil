package org.sigil.compose

import org.sigil.SigilButton

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
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.platform.LocalContext
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
    val stage=flow.getString("stage")
    val close={if(!busy)command(if(stage=="done")"close" else if(flow.optBoolean("can_cancel",true))"cancel" else "pause",null)}
    Dialog(close,DialogProperties(usePlatformDefaultWidth=false,securePolicy=SecureFlagPolicy.SecureOn)) {
        org.sigil.LinkPanel(flow.toString(),busy,issue,command) {found->QrScanner(found)}
    }
}

@Composable
internal fun QrScanner(found: (String) -> Unit) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current
    val onFound by rememberUpdatedState(found)
    var granted by remember { mutableStateOf(context.checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted = it }
    var issue by remember { mutableStateOf<String?>(null) }
    val preview = remember { PreviewView(context).apply {
        implementationMode = PreviewView.ImplementationMode.COMPATIBLE
        layoutParams = android.view.ViewGroup.LayoutParams(android.view.ViewGroup.LayoutParams.MATCH_PARENT, android.view.ViewGroup.LayoutParams.MATCH_PARENT)
        clipChildren = true; clipToPadding = true
    } }
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
    if (granted) Box(Modifier.fillMaxWidth().aspectRatio(1f).clip(RoundedCornerShape(24.dp)).testTag("link-viewfinder")) {
        AndroidView({ preview }, Modifier.matchParentSize())
    }
    else SigilButton({ permission.launch(Manifest.permission.CAMERA) }) { Text("Allow camera to scan") }
    issue?.let { Text(it) }
}


@Composable
internal fun ContactQrDialog(flow: JSONObject, busy: Boolean, issue: String?, command: (String, String?) -> Unit) {
    Dialog({if(!busy)command("close",null)},DialogProperties(usePlatformDefaultWidth=false,securePolicy=SecureFlagPolicy.SecureOn)) {
        org.sigil.ContactPanel(flow.toString(),busy,issue,command) {found->QrScanner(found)}
    }
}
