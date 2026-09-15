package org.sigil

import androidx.compose.animation.core.*
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.unit.dp

@Composable
fun LocationAvatar(name: String, photo: String, fresh: Boolean) {
    val motion = LocalMotion.current
    val visible = LocalMotionVisible.current
    val pulse = if (fresh && visible && !motion.reduced) {
        val transition = rememberInfiniteTransition(label = "Location signal")
        transition.animateFloat(0f, 1f, infiniteRepeatable(tween(1800, easing = LinearEasing)), label = "Radio ring").value
    } else 0f
    val accent = MaterialTheme.colorScheme.primary
    Box(Modifier.size(64.dp), contentAlignment = Alignment.Center) {
        if (fresh) Canvas(Modifier.matchParentSize()) {
            for (shift in listOf(0f, .5f)) {
                val progress = (pulse + shift) % 1f
                drawCircle(accent.copy(alpha = (1f - progress) * .6f), radius = (20 + progress * 12).dp.toPx(), style = Stroke(1.5.dp.toPx()))
            }
        }
        Surface(Modifier.size(40.dp), shape = CircleShape, color = MaterialTheme.colorScheme.surfaceVariant, border = BorderStroke(2.dp, MaterialTheme.colorScheme.surface)) {
            Box(contentAlignment = Alignment.Center) { Text(name.take(1).uppercase()); LocalProfilePhoto.current(photo, Modifier.matchParentSize()) }
        }
    }
}
