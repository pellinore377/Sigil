package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.interaction.*
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.compositeOver
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp

@Composable
internal fun AdminTab(label: String, active: Boolean, enabled: Boolean, onClick: () -> Unit) {
    val interactions = remember { MutableInteractionSource() }
    val hovered by interactions.collectIsHoveredAsState()
    val focused by interactions.collectIsFocusedAsState()
    val pressed by interactions.collectIsPressedAsState()
    val colors = MaterialTheme.colorScheme
    val base = if (active) colors.secondaryContainer else colors.surface
    val background = if (enabled && (hovered || pressed)) colors.onSurface.copy(alpha = if (pressed) .10f else .05f).compositeOver(base) else base
    val shape = RoundedCornerShape(10.dp)
    Text(label, Modifier.clip(shape).background(background)
        .border(if (focused) 2.dp else 1.dp, if (focused) colors.primary else if (active) Color.Transparent else colors.outlineVariant, shape)
        .clickable(interactions, indication = null, enabled = enabled, role = Role.Tab, onClick = onClick)
        .semantics { selected = active }
        .padding(horizontal = 16.dp, vertical = 10.dp), style = MaterialTheme.typography.labelLarge,
        color = (if (active) colors.onSecondaryContainer else colors.onSurface).copy(alpha = if (enabled) 1f else .38f))
}
