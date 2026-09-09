package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.background
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.*
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInWindow
import androidx.compose.ui.input.pointer.*
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.*
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.launch

@Composable
internal fun Field(label: String, value: String, change: (String) -> Unit, secret: Boolean = false, enabled: Boolean = true, onSubmit: (() -> Unit)? = null) {
    var editing by remember { mutableStateOf(TextFieldValue(value)) }
    if (editing.text != value) editing = TextFieldValue(value, editing.selection)
    var focused by remember { mutableStateOf(false) }
    var menu by remember { mutableStateOf<Offset?>(null) }
    var menuSelection by remember { mutableStateOf<TextFieldValue?>(null) }
    if (menu == null) menuSelection = null
    var origin by remember { mutableStateOf(Offset.Zero) }
    var bottom by remember { mutableStateOf(Offset.Zero) }
    var selectedItem by remember { mutableStateOf<Int?>(null) }
    var clipboardError by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val focus = remember { FocusRequester() }
    val manager = LocalFocusManager.current
    val currentEnabled = rememberUpdatedState(enabled)
    val currentChange = rememberUpdatedState(change)
    val enter: () -> Unit = { if (enabled) { if (onSubmit != null) onSubmit() else manager.moveFocus(FocusDirection.Next) } }
    fun update(next: TextFieldValue) { if (next.text != editing.text) menu = null; editing = next; clipboardError = false; currentChange.value(next.text) }
    fun clipboard(action: String) {
        val selected = menuSelection ?: editing
        menu = null
        focus.requestFocus()
        scope.launch(start = CoroutineStart.UNDISPATCHED) {
            try {
                val replacement = if (action == "Paste") readClipboard() else {
                    writeClipboard(selected.text.substring(selected.selection.min, selected.selection.max))
                    ""
                }
                if (currentEnabled.value && editing.text == selected.text && action != "Copy") {
                    val plain = replacement.replace('\n', ' ').replace('\r', ' ')
                    update(TextFieldValue(selected.text.replaceRange(selected.selection.min, selected.selection.max, plain), TextRange(selected.selection.min + plain.length)))
                }
            } catch (e: CancellationException) { throw e }
            catch (_: Exception) { clipboardError = true }
        }
    }
    val items = buildList {
        if (!secret && !(menuSelection ?: editing).selection.collapsed) { add("Cut"); add("Copy") }
        add("Paste")
        add("Select all")
    }
    fun choose(item: String) {
        if (item == "Select all") {
            menu = null; focus.requestFocus()
            update(editing.copy(selection = TextRange(0, editing.text.length)))
        } else clipboard(item)
    }
    val menuKey: ((String) -> Boolean)? = if (menu == null) null else { key ->
        when (key) {
            "Escape" -> { menu = null; true }
            "ArrowDown" -> { selectedItem = ((selectedItem ?: -1) + 1) % items.size; true }
            "ArrowUp" -> { selectedItem = ((selectedItem ?: 0) + items.size - 1) % items.size; true }
            "Enter" -> { choose(items[(selectedItem ?: 0).coerceIn(items.indices)]); true }
            "Tab" -> { menu = null; false }
            else -> false
        }
    }
    if (focused || menu != null) WebFieldInput(label, secret, onSubmit != null, enter,
        { if (enabled) { menuSelection = editing; selectedItem = 0; menu = bottom } }, menuKey)
    Column {
        Box(Modifier.fillMaxWidth().onGloballyPositioned { origin = it.positionInWindow(); bottom = origin + Offset(0f, it.size.height.toFloat()) }.pointerInput(enabled) {
            awaitPointerEventScope {
                while (true) {
                    val event = awaitPointerEvent(PointerEventPass.Initial)
                    if (enabled && event.type == PointerEventType.Press && event.buttons.isSecondaryPressed) {
                        val point = event.changes.first().position
                        event.changes.forEach { it.consume() }
                        menuSelection = editing
                        focus.requestFocus()
                        selectedItem = null
                        menu = origin + point
                    }
                }
            }
        }) {
            OutlinedTextField(editing, { update(it) }, Modifier.fillMaxWidth().focusRequester(focus).onFocusChanged { focused = it.isFocused }.semantics { contentDescription = label },
                label = { Text(label) }, singleLine = true, enabled = enabled,
                keyboardOptions = KeyboardOptions(keyboardType = if (secret) KeyboardType.Password else KeyboardType.Text, autoCorrectEnabled = false, imeAction = if (onSubmit == null) ImeAction.Next else ImeAction.Done),
                keyboardActions = KeyboardActions(onNext = { enter() }, onDone = { enter() }),
                shape = RoundedCornerShape(10.dp), visualTransformation = if (secret) PasswordVisualTransformation() else VisualTransformation.None)
            if (menu != null && enabled) AdminMenu(checkNotNull(menu), { menu = null }) {
                items.forEachIndexed { index, item ->
                    DropdownMenuItem(text = { Text(item) }, onClick = { choose(item) }, enabled = item != "Select all" || editing.text.isNotEmpty(),
                        modifier = Modifier.focusProperties { canFocus = false }.background(if (selectedItem == index) MaterialTheme.colorScheme.surfaceVariant else Color.Transparent).semantics { selected = selectedItem == index })
                }
            }
        }
        if (clipboardError) Text("Clipboard access was not granted. Use your keyboard’s copy or paste shortcut.", style = MaterialTheme.typography.bodySmall)
    }
}
