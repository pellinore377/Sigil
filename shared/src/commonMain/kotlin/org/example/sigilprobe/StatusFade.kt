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
    Box(modifier.fillMaxWidth().height(height).background(Brush.verticalGradient(
        0f to background,
        (statusInset / height) to background,
        ((statusInset + 12.dp) / height) to background.copy(alpha = .96f),
        1f to background.copy(alpha = 0f)
    )))
}
