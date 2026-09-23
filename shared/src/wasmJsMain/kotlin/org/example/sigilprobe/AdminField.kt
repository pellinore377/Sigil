package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.*
import androidx.compose.ui.input.pointer.*
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.input.*
import androidx.compose.ui.unit.dp

@Composable
internal fun Field(label: String, value: String, change: (String) -> Unit, secret: Boolean = false, enabled: Boolean = true, onSubmit: (() -> Unit)? = null) {
    var editing by remember { mutableStateOf(TextFieldValue(value)) }
    if (editing.text != value) editing = TextFieldValue(value, editing.selection)
    var focused by remember { mutableStateOf(false) }
    var hovered by remember { mutableStateOf(false) }
    val focus = remember { FocusRequester() }
    val manager = LocalFocusManager.current
    val enter: () -> Unit = { if (enabled) { if (onSubmit != null) onSubmit() else manager.moveFocus(FocusDirection.Next) } }
    fun update(next: TextFieldValue) { editing = next; change(next.text) }
    NativeFieldMenu({ hovered }, editing, secret, enabled, { update(it) }, { focus.requestFocus() })
    if (focused) WebFieldInput(label, secret, onSubmit != null, enter)
    Column {
        Box(Modifier.fillMaxWidth().pointerInput(Unit) {
            awaitPointerEventScope {
                while (true) {
                    val event = awaitPointerEvent(PointerEventPass.Initial)
                    if (event.type == PointerEventType.Enter || event.type == PointerEventType.Move) hovered = true
                    if (event.type == PointerEventType.Exit) hovered = false
                    // The browser's own menu handles the right button.
                    if (event.buttons.isSecondaryPressed) event.changes.forEach { it.consume() }
                }
            }
        }) {
            OutlinedTextField(editing, { update(it) }, Modifier.fillMaxWidth().focusRequester(focus).onFocusChanged { focused = it.isFocused }.semantics { contentDescription = label },
                label = { Text(label) }, singleLine = true, enabled = enabled,
                keyboardOptions = KeyboardOptions(keyboardType = if (secret) KeyboardType.Password else KeyboardType.Text, autoCorrectEnabled = false, imeAction = if (onSubmit == null) ImeAction.Next else ImeAction.Done),
                keyboardActions = KeyboardActions(onNext = { enter() }, onDone = { enter() }),
                colors = OutlinedTextFieldDefaults.colors(
                    unfocusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                    focusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                    unfocusedBorderColor = Color.Transparent,
                ),
                shape = RoundedCornerShape(14.dp), visualTransformation = if (secret) PasswordVisualTransformation() else VisualTransformation.None)
        }
    }
}
