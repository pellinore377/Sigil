package org.sigil

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.layout.onSizeChanged

val LocalMediaCommand = staticCompositionLocalOf<Command?> { null }
val LocalMediaSender = staticCompositionLocalOf<(ChatMessage) -> String> { { if (it.mine) "You" else it.author } }
val LocalMediaMessage = staticCompositionLocalOf<(String, String, String) -> ChatMessage?> { { _, _, _ -> null } }

@Composable
fun MediaViewerChrome(message: ChatMessage?, close: () -> Unit, save: (() -> Unit)? = null, menu: (() -> Unit)? = null, saveEnabled: Boolean = true, content: @Composable BoxScope.() -> Unit) {
    val command = LocalMediaCommand.current
    val sender = LocalMediaSender.current
    val colors = MaterialTheme.colorScheme
    val density=LocalDensity.current
    var headerHeight by remember {mutableStateOf(64.dp)}
    var reactionsHeight by remember {mutableStateOf(56.dp)}
    Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = .78f)).systemBarsPadding().padding(12.dp)) {
        Box(Modifier.fillMaxSize().padding(top = headerHeight + 12.dp, bottom = if (message != null && command != null && message.attachment?.draft != true) reactionsHeight + 12.dp else 12.dp), contentAlignment = Alignment.Center) { CompositionLocalProvider(LocalContentColor provides Color.White) { content() } }
        Surface(Modifier.align(Alignment.TopCenter).widthIn(max = 720.dp).fillMaxWidth().onSizeChanged {headerHeight=with(density){it.height.toDp()}}, shape = RoundedCornerShape(26.dp), color = colors.surfaceContainerHigh.copy(alpha = .94f), contentColor = colors.onSurface) {
            Row(Modifier.padding(6.dp), verticalAlignment = Alignment.CenterVertically) {
                SigilIconButton(close) { Glyph("close", 24, "Close media") }
                Column(Modifier.weight(1f).padding(horizontal = 6.dp)) {
                    Text(message?.let(sender).orEmpty(), maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.titleMedium)
                    if (!message?.time.isNullOrBlank()) Text(message!!.time, style = MaterialTheme.typography.labelSmall, color = colors.onSurfaceVariant)
                }
                save?.let { SigilIconButton(it, enabled = saveEnabled) { Glyph("download", 24, if (saveEnabled) "Save attachment" else "Saving attachment") } }
                menu?.let { SigilIconButton(it) { Glyph("more_horiz", 24, "Media options") } }
            }
        }
        if (message != null && command != null && message.attachment?.draft != true) {
            Surface(Modifier.align(Alignment.BottomCenter).widthIn(max = 380.dp).fillMaxWidth().onSizeChanged {reactionsHeight=with(density){it.height.toDp()}}, shape = RoundedCornerShape(28.dp), color = colors.surfaceContainerHigh.copy(alpha = .94f), contentColor = colors.onSurface) {
                Row(Modifier.padding(4.dp), horizontalArrangement = Arrangement.SpaceEvenly) {
                    listOf("❤️", "👍", "😂", "😮", "😢", "😡").forEach { emoji ->
                        TextButton({ command("react", mediaReaction(message, emoji)) }, Modifier.weight(1f), contentPadding = PaddingValues(0.dp), colors = ButtonDefaults.textButtonColors(containerColor = if (emoji in message.myReactions) colors.secondaryContainer else Color.Transparent)) { Text(emoji, fontSize = 22.sp) }
                    }
                }
            }
        }
    }
}

@Composable
fun GifChip(modifier: Modifier = Modifier) {
    Surface(modifier.padding(8.dp), shape = RoundedCornerShape(8.dp), color = Color.Black.copy(alpha = .65f), contentColor = Color.White) {
        Text("GIF", Modifier.padding(horizontal = 7.dp, vertical = 3.dp), style = MaterialTheme.typography.labelSmall)
    }
}

internal fun mediaReaction(message: ChatMessage, emoji: String): Map<String, Any?> = mapOf("peer" to message.peer, "author" to message.author, "message" to message.id, "emoji" to emoji, "active" to (emoji !in message.myReactions))

@Composable
fun MediaViewerFrame(width: Int, height: Int, modifier: Modifier = Modifier, content: @Composable (Modifier) -> Unit) {
    BoxWithConstraints(modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        val ratio = if (width > 0 && height > 0) width.toFloat() / height else 1f
        val fittedWidth = minOf(maxWidth.value, maxHeight.value * ratio).coerceAtLeast(1f)
        content(Modifier.size(fittedWidth.dp, (fittedWidth / ratio).dp).clip(RoundedCornerShape(20.dp)))
    }
}
