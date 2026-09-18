package org.sigil

import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.unit.dp

internal const val MessageBubbleMaxWidth = 348f
/// Cards fill the bubble they sit in; only their minimum keeps a tiny one from collapsing.
internal val MessageCardMinWidth = 200.dp
internal val MessageCardMaxWidth = 340.dp

/// A picture whose own shape is not known yet holds the commonest one rather than a full-width square.
internal const val PendingPictureRatio = 4f / 3f

internal fun imageMessageSize(width: Int, height: Int, available: Float): Size {
    val ratio = if (width > 0 && height > 0) width.toFloat() / height else PendingPictureRatio
    val w = minOf(available.coerceAtLeast(1f), MessageBubbleMaxWidth, 360f * ratio)
    return Size(w, w / ratio)
}

@Composable
fun ImageMessageFrame(width: Int, height: Int, shape: Shape = RoundedCornerShape(20.dp), caption: (@Composable () -> Unit)? = null, content: @Composable (Modifier) -> Unit) {
    BoxWithConstraints {
        val size = imageMessageSize(width, height, maxWidth.value)
        val picture = Modifier.size(size.width.dp, size.height.dp).clip(shape)
        // A caption sits on the bubble ground beneath the picture, kept to the picture's own width.
        if (caption == null) content(picture)
        else Column(Modifier.width(size.width.dp)) { content(picture); caption() }
    }
}
