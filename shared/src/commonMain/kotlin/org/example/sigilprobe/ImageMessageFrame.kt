package org.sigil

import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.unit.dp

internal fun imageMessageSize(width: Int, height: Int, available: Float): Size {
    val ratio = if (width > 0 && height > 0) width.toFloat() / height else 1f
    val w = minOf(available.coerceAtLeast(1f), 300f, 360f * ratio)
    return Size(w, w / ratio)
}

@Composable
fun ImageMessageFrame(width: Int, height: Int, content: @Composable (Modifier) -> Unit) {
    BoxWithConstraints {
        val size = imageMessageSize(width, height, maxWidth.value)
        content(Modifier.size(size.width.dp, size.height.dp).clip(RoundedCornerShape(18.dp)))
    }
}
