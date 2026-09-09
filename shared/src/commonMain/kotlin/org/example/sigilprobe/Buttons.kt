package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp

val SigilButtonShape = RoundedCornerShape(16.dp)

@Composable
fun SigilButton(onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true,
    shape: Shape = SigilButtonShape, colors: ButtonColors = ButtonDefaults.buttonColors(),
    border: BorderStroke? = null,
    contentPadding: PaddingValues = ButtonDefaults.ContentPadding, content: @Composable RowScope.() -> Unit) {
    Button(onClick, modifier, enabled, shape = shape, colors = colors, border = border, contentPadding = contentPadding, content = content)
}

@Composable
fun SigilOutlinedButton(onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true,
    shape: Shape = SigilButtonShape, content: @Composable RowScope.() -> Unit) {
    OutlinedButton(onClick, modifier, enabled, shape = shape, content = content)
}

@Composable
fun SigilTextButton(onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true,
    contentPadding: PaddingValues = ButtonDefaults.TextButtonContentPadding, content: @Composable RowScope.() -> Unit) {
    TextButton(onClick, modifier, enabled, shape = SigilButtonShape, contentPadding = contentPadding, content = content)
}

@Composable
fun SigilIconButton(onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true, content: @Composable () -> Unit) {
    val colors = IconButtonDefaults.iconButtonColors()
    Surface(onClick, modifier.semantics { role = Role.Button }, enabled, shape = SigilButtonShape,
        color = if (enabled) colors.containerColor else colors.disabledContainerColor,
        contentColor = if (enabled) colors.contentColor else colors.disabledContentColor) {
        Box(Modifier.size(48.dp), contentAlignment = Alignment.Center, content = { content() })
    }
}
