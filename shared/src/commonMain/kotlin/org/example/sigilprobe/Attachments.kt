package org.sigil

import androidx.compose.runtime.*
import androidx.compose.material3.Text
import androidx.compose.ui.Modifier

val LocalAttachmentContent = staticCompositionLocalOf<@Composable (ChatMessage) -> Unit> {
    { message -> Text(message.attachment?.name ?: "Attachment") }
}
val LocalAttachmentDraft = staticCompositionLocalOf<@Composable (Transfer, Modifier) -> Unit> { { file, modifier -> Text(file.name, modifier) } }
val LocalCameraPanel = staticCompositionLocalOf<@Composable (Map<String, Any?>, () -> Unit, () -> Unit) -> Unit> {
    { _, back, _ -> SigilTextButton(back) { Text("Camera unavailable · Back") } }
}
val LocalLocationContent = staticCompositionLocalOf<@Composable (MessagePart) -> Unit> {
    { part -> Text("${part.latitude}, ${part.longitude}") }
}
val LocalWallpaper = staticCompositionLocalOf<@Composable (String, Modifier) -> Boolean> { { _, _ -> false } }
