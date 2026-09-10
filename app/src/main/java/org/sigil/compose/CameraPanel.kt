package org.sigil.compose

import org.sigil.SigilButton
import org.sigil.SigilOutlinedButton
import org.sigil.SigilIconButton

import android.Manifest
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.Matrix
import android.util.Size
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.*
import androidx.camera.core.resolutionselector.*
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.Image
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import kotlinx.coroutines.*
import java.io.ByteArrayOutputStream
import org.sigil.Glyph

@Composable
internal fun CameraPanel(close: () -> Unit, use: suspend (ByteArray) -> Unit) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current
    val scope = rememberCoroutineScope()
    var granted by remember { mutableStateOf(context.checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) }
    val permission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted = it }
    var front by remember { mutableStateOf(false) }
    var photo by remember { mutableStateOf<Bitmap?>(null) }
    var issue by remember { mutableStateOf<String?>(null) }
    var taking by remember { mutableStateOf(false) }
    var encoding by remember { mutableStateOf<Bitmap?>(null) }
    var ready by remember { mutableStateOf(false) }
    var closed by remember { mutableStateOf(false) }
    val preview = remember { PreviewView(context).apply {
        implementationMode = PreviewView.ImplementationMode.COMPATIBLE
        layoutParams = android.view.ViewGroup.LayoutParams(android.view.ViewGroup.LayoutParams.MATCH_PARENT, android.view.ViewGroup.LayoutParams.MATCH_PARENT)
        clipChildren = true; clipToPadding = true
    } }
    val capture = remember { ImageCapture.Builder().setResolutionSelector(ResolutionSelector.Builder().setResolutionStrategy(ResolutionStrategy(Size(1920, 1440), ResolutionStrategy.FALLBACK_RULE_CLOSEST_LOWER_THEN_HIGHER)).build()).build() }
    DisposableEffect(granted, front, photo) {
        var disposed = false
        var provider: ProcessCameraProvider? = null
        var feed: Preview? = null
        ready = false
        if (granted && photo == null) {
            val future = ProcessCameraProvider.getInstance(context)
            future.addListener({
                if (!disposed) try {
                    provider = future.get()
                    feed = Preview.Builder().build().also { it.surfaceProvider = preview.surfaceProvider }
                    provider!!.bindToLifecycle(lifecycle, if (front) CameraSelector.DEFAULT_FRONT_CAMERA else CameraSelector.DEFAULT_BACK_CAMERA, feed!!, capture)
                    ready = true
                } catch (_: Exception) { issue = "This camera could not be opened." }
            }, ContextCompat.getMainExecutor(context))
        }
        onDispose { disposed = true; feed?.let { provider?.unbind(it, capture) } }
    }
    DisposableEffect(Unit) { onDispose { closed = true; photo?.takeUnless { it === encoding }?.recycle(); photo = null } }
    Column(Modifier.fillMaxSize().padding(horizontal = 16.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            SigilIconButton(close) { Glyph("chevron_left", 24, "Back to attachments") }
            Text("Camera", Modifier.weight(1f).padding(start = 8.dp), style = MaterialTheme.typography.titleMedium)
            SigilIconButton({ front = !front }, enabled = photo == null && !taking) { Glyph("flip_camera_android", 24, "Switch camera") }
        }
        Box(Modifier.weight(1f).fillMaxWidth().clip(RoundedCornerShape(28.dp)), contentAlignment = Alignment.Center) {
            val bitmap = photo
            if (bitmap != null) Image(bitmap.asImageBitmap(), "Photo preview", Modifier.fillMaxSize())
            else if (granted) AndroidView({ preview }, Modifier.matchParentSize())
            else SigilButton({ permission.launch(Manifest.permission.CAMERA) }) { Text("Allow camera") }
        }
        issue?.let { Text(it, Modifier.padding(8.dp)) }
        Row(Modifier.padding(vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(16.dp)) {
            val bitmap = photo
            if (bitmap != null) {
                SigilOutlinedButton({ photo = null; bitmap.recycle() }, enabled = !taking, shape = RoundedCornerShape(16.dp)) { Text("Retake") }
                SigilButton({ taking = true; encoding = bitmap; scope.launch(start = CoroutineStart.UNDISPATCHED) {
                    var bytes: ByteArray? = null
                    try {
                        withContext(Dispatchers.Default) {
                            val output = object : ByteArrayOutputStream() { fun erase() { buf.fill(0); reset() } }
                            try { check(bitmap.compress(Bitmap.CompressFormat.JPEG, 90, output)); bytes = output.toByteArray() } finally { output.erase() }
                        }
                        use(bytes!!)
                    } catch (cancelled: CancellationException) { throw cancelled }
                    catch (_: Exception) { issue = "Could not prepare this photo. You can try again." }
                    finally { bytes?.fill(0); if (closed) bitmap.recycle(); encoding = null; taking = false }
                } }, enabled = !taking, shape = RoundedCornerShape(16.dp)) { Glyph("check", 22); Spacer(Modifier.width(8.dp)); Text("Use photo") }
            } else SigilButton({
                taking = true; issue = null
                capture.targetRotation = preview.display?.rotation ?: android.view.Surface.ROTATION_0
                capture.takePicture(ContextCompat.getMainExecutor(context), object : ImageCapture.OnImageCapturedCallback() {
                    override fun onCaptureSuccess(image: ImageProxy) {
                        if (closed) { image.close(); return }
                        scope.launch(start = CoroutineStart.UNDISPATCHED) {
                            var owned: Bitmap? = null
                            try {
                                withContext(Dispatchers.Default) {
                                    val raw = image.toBitmap()
                                    owned = raw
                                    if (image.imageInfo.rotationDegrees != 0) owned = Bitmap.createBitmap(raw, 0, 0, raw.width, raw.height, Matrix().apply { postRotate(image.imageInfo.rotationDegrees.toFloat()) }, true).also { if (it !== raw) raw.recycle() }
                                }
                                if (!closed) { photo = owned; owned = null }
                            } finally { owned?.recycle(); image.close(); taking = false }
                        }
                    }
                    override fun onError(error: ImageCaptureException) { taking = false; issue = "Could not take this photo. Try again." }
                })
            }, modifier = Modifier.size(64.dp), enabled = ready && !taking, shape = RoundedCornerShape(22.dp), contentPadding = PaddingValues(0.dp),
                border = BorderStroke(3.dp, MaterialTheme.colorScheme.outlineVariant), colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.inverseSurface, contentColor = MaterialTheme.colorScheme.inverseOnSurface)) { Glyph("photo_camera", 32, "Take photo") }
        }
    }
}
