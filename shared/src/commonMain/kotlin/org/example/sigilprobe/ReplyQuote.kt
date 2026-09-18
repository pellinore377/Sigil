package org.sigil

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

val QuotePreviewSize = 48.dp

// What a quoted attachment is called beneath the sender's name.
fun attachmentLabel(file: AttachmentDetails): String = when {
    file.name == "Voice message.aac" -> "Voice message"
    file.mediaType == "image/gif" -> "GIF"
    file.mediaType.startsWith("image/") -> "Photo"
    file.mediaType.startsWith("video/") -> "Video"
    file.mediaType.startsWith("audio/") -> "Audio"
    else -> attachmentKind(file.name, file.mediaType).name
}

private fun attachmentGlyph(file: AttachmentDetails): String = when {
    file.name == "Voice message.aac" -> "mic"
    file.mediaType.startsWith("image/") -> "image"
    file.mediaType.startsWith("video/") -> "videocam"
    else -> attachmentKind(file.name, file.mediaType).glyph
}

// A squircle glimpse of the quoted file: the picture or poster where one can be drawn, otherwise its glyph.
@Composable
fun QuotePreview(file: AttachmentDetails, source: ChatMessage?) {
    val ink = LocalContentColor.current
    Box(Modifier.size(QuotePreviewSize).clip(RoundedCornerShape(12.dp)).background(ink.copy(alpha = .1f)), contentAlignment = Alignment.Center) {
        val drawn = source != null && source.attachment != null && (file.mediaType.startsWith("image/") || file.mediaType.startsWith("video/")) && LocalAttachmentThumbnail.current(source, Modifier.matchParentSize())
        if (!drawn) Glyph(attachmentGlyph(file), 24, filled = file.name == "Voice message.aac")
    }
}

// The body shared by the bubble quote and the composer chip: name over text, or a preview beside name and type.
@Composable
fun QuoteBody(name: String?, text: String?, file: AttachmentDetails?, source: ChatMessage?, lines: Int, modifier: Modifier = Modifier) {
    val title = MaterialTheme.typography.labelMedium.copy(fontWeight = FontWeight.SemiBold)
    if (file != null) Row(modifier, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        QuotePreview(file, source)
        Column {
            if (name != null) Text(name, style = title, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(attachmentLabel(file), style = MaterialTheme.typography.bodySmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    } else Column(modifier) {
        if (name != null) Text(name, style = title, maxLines = 1, overflow = TextOverflow.Ellipsis)
        if (text != null) Text(text, style = MaterialTheme.typography.bodySmall, maxLines = lines, overflow = TextOverflow.Ellipsis)
    }
}

// The quoted message set into a bubble: the timeline ground, so it reads darker in dark mode and lighter in light; its bottom corners meet the text.
@Composable
internal fun ReplyQuote(message: ChatMessage, topStart: androidx.compose.ui.unit.Dp = 14.dp, topEnd: androidx.compose.ui.unit.Dp = 14.dp) {
    val quoted = message.reply ?: return
    val scheme = MaterialTheme.colorScheme
    val name = message.replyAuthor?.let { LocalMediaSender.current(message.copy(author = it, mine = message.replyMine)) }
    val source = if (message.replyAuthor != null && message.replyMessage != null) LocalMediaMessage.current(message.peer, message.replyAuthor, message.replyMessage) else null
    Surface(Modifier.fillMaxWidth().testTag("reply-quote"), shape = RoundedCornerShape(topStart, topEnd, 5.dp, 5.dp), color = scheme.background, contentColor = scheme.onBackground) {
        QuoteBody(name, quoted, message.replyAttachment, source, 4, Modifier.padding(horizontal = 12.dp, vertical = 10.dp))
    }
}

// What the next message answers or replaces, above the writing field: the same block the bubble will show.
@Composable
internal fun ContextChip(title: String, text: String?, file: AttachmentDetails? = null, source: ChatMessage? = null, close: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    Surface(Modifier.fillMaxWidth().padding(start = 8.dp, end = 8.dp, top = 8.dp).testTag("context-chip"), shape = RoundedCornerShape(14.dp, 14.dp, 5.dp, 5.dp), color = scheme.background, contentColor = scheme.onBackground) {
        Row(Modifier.padding(start = 12.dp, end = 4.dp, top = 2.dp, bottom = 2.dp), verticalAlignment = Alignment.CenterVertically) {
            QuoteBody(title, text, file, source, 1, Modifier.weight(1f).padding(vertical = 8.dp))
            Symbol("close", "Cancel reply or edit", close)
        }
    }
}
