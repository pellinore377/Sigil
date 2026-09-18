package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.text.*
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

/** Panel, ink and one colour per token role the tokenizer emits; appearance settings can supply their own. */
@Immutable
data class CodeTheme(val panel: Color, val ink: Color, val roles: Map<String, Color>)

val LocalCodeTheme = staticCompositionLocalOf<CodeTheme?> { null }

// Panel, ink and token hues read from the reference screenshot; a light panel resolves them through the palette.
private val CodePanelDark = Color(0xff242428)
private val CodeInkDark = Color(0xffbec3cc)
private val DarkRoles = mapOf(
    "keyword" to Color(0xffbf95b7), "type" to Color(0xff8fb9c4), "constant" to Color(0xffc8a15f),
    "function" to Color(0xff93aec4), "string" to Color(0xffa8c48f), "number" to Color(0xffd08a71),
    "comment" to Color(0xff6c7177), "operator" to Color(0xff9fa6ae), "punctuation" to Color(0xff868c94),
    "preprocessor" to Color(0xffc39a6a), "attribute" to Color(0xffc0a6d0), "variable" to Color(0xffd19a9a),
    "tag" to Color(0xffc48f9e), "key" to Color(0xff93aec4), "heading" to Color(0xffb8c4d6),
    "inserted" to Color(0xff8fc48f), "deleted" to Color(0xffc48f8f))
private val LightRoles = listOf(
    "keyword" to "purple2", "type" to "cyan2", "constant" to "orange1", "function" to "blue2",
    "string" to "green2", "number" to "orange2", "comment" to "gray2", "operator" to "gray1",
    "punctuation" to "gray1", "preprocessor" to "orange1", "attribute" to "purple1",
    "variable" to "pink2", "tag" to "pink2", "key" to "blue2", "heading" to "blue1",
    "inserted" to "green2", "deleted" to "red2")

internal fun codeTheme(background: Color, onBackground: Color): CodeTheme {
    val dark = background.luminance() < .18f
    val panel = if (dark) CodePanelDark else lerp(background, onBackground, .055f)
    val ink = if (dark) CodeInkDark else onBackground
    return CodeTheme(panel, ink, if (dark) DarkRoles else LightRoles.associate { (role, name) -> role to textColor(name, panel) })
}

internal fun visibleCodeBlocks(value: RichText) = value.blocks.filter { block ->
    block.kind == "code" && block.start >= 0 && block.end <= value.text.length && block.start < block.end &&
        value.spans.none { it.reveal.isNotEmpty() && it.start < block.end && it.end > block.start }
}
internal fun richSlice(value: RichText, start: Int, end: Int) = RichText(value.text.substring(start, end),
    value.spans.filter { it.start < end && it.end > start }.map { it.copy(start = maxOf(it.start, start) - start, end = minOf(it.end, end) - start) },
    value.blocks.filter { it.start < end && it.end > start }.map { it.copy(start = maxOf(it.start, start) - start, end = minOf(it.end, end) - start) },
    value.codeTokens.filter { it.start >= start && it.end <= end }.map { it.copy(start = it.start - start, end = it.end - start) },
    value.motion.mapNotNull { run ->run.copy(units=run.units.filter {it.first>=start && it.second<=end}.map {it.first-start to it.second-start}).takeIf {it.units.isNotEmpty()} })

@Composable
internal fun CodeBlock(value: RichText, language: String, modifier: Modifier = Modifier, shape: Shape = RoundedCornerShape(12.dp)) = CodePanel(value, language, false, modifier, shape)

@Composable
internal fun AsciiArt(value: RichText) = CodePanel(value, "", true, Modifier, RoundedCornerShape(12.dp))

// An opaque panel, not an alpha wash: the block must read identically on a primary bubble and on a surfaceVariant one.
@Composable
private fun CodePanel(value: RichText, language: String, hug: Boolean, modifier: Modifier, shape: Shape) {
    val scheme = MaterialTheme.colorScheme
    val theme = LocalCodeTheme.current
        ?: remember(scheme.background, scheme.onBackground) { codeTheme(scheme.background, scheme.onBackground) }
    val label = language.takeIf { it.isNotEmpty() }?.lowercase()
    Box(modifier.then(if (hug) Modifier else Modifier.fillMaxWidth()).clip(shape).background(theme.panel)
        .padding(start = 14.dp, end = 14.dp, top = 12.dp, bottom = if (label == null) 12.dp else 10.dp)) {
        CompositionLocalProvider(LocalMessageSurface provides theme.panel, LocalContentColor provides theme.ink, LocalCodeTheme provides theme) {
            Column(Modifier.then(if (hug) Modifier else Modifier.fillMaxWidth())) {
                CodeText(value, theme, true, Int.MAX_VALUE)
                if (label != null) Box(Modifier.fillMaxWidth().padding(top = 8.dp)) {
                    Text(label, Modifier.align(Alignment.CenterEnd).clip(RoundedCornerShape(5.dp))
                        .background(lerp(theme.panel, theme.ink, .10f)).padding(horizontal = 6.dp, vertical = 2.dp),
                        style = MaterialTheme.typography.labelSmall, color = theme.ink.copy(alpha = .55f), maxLines = 1)
                }
            }
        }
    }
}

@Composable
private fun CodeText(value: RichText, theme: CodeTheme, wrap: Boolean, maxLines: Int) {
    val text = remember(value, theme) {
        buildAnnotatedString {
            append(value.text)
            value.codeTokens.forEach { token ->
                if (token.start >= 0 && token.end <= length && token.start < token.end) {
                    theme.roles[token.role]?.let { addStyle(SpanStyle(color = it), token.start, token.end) }
                }
            }
        }
    }
    Text(text, style = MaterialTheme.typography.bodyMedium.copy(fontFamily = LocalCodeFont.current), color = theme.ink, softWrap = wrap, maxLines = maxLines, overflow = TextOverflow.Clip)
}
