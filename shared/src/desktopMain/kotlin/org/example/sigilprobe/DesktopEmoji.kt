package org.sigil
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
@Composable
internal actual fun EmojiArtwork(emoji: EmojiToken, modifier: Modifier) = StaticEmoji(emoji, modifier)
