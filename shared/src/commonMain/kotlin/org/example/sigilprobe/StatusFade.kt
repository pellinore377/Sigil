package org.sigil

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

@Composable
internal fun StatusFade(statusInset: Dp, modifier: Modifier = Modifier) {
    val height = statusInset + 31.dp
    val background = MaterialTheme.colorScheme.background
    val fade = if (statusInset > 0.dp) Brush.verticalGradient(
        0f to background.copy(alpha = .92f),
        .25f to background.copy(alpha = .84f),
        .5f to background.copy(alpha = .60f),
        .75f to background.copy(alpha = .24f),
        1f to background.copy(alpha = 0f)
    ) else Brush.verticalGradient(
        0f to background,
        (12.dp / height) to background.copy(alpha = .96f),
        1f to background.copy(alpha = 0f)
    )
    Box(modifier.fillMaxWidth().height(height).background(fade))
}
