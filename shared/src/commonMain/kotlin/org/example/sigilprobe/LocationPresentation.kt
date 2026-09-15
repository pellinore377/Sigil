package org.sigil

import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

fun locationRemaining(until: Long?, now: Long, stopped: Boolean): String {
    val seconds = if (stopped) 0 else ((until ?: now) - now).coerceAtLeast(0)
    if (seconds == 0L) return "Sharing ended"
    val hours = seconds / 3600
    val minutes = seconds % 3600 / 60
    return if (hours > 0) "${hours}h ${minutes}m left" else "${seconds / 60}:${(seconds % 60).toString().padStart(2, '0')} left"
}

@Composable
fun LocationMapChip(text: String, modifier: Modifier = Modifier) {
    Surface(modifier, shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.surface.copy(alpha = .94f), contentColor = MaterialTheme.colorScheme.onSurface) {
        Text(text, Modifier.padding(horizontal = 12.dp, vertical = 7.dp), style = MaterialTheme.typography.labelMedium)
    }
}
