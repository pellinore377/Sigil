package org.sigil

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.dropShadow
import androidx.compose.ui.graphics.shadow.Shadow
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.DpOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.layout.onSizeChanged

val LocalMediaCommand = staticCompositionLocalOf<Command?> { null }
val LocalMediaSender = staticCompositionLocalOf<(ChatMessage) -> String> { { if (it.mine) "You" else it.author } }
val LocalMediaMessage = staticCompositionLocalOf<(String, String, String) -> ChatMessage?> { { _, _, _ -> null } }

private fun chromeShadow(shape: RoundedCornerShape) = Modifier.dropShadow(shape, Shadow(radius = 8.dp, color = Color.Black.copy(alpha = .15f), offset = DpOffset(0.dp, 3.dp)))

@Composable
fun MediaViewerChrome(message: ChatMessage?, close: () -> Unit, save: (() -> Unit)? = null, menu: (() -> Unit)? = null, saveEnabled: Boolean = true, backdrop: ChromeBackdrop? = null, content: @Composable BoxScope.() -> Unit) {
    val command = LocalMediaCommand.current
    val sender = LocalMediaSender.current
    val colors = MaterialTheme.colorScheme
    val motionPolicy = LocalMotion.current
    val density=LocalDensity.current
    var headerHeight by remember {mutableStateOf(64.dp)}
    var reactionsHeight by remember {mutableStateOf(56.dp)}
    var chrome by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) { chrome = true }
    val headerShape = RoundedCornerShape(24.dp)
    val reactionShape = RoundedCornerShape(28.dp)
    // A caller that owns the backdrop feeds it from a page that draws only pixels; views in the capture would leave the blur holding dead render nodes.
    val own = rememberChromeBackdrop()
    val glass = backdrop ?: own
    Box(Modifier.fillMaxSize().then(if (backdrop == null) Modifier.captureBackdrop(own) else Modifier).background(colors.scrim.copy(alpha = .92f)).safeDrawingPadding().padding(12.dp)) {
        // The scrim is black in both themes, so no scheme role stays legible over it.
        Box(Modifier.fillMaxSize().padding(top = headerHeight + 12.dp, bottom = if (message != null && command != null && message.attachment?.draft != true) reactionsHeight + 12.dp else 12.dp), contentAlignment = Alignment.Center) { CompositionLocalProvider(LocalContentColor provides Color.White) { content() } }
        AnimatedVisibility(chrome, Modifier.align(Alignment.TopCenter), enter = fadeIn(motionPolicy.enter(MotionQuick, delayMillis = MotionStagger)) + slideInVertically(motionPolicy.enter(MotionQuick, delayMillis = MotionStagger)) { -it }, exit = fadeOut(motionPolicy.exit(MotionExit)), label = "Media chrome") {
            FloatingChrome(glass, Modifier.widthIn(max = 680.dp).fillMaxWidth().onSizeChanged {headerHeight=with(density){it.height.toDp()}}, headerShape) {
                Row(Modifier.padding(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    SigilIconButton(close) { Glyph("close", 24, "Close media") }
                    Column(Modifier.weight(1f).padding(start = 10.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(message?.let(sender).orEmpty(), maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.titleMedium)
                        if (!message?.time.isNullOrBlank()) Text(message!!.time, style = MaterialTheme.typography.labelSmall, color = colors.onSurfaceVariant)
                    }
                    save?.let { SigilIconButton(it, enabled = saveEnabled) { Glyph("download", 24, if (saveEnabled) "Save attachment" else "Saving attachment") } }
                    menu?.let { SigilIconButton(it) { Glyph("more_horiz", 24, "Media options") } }
                }
            }
        }
        if (message != null && command != null && message.attachment?.draft != true) {
            AnimatedVisibility(chrome, Modifier.align(Alignment.BottomCenter), enter = fadeIn(motionPolicy.enter(MotionQuick, delayMillis = MotionStagger)) + slideInVertically(motionPolicy.enter(MotionQuick, delayMillis = MotionStagger)) { it }, exit = fadeOut(motionPolicy.exit(MotionExit)), label = "Media reactions") {
                FloatingChrome(glass, Modifier.widthIn(max = 360.dp).fillMaxWidth().onSizeChanged {reactionsHeight=with(density){it.height.toDp()}}, reactionShape) {
                    Row(Modifier.padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                        listOf("❤️", "👍", "😂", "😮", "😢", "😡").forEach { emoji ->
                            SigilTextButton({ command("react", mediaReaction(message, emoji)) },
                                Modifier.weight(1f).background(if (emoji in message.myReactions) colors.primaryContainer else Color.Transparent, SigilButtonShape),
                                contentPadding = PaddingValues(0.dp)) { Text(emoji, fontSize = 22.sp) }
                        }
                    }
                }
            }
        }
    }
}

@Composable
fun GifChip(modifier: Modifier = Modifier) {
    Surface(modifier.padding(8.dp), shape = RoundedCornerShape(8.dp), color = MaterialTheme.colorScheme.scrim.copy(alpha = .65f), contentColor = Color.White) {
        Text("GIF", Modifier.padding(horizontal = 8.dp, vertical = 4.dp), style = MaterialTheme.typography.labelSmall)
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
