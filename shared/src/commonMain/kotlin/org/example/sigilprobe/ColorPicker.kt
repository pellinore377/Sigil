package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.*
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import kotlin.math.*

private val accents = listOf("Ink" to 0x555555, "Slate" to 0x8999a8, "Rose" to 0xb69795, "Sand" to 0xb29b7e, "Moss" to 0x919d90, "Dusk" to 0x808ca3, "Lavender" to 0xa296ad)
private fun hsv(color: Int): FloatArray {
    val r = (color shr 16 and 255) / 255f; val g = (color shr 8 and 255) / 255f; val b = (color and 255) / 255f
    val maximum = maxOf(r, g, b); val minimum = minOf(r, g, b); val delta = maximum - minimum
    val hue = if (delta == 0f) 0f else ((when (maximum) { r -> (g - b) / delta; g -> (b - r) / delta + 2; else -> (r - g) / delta + 4 }) * 60 + 360) % 360
    return floatArrayOf(hue, if (maximum == 0f) 0f else delta / maximum, maximum)
}
@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun AccentPicker(value: Int?, update: (Int) -> Unit) {
    var custom by remember { mutableStateOf(false) }
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            Text("Accent color", style = MaterialTheme.typography.titleLarge)
            Text(accents.find { it.second == value }?.first ?: if (value == null) "App default" else "Custom", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            accents.forEach { (name, color) ->
                Box(Modifier.size(44.dp).semantics { contentDescription = name; selected = value == color; role = Role.RadioButton }.clickable { update(color) }.padding(2.dp)
                    .then(if (value == color) Modifier.border(1.dp, MaterialTheme.colorScheme.onSurface, RoundedCornerShape(13.dp)) else Modifier).padding(3.dp).background(Color(0xff000000L or color.toLong()), RoundedCornerShape(10.dp)), contentAlignment = Alignment.Center) {
                    if (value == color) CompositionLocalProvider(LocalContentColor provides if (Color(0xff000000L or color.toLong()).luminance() > .18f) Color.Black else Color.White) { Glyph("check", 20) }
                }
            }
        }
        TextButton({ custom = true }) { Glyph("palette", 20); Spacer(Modifier.width(8.dp)); Text("Custom color") }
    }
    if (custom) CustomColor(value ?: 0x555555, { custom = false }) { update(it); custom = false }
}
@Composable
private fun CustomColor(initial: Int, close: () -> Unit, apply: (Int) -> Unit) {
    val seed = remember(initial) { hsv(initial) }
    var hue by remember { mutableFloatStateOf(seed[0]) }; var saturation by remember { mutableFloatStateOf(seed[1]) }; var brightness by remember { mutableFloatStateOf(seed[2]) }
    var advanced by remember { mutableStateOf(false) }
    val color = Color.hsv(hue, saturation, brightness)
    val rgb = color.toArgb() and 0xffffff
    fun select(value: Int) { val v = hsv(value); hue = v[0]; saturation = v[1]; brightness = v[2] }
    AlertDialog(close, title = { Text("Your accent") }, text = {
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Row { TextButton({ advanced = false }) { Text("Color") }; TextButton({ advanced = true }) { Text("Advanced") } }
            if (!advanced) {
                Canvas(Modifier.fillMaxWidth().height(160.dp).clip(RoundedCornerShape(16.dp))
                    .pointerInput(Unit) { detectTapGestures { point -> saturation = (point.x / size.width).coerceIn(0f, 1f); brightness = (1 - point.y / size.height).coerceIn(0f, 1f) } }
                    .pointerInput(Unit) { detectDragGestures { change, _ -> change.consume(); saturation = (change.position.x / size.width).coerceIn(0f, 1f); brightness = (1 - change.position.y / size.height).coerceIn(0f, 1f) } }) {
                    drawRect(Color.hsv(hue, 1f, 1f)); drawRect(Brush.horizontalGradient(listOf(Color.White, Color.Transparent))); drawRect(Brush.verticalGradient(listOf(Color.Transparent, Color.Black)))
                    val position = Offset(saturation * size.width, (1 - brightness) * size.height)
                    drawCircle(Color.White, 7.dp.toPx(), position, style = androidx.compose.ui.graphics.drawscope.Stroke(2.dp.toPx())); drawCircle(Color.Black.copy(alpha = .4f), 9.dp.toPx(), position, style = androidx.compose.ui.graphics.drawscope.Stroke(1.dp.toPx()))
                }
                Box(Modifier.fillMaxWidth().height(48.dp), contentAlignment = Alignment.Center) {
                    Box(Modifier.fillMaxWidth().height(10.dp).clip(CircleShape).background(Brush.horizontalGradient((0..6).map { Color.hsv(it * 60f, 1f, 1f) })))
                    Slider(hue, { hue = it }, valueRange = 0f..359.99f, modifier = Modifier.semantics { contentDescription = "Hue" }, colors = SliderDefaults.colors(activeTrackColor = Color.Transparent, inactiveTrackColor = Color.Transparent, thumbColor = color))
                }
                Text("Suggested", style = MaterialTheme.typography.labelMedium)
                Row(horizontalArrangement = Arrangement.SpaceEvenly, modifier = Modifier.fillMaxWidth()) { accents.forEach { (name, value) -> Box(Modifier.size(28.dp).background(Color(0xff000000L or value.toLong()), CircleShape).semantics { contentDescription = name }.clickable { select(value) }) } }
            } else {
                var text by remember(rgb) { mutableStateOf(accentText(rgb)) }
                OutlinedTextField(text, { text = it.take(7); parseAccent(it)?.let(::select) }, singleLine = true, label = { Text("Hex") }, prefix = { Text("#") }, isError = parseAccent(text) == null)
                listOf("Red" to 16, "Green" to 8, "Blue" to 0).forEach { (name, shift) ->
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(name, Modifier.width(50.dp), style = MaterialTheme.typography.labelSmall)
                        Slider((rgb shr shift and 255).toFloat(), { select((rgb and (255 shl shift).inv()) or (it.roundToInt() shl shift)) }, valueRange = 0f..255f, modifier = Modifier.weight(1f).semantics { contentDescription = name })
                        Text((rgb shr shift and 255).toString(), style = MaterialTheme.typography.labelSmall)
                    }
                }
            }
            Surface(Modifier.fillMaxWidth().height(28.dp), color = color, shape = RoundedCornerShape(8.dp)) {}
        }
    }, confirmButton = { TextButton({ apply(rgb) }) { Text("Apply color") } }, dismissButton = { TextButton(close) { Text("Cancel") } })
}
