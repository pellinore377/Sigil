package org.sigil

import androidx.compose.animation.animateColor
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.updateTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.text.selection.LocalTextSelectionColors
import androidx.compose.foundation.text.selection.TextSelectionColors
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.PlatformTextStyle
import androidx.compose.ui.text.style.LineHeightStyle
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.text.font.*
import androidx.compose.ui.unit.sp
import org.jetbrains.compose.resources.Font
import sigil.shared.generated.resources.*

data class Appearance(val font: String = "Newsreader", val mode: String = "System", val accent: Int = 0x555555, val dynamic: Boolean = false,
    val textScale: Float = 1f, val compact: Boolean = false, val previewLines: Int = 1, val gradient: Boolean = false)
data class ChatTheme(val accent: Int? = null, val gradient: Boolean? = null)
private data class ThemeTarget(val seed: Int, val dark: Boolean, val chat: String?, val tinted: Boolean)

internal val LocalChatTint = staticCompositionLocalOf { 0f }
internal val LocalAppearance = staticCompositionLocalOf { Appearance() }
val LocalCodeFont = staticCompositionLocalOf<FontFamily> { FontFamily.Monospace }
val LocalSystemAppearance = staticCompositionLocalOf<(Boolean) -> Unit> { {} }
val LocalTextPlatformStyle = staticCompositionLocalOf<PlatformTextStyle?> { null }

internal fun parseAccent(text: String): Int? = text.removePrefix("#").takeIf { it.length == 6 }?.toIntOrNull(16)?.takeIf { it in 0..0xffffff }
internal fun accentText(color: Int) = color.toString(16).padStart(6, '0').uppercase()

internal fun decodeAppearance(value: String?): Appearance {
    val parts = value?.split('|') ?: return Appearance()
    return Appearance(
        font = parts.getOrNull(0)?.takeIf { it == "Google Sans Flex" } ?: "Newsreader",
        mode = parts.getOrNull(1)?.takeIf { it in listOf("Light", "Dark") } ?: "System",
        accent = parts.getOrNull(2)?.let(::parseAccent) ?: 0x555555,
        dynamic = parts.getOrNull(3) == "true",
        textScale = parts.getOrNull(4)?.toFloatOrNull()?.takeIf { it.isFinite() && it in .85f..1.3f } ?: 1f,
        compact = parts.getOrNull(5) == "true",
        previewLines = parts.getOrNull(6)?.toIntOrNull()?.takeIf { it in 0..2 } ?: 1,
        gradient = parts.getOrNull(7) == "true",
    )
}
internal fun Appearance.encode() = "$font|$mode|${accentText(accent)}|$dynamic|$textScale|$compact|$previewLines|$gradient"
internal fun decodeChat(value: String?): ChatTheme {
    val parts = value?.split('|') ?: return ChatTheme()
    return ChatTheme(parts.getOrNull(0)?.let(::parseAccent), parts.getOrNull(1)?.toBooleanStrictOrNull())
}
internal fun ChatTheme.encode() = "${accent?.let(::accentText) ?: ""}|${gradient ?: ""}"

@Composable
internal fun SigilTheme(appearance: Appearance, chat: ChatTheme? = null, dynamicAccent: Int? = null,
    palette: (Int, Boolean) -> String, chatKey: String? = null, content: @Composable () -> Unit) {
    val dark = when (appearance.mode) { "Dark" -> true; "Light" -> false; else -> isSystemInDarkTheme() }
    val systemAppearance = LocalSystemAppearance.current
    SideEffect { systemAppearance(dark) }
    val seed = chat?.accent ?: if (appearance.dynamic) dynamicAccent ?: appearance.accent else appearance.accent
    val transition = updateTransition(ThemeTarget(seed, dark, chatKey, chat != null), label = "Appearance")
    val tint by transition.animateFloat(transitionSpec = { tween(180, if (targetState.chat != null && initialState.chat != targetState.chat) MotionMillis else 0) }, label = "Conversation tint") { if (it.tinted) 1f else 0f }
    val palettes = remember(palette) { linkedMapOf<Pair<Int, Boolean>, List<Color>>() }
    val colors = (0..8).map { index ->
        transition.animateColor(transitionSpec = { tween(180, if (targetState.chat != null && initialState.chat != targetState.chat) MotionMillis else 0) }, label = "Theme color") { target ->
            palettes.getOrPut(target.seed to target.dark) {
                if (palettes.size >= 4) palettes.remove(palettes.keys.first())
                palette(target.seed, target.dark).split(',').map { Color(0xff000000L or it.toLong(16)) }
            }[index]
        }.value
    }
    val base = if (dark) darkColorScheme() else lightColorScheme()
    val scheme = base.copy(
        background = lerp(colors[0], Color.Black, if (dark) .12f else .025f), onBackground = colors[1], surface = colors[0], onSurface = colors[1],
        primary = colors[4], onPrimary = colors[5], primaryContainer = colors[6], onPrimaryContainer = colors[7],
        secondary = colors[4], onSecondary = colors[5], secondaryContainer = colors[6], onSecondaryContainer = colors[7],
        tertiary = colors[4], onTertiary = colors[5], tertiaryContainer = colors[6], onTertiaryContainer = colors[7],
        surfaceVariant = colors[6], onSurfaceVariant = colors[7], outline = colors[8], outlineVariant = colors[8].copy(alpha = .3f),
        surfaceTint = Color.Transparent, surfaceContainer = colors[2], surfaceContainerHigh = colors[2],
        surfaceContainerHighest = colors[6], surfaceContainerLow = colors[0], surfaceContainerLowest = colors[0],
        inverseSurface = colors[1], inverseOnSurface = colors[0], inversePrimary = colors[0],
    )
    val family = if (appearance.font == "Newsreader") FontFamily(
        Font(Res.font.newsreader), Font(Res.font.newsreader_italic, style = FontStyle.Italic)
    ) else FontFamily(Font(Res.font.google_sans_flex), Font(Res.font.google_sans_flex_semibold, FontWeight.SemiBold))
    val textPlatformStyle = LocalTextPlatformStyle.current
    fun style(size: Int, line: Int, weight: FontWeight = FontWeight.Normal) = TextStyle(fontFamily = family, fontSize = (size * appearance.textScale).sp, lineHeight = (line * appearance.textScale).sp, fontWeight = weight,
        platformStyle = textPlatformStyle, lineHeightStyle = LineHeightStyle(LineHeightStyle.Alignment.Center, LineHeightStyle.Trim.Both))
    val typography = Typography(
        displayLarge = style(52, 60), displayMedium = style(44, 52), displaySmall = style(36, 44),
        headlineLarge = style(34, 42), headlineMedium = style(28, 36), headlineSmall = style(24, 32),
        titleLarge = style(23, 30), titleMedium = style(19, 26), titleSmall = style(17, 24),
        bodyLarge = style(18, 26), bodyMedium = style(16, 23), bodySmall = style(14, 20),
        labelLarge = style(16, 22), labelMedium = style(14, 20), labelSmall = style(12, 18),
    )
    CompositionLocalProvider(LocalCodeFont provides FontFamily(Font(Res.font.google_sans_code)), LocalChatTint provides tint, LocalAppearance provides appearance) {
        MaterialTheme(colorScheme = scheme, typography = typography) {
            CompositionLocalProvider(LocalTextSelectionColors provides TextSelectionColors(scheme.primary, scheme.primary.copy(alpha = .3f)), content = content)
        }
    }
}
