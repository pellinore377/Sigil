package org.sigil

import androidx.compose.runtime.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

val LocalAttachmentContent = staticCompositionLocalOf<@Composable (ChatMessage) -> Unit> {
    { message -> Text(message.attachment?.name ?: "Attachment", style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis) }
}
val LocalAttachmentDraft = staticCompositionLocalOf<@Composable (Transfer, Modifier) -> Unit> {
    { file, modifier -> Text(file.name, modifier, style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis) }
}
val LocalCameraPanel = staticCompositionLocalOf<@Composable (Map<String, Any?>, () -> Unit, () -> Unit) -> Unit> {
    { _, back, _ -> PanelUnavailable("Camera unavailable on this device.", back) }
}
val LocalPlacePanel = staticCompositionLocalOf<@Composable (Map<String, Any?>, () -> Unit, () -> Unit) -> Unit> {
    { _, back, _ -> PanelUnavailable("Location unavailable on this device.", back) }
}
val LocalLocationContent = staticCompositionLocalOf<@Composable (ChatMessage, MessagePart, Command?) -> Unit> {
    { message, part, command -> LocationSurface(message, part, command) }
}
val LocalWallpaper = staticCompositionLocalOf<@Composable (String, Modifier) -> Boolean> { { _, _ -> false } }
val LocalRecipeScale = staticCompositionLocalOf<(suspend (ChatMessage, MessagePart, Int) -> RecipeContent)? > { null }
val LocalKeepScreenAwake = staticCompositionLocalOf<(@Composable (Boolean) -> Unit)?> { null }

@Composable
private fun PanelUnavailable(message: String, back: () -> Unit) {
    Column(Modifier.fillMaxWidth().padding(32.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(message, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        SigilTextButton(back) { Text("Back") }
    }
}
