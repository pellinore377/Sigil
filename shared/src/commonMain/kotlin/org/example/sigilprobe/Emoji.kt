package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp

internal data class EmojiToken(val key: String, val text: String)
private val animatedSequences by lazy {
    animationKeys.split(' ').associateBy { key -> buildString {
        key.split('_').forEach { hex ->
            val cp = hex.toInt(16)
            if (cp <= 0xffff) append(cp.toChar()) else { append((0xd800 + ((cp - 0x10000) shr 10)).toChar()); append((0xdc00 + ((cp - 0x10000) and 1023)).toChar()) }
        }
    }.replace("\ufe0f", "") }.entries.sortedByDescending { it.key.length }
}
internal fun animatedEmoji(text: String): List<EmojiToken>? {
    val source = text.replace("\ufe0f", "")
    val result = mutableListOf<EmojiToken>()
    var at = 0
    while (at < source.length) {
        if (source[at].isWhitespace()) { at++; continue }
        val match = animatedSequences.firstOrNull { source.startsWith(it.key, at) } ?: return null
        result += EmojiToken(match.value, match.key)
        at += match.key.length
    }
    return result.takeIf { it.isNotEmpty() }
}
@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun EmojiMessage(tokens: List<EmojiToken>) {
    if (tokens.size > 8) { Text(tokens.joinToString("") { it.text }, style = MaterialTheme.typography.headlineSmall); return }
    val scale = LocalAppearance.current.textScale
    FlowRow(horizontalArrangement = Arrangement.spacedBy(4.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        tokens.forEach { EmojiArtwork(it, Modifier.size((if (tokens.size == 1) 88.dp else 64.dp) * scale)) }
    }
}
@Composable
internal expect fun EmojiArtwork(emoji: EmojiToken, modifier: Modifier)
@Composable
internal fun StaticEmoji(emoji: EmojiToken, modifier: Modifier) = BoxWithConstraints(modifier, contentAlignment = Alignment.Center) {
    val glyph = with(LocalDensity.current) { (minOf(maxWidth, maxHeight) * .55f).toSp() }
    Text(emoji.text, fontSize = glyph, lineHeight = glyph, maxLines = 1)
}
