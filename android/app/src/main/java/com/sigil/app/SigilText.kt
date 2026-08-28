package com.sigil.app

import androidx.compose.animation.core.*
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.scale
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.json.JSONArray
import org.json.JSONObject
import uniffi.sigil_engine.sigiltextMotion
import uniffi.sigil_engine.sigiltextRender

/// One character with everything the engine decided about it.
private data class Glyph(
    val ch: String,
    val index: Int,
    val colour: Color?,
    val markRgb: Color?,
    val anim: String,
    val bold: Boolean,
    val italic: Boolean,
    val underline: Boolean,
    val strike: Boolean,
    val sizeScale: Double,
)

/// The motion specification, fetched once. Timings, easings and displacements
/// all come from here — nothing in this file chooses a number.
private object Motion {
    private val byName: Map<String, JSONObject> = run {
        val root = JSONObject(sigiltextMotion())
        val out = mutableMapOf<String, JSONObject>()
        val arr = root.getJSONArray("animations")
        for (i in 0 until arr.length()) {
            val a = arr.getJSONObject(i)
            out[a.getString("name")] = a
        }
        out
    }
    val staggerMs: Int = JSONObject(sigiltextMotion()).optInt("staggerMs", 90)

    fun spec(name: String): JSONObject? = byName[name]
    fun duration(name: String): Int = spec(name)?.optInt("durationMs") ?: 0
    fun param(name: String, key: String, fallback: Double): Double =
        spec(name)?.optJSONObject("params")?.optDouble(key, fallback) ?: fallback

    /// The engine ships cubic Bézier control points precisely so every platform
    /// applies the same curve; Compose takes them directly.
    fun easing(name: String): Easing {
        val e = spec(name)?.optJSONArray("easing") ?: return LinearEasing
        return CubicBezierEasing(
            e.getDouble(0).toFloat(), e.getDouble(1).toFloat(),
            e.getDouble(2).toFloat(), e.getDouble(3).toFloat(),
        )
    }

    fun stagger(name: String, index: Int): Int {
        val s = spec(name) ?: return 0
        val base = index * staggerMs
        val modulo = s.opt("staggerModulo")
        val stride = s.optJSONObject("params")?.optInt("staggerStride", 0) ?: 0
        val raw = if (stride > 0) index * stride else base
        return if (modulo is Int) raw % modulo else raw
    }
}

@Composable
fun SigilText(source: String, baseSize: Int = 19, modifier: Modifier = Modifier) {
    val glyphs = remember(source) { parse(source) }
    FlowGlyphs(glyphs, baseSize, modifier)
}

/// Words are laid out as units so a line break never lands inside one; the
/// characters inside an animated word are individually placed.
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun FlowGlyphs(glyphs: List<Glyph>, baseSize: Int, modifier: Modifier) {
    val words = remember(glyphs) {
        val out = mutableListOf<List<Glyph>>()
        var cur = mutableListOf<Glyph>()
        glyphs.forEach { g ->
            if (g.ch == " ") { if (cur.isNotEmpty()) { out.add(cur); cur = mutableListOf() }; out.add(listOf(g)) }
            else cur.add(g)
        }
        if (cur.isNotEmpty()) out.add(cur)
        out
    }
    FlowRow(modifier) {
        words.forEach { w -> Row { w.forEach { AnimatedGlyph(it, baseSize) } } }
    }
}

@Composable
private fun AnimatedGlyph(g: Glyph, baseSize: Int) {
    val infinite = rememberInfiniteTransition(label = g.anim.ifEmpty { "static" })
    val density = LocalDensity.current

    var dx = 0f; var dy = 0f; var scale = 1f; var alpha = 1f; var rotation = 0f

    when (g.anim) {
        "shake" -> {
            val amp = Motion.param("shake", "amplitudePx", 0.8).toFloat()
            dx = infinite.animateFloat(
                -amp, amp,
                infiniteRepeatable(
                    tween(Motion.duration("shake"), Motion.stagger("shake", g.index), Motion.easing("shake")),
                    RepeatMode.Reverse,
                ), label = "shake",
            ).value
        }
        "wave" -> {
            val amp = Motion.param("wave", "amplitudePx", 1.8).toFloat()
            dy = infinite.animateFloat(
                -amp, amp,
                infiniteRepeatable(
                    tween(Motion.duration("wave"), Motion.stagger("wave", g.index), Motion.easing("wave")),
                    RepeatMode.Reverse,
                ), label = "wave",
            ).value
        }
        "pulse" -> {
            scale = infinite.animateFloat(
                Motion.param("pulse", "minScale", 1.0).toFloat(),
                Motion.param("pulse", "maxScale", 1.18).toFloat(),
                infiniteRepeatable(
                    tween(Motion.duration("pulse") / 2, Motion.stagger("pulse", g.index), Motion.easing("pulse")),
                    RepeatMode.Reverse,
                ), label = "pulse",
            ).value
        }
        "glow" -> {
            alpha = infinite.animateFloat(
                Motion.param("glow", "minAlpha", 0.35).toFloat().coerceAtLeast(0.35f),
                Motion.param("glow", "maxAlpha", 1.0).toFloat(),
                infiniteRepeatable(
                    tween(Motion.duration("glow") / 2, 0, Motion.easing("glow")), RepeatMode.Reverse,
                ), label = "glow",
            ).value
        }
        "barrel" -> {
            rotation = infinite.animateFloat(
                0f, Motion.param("barrel", "rotationDeg", 360.0).toFloat(),
                infiniteRepeatable(tween(Motion.duration("barrel"), 0, LinearEasing)), label = "barrel",
            ).value
        }
        "flip" -> rotation = Motion.param("flip", "rotationDeg", 180.0).toFloat()
    }

    val px = with(density) { 1.dp.toPx() }
    androidx.compose.material3.Text(
        text = g.ch,
        modifier = Modifier
            .graphicsLayer {
                translationX = dx * px
                translationY = dy * px
                scaleX = scale; scaleY = scale
                rotationZ = rotation
            }
            .alpha(alpha)
            .then(if (g.markRgb != null) Modifier.background(g.markRgb.copy(alpha = 0.32f)) else Modifier),
        style = TextStyle(
            color = g.colour ?: Color.White,
            fontSize = (baseSize * g.sizeScale).sp,
            fontWeight = if (g.bold) FontWeight.Bold else null,
            fontStyle = if (g.italic) FontStyle.Italic else null,
            textDecoration = when {
                g.strike && g.underline -> TextDecoration.combine(listOf(TextDecoration.LineThrough, TextDecoration.Underline))
                g.strike -> TextDecoration.LineThrough
                g.underline -> TextDecoration.Underline
                else -> null
            },
        ),
    )
}

/// Flatten the engine's spans onto characters. Offsets are character indices
/// into `body`, so the two stay aligned without re-parsing.
private fun parse(source: String): List<Glyph> {
    val out = JSONObject(sigiltextRender(source))
    val body = out.getString("body")
    val effects = out.getJSONArray("effects")

    return body.mapIndexed { i, ch ->
        var colour: Color? = null
        var mark: Color? = null
        var anim = ""
        var bold = false; var italic = false; var underline = false; var strike = false
        var size = 1.0

        for (n in 0 until effects.length()) {
            val e = effects.getJSONObject(n)
            val start = e.getInt("start"); val end = e.getInt("end")
            if (i < start || i >= end) continue

            if (e.optBoolean("bold")) bold = true
            if (e.optBoolean("italic")) italic = true
            if (e.optBoolean("underline")) underline = true
            if (e.optBoolean("strike")) strike = true
            if (e.has("sizeScale")) size = e.getDouble("sizeScale")
            if (e.has("animation")) anim = e.getString("animation")
            e.optJSONObject("markRgb")?.let { mark = hexColour(it.getString("dark")) }

            e.optJSONObject("color")?.let { c ->
                val span = (end - start).coerceAtLeast(1)
                val frac = if (span > 1) (i - start).toFloat() / (span - 1) else 0f
                colour = when (c.optString("type")) {
                    "solid" -> c.optJSONObject("rgb")?.let { hexColour(it.getString("dark")) }
                    "gradient" -> gradientAt(c.optJSONArray("rgb"), frac)
                    "rainbow" -> rainbowAt(frac, c.optDouble("saturation", 0.62), c.optDouble("lightness", 0.62))
                    else -> colour
                }
            }
        }
        Glyph(ch.toString(), i, colour, mark, anim, bold, italic, underline, strike, size)
    }
}

private fun hexColour(s: String): Color = Color(("ff" + s.removePrefix("#")).toLong(16))

private fun gradientAt(stops: JSONArray?, frac: Float): Color? {
    if (stops == null || stops.length() == 0) return null
    if (stops.length() == 1) return hexColour(stops.getJSONObject(0).getString("dark"))
    val f = frac.coerceIn(0f, 1f) * (stops.length() - 1)
    val i = f.toInt().coerceAtMost(stops.length() - 2)
    val t = f - i
    val a = hexColour(stops.getJSONObject(i).getString("dark"))
    val b = hexColour(stops.getJSONObject(i + 1).getString("dark"))
    return Color(a.red + (b.red - a.red) * t, a.green + (b.green - a.green) * t, a.blue + (b.blue - a.blue) * t)
}

private fun rainbowAt(frac: Float, sat: Double, lum: Double): Color {
    val h = frac.coerceIn(0f, 0.999f) * 6f
    val c = (1 - kotlin.math.abs(2 * lum - 1)) * sat
    val x = c * (1 - kotlin.math.abs((h % 2f) - 1))
    val m = lum - c / 2
    val (r, g, b) = when (h.toInt()) {
        0 -> Triple(c, x, 0.0); 1 -> Triple(x, c, 0.0); 2 -> Triple(0.0, c, x)
        3 -> Triple(0.0, x, c); 4 -> Triple(x, 0.0, c); else -> Triple(c, 0.0, x)
    }
    return Color((r + m).toFloat(), (g + m).toFloat(), (b + m).toFloat())
}
