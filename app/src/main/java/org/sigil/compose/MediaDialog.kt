package org.sigil.compose

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.window.DialogWindowProvider
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import kotlinx.coroutines.*
import org.sigil.*

@Composable
internal fun MediaDialog(message: ChatMessage, close: () -> Unit, backdrop: ChromeBackdrop? = null, content: @Composable BoxScope.() -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var issue by remember { mutableStateOf<String?>(null) }
    var menu by remember { mutableStateOf(false) }
    var saving by remember { mutableStateOf(false) }
    val destination = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument(message.attachment!!.mediaType)) { uri ->
        if (uri != null) scope.launch {
            saving = true
            try {
                withContext(Dispatchers.IO) {
                    check(prepare(context, message))
                    context.contentResolver.openOutputStream(uri)?.use { output ->
                        EncryptedMedia(context, message).use { media ->
                            val buffer = ByteArray(64 * 1024)
                            try { var at = 0L; while (at < media.size) { ensureActive(); val count = media.readAt(at, buffer, 0, buffer.size); check(count > 0); output.write(buffer, 0, count); at += count } }
                            finally { buffer.fill(0) }
                        }
                    } ?: error("Destination unavailable")
                }
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { issue = "Could not save this attachment." }
            finally { saving = false }
        }
    }
    Presented(close) {
        MediaViewerChrome(message, close, save = { destination.launch(message.attachment!!.name) }, menu = { menu = true }, saveEnabled = !saving, backdrop = backdrop, content = content)
        if (menu) AlertDialog({ menu = false }, text = { SigilTextButton({ menu = false; NativeFileProvider.open(context, message) }) { Text("Open externally") } }, confirmButton = { SigilTextButton({ menu = false }) { Text("Close") } })
        issue?.let { text -> AlertDialog({ issue = null }, text = { Text(text) }, confirmButton = { SigilTextButton({ issue = null }) { Text("OK") } }) }
    }
}
