package org.sigil.compose

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.platform.LocalContext
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.sigil.ChatMessage
import org.sigil.SigilTextButton

// Saving an attachment through the system picker, shared by every viewer that offers a download.
internal class AttachmentSaver(val save: () -> Unit, val saving: Boolean, val issue: String?, val dismiss: () -> Unit)

@Composable internal fun rememberAttachmentSaver(message: ChatMessage): AttachmentSaver {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var issue by remember { mutableStateOf<String?>(null) }
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
    return AttachmentSaver({ destination.launch(message.attachment!!.name) }, saving, issue) { issue = null }
}

@Composable internal fun AttachmentSaver.Notice() {
    issue?.let { text -> AlertDialog(dismiss, text = { Text(text) }, confirmButton = { SigilTextButton(dismiss) { Text("OK") } }) }
}
