package org.sigil

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import kotlin.math.*

/** A colour in OKLCH: perceptual lightness, chroma and hue in radians. */
internal data class Oklch(val l: Float, val c: Float, val h: Float)

private fun linear(c: Float) = if (c <= .04045f) c / 12.92f else ((c + .055f) / 1.055f).pow(2.4f)
private fun encode(c: Float) = if (c <= .0031308f) c * 12.92f else 1.055f * c.pow(1 / 2.4f) - .055f
private fun cbrt(x: Float) = sign(x) * abs(x).pow(1 / 3f)

internal fun Color.oklch(): Oklch {
    val r = linear(red); val g = linear(green); val b = linear(blue)
    val l = cbrt(.4122214708f * r + .5363325363f * g + .0514459929f * b)
    val m = cbrt(.2119034982f * r + .6806995451f * g + .1073969566f * b)
    val s = cbrt(.0883024619f * r + .2817188376f * g + .6299787005f * b)
    val bigL = .2104542553f * l + .7936177850f * m - .0040720468f * s
    val a = 1.9779984951f * l - 2.4285922050f * m + .4505937099f * s
    val bb = .0259040371f * l + .7827717662f * m - .8086757660f * s
    return Oklch(bigL, hypot(a, bb), atan2(bb, a))
}

/** Linear sRGB channels, unclamped, so the caller can tell whether the colour is in gamut. */
private fun Oklch.linearRgb(): FloatArray {
    val a = c * cos(h); val b = c * sin(h)
    val l = (this.l + .3963377774f * a + .2158037573f * b).pow(3)
    val m = (this.l - .1055613458f * a - .0638541728f * b).pow(3)
    val s = (this.l - .0894841775f * a - 1.2914855480f * b).pow(3)
    return floatArrayOf(4.0767416621f * l - 3.3077115913f * m + .2309699292f * s,
        -1.2684380046f * l + 2.6097574011f * m - .3413193965f * s,
        -.0041960863f * l - .7034186147f * m + 1.7076147010f * s)
}
private fun Oklch.inGamut() = linearRgb().all { it in -1e-4f..1.0001f }

/** The most chroma this lightness and hue can carry in sRGB. */
internal fun maxChroma(l: Float, h: Float): Float {
    var lo = 0f; var hi = .4f
    repeat(18) { val mid = (lo + hi) / 2; if (Oklch(l, mid, h).inGamut()) lo = mid else hi = mid }
    return lo
}
private fun Oklch.clamped() = copy(c = minOf(c, maxChroma(l, h)))
internal fun Oklch.toColor(): Color = clamped().linearRgb().map { encode(it.coerceIn(0f, 1f)) }.let { Color(it[0], it[1], it[2]) }
private fun Oklch.luminance() = clamped().linearRgb().let { .2126f * it[0].coerceIn(0f, 1f) + .7152f * it[1].coerceIn(0f, 1f) + .0722f * it[2].coerceIn(0f, 1f) }

/** Distance in OKLab; about .02 is a just-noticeable difference. */
internal fun deltaE(a: Color, b: Color): Float {
    val x = a.oklch(); val y = b.oklch()
    return sqrt((x.l - y.l).pow(2) + (x.c * cos(x.h) - y.c * cos(y.h)).pow(2) + (x.c * sin(x.h) - y.c * sin(y.h)).pow(2))
}

private const val ShadeStep = .05f
private const val ShadeMinStep = .03f
// Below this chroma a light step reads as plain ink rather than its hue.
private const val ShadeMinChroma = .08f
private const val ShadeGrid = 200
// A hair over 4.5:1, so rounding to 8-bit channels never lands under it.
private const val ShadeContrast = 4.56f
private val shadeCache = HashMap<Triple<String, Color, List<Color>>, Color>()

/**
 * A named colour's three brightness steps on the given grounds: each clears 4.5:1, 1 stays lighter than 2 and 2 than 3,
 * and on dark grounds the steps stop rising where the hue would wash out to white.
 */
internal fun shadeColor(name: String, surface: Color, alternates: List<Color>): Color {
    val key = Triple(name, surface, alternates)
    shadeCache[key]?.let { return it }
    val hue = when (name.dropLast(1)) { "red" -> 5f; "orange" -> 28f; "yellow" -> 52f; "green" -> 135f; "cyan" -> 185f; "blue" -> 220f; "purple" -> 275f; "pink" -> 330f; else -> 0f }
    val shade = (name.lastOrNull()?.digitToIntOrNull() ?: 2).coerceIn(1, 3)
    val gray = name.startsWith("gray")
    // The design's nominal steps, read in OKLCH so correction moves lightness and keeps hue and chroma.
    val nominal = (1..3).map { Color.hsl(hue, if (gray) 0f else .80f, .66f - it * .08f).oklch() }
    val h = nominal[1].h
    val grounds = (alternates + surface).map { it.luminance() }
    fun contrast(c: Oklch): Float { val y = c.luminance(); return grounds.minOf { g -> (maxOf(y, g) + .05f) / (minOf(y, g) + .05f) } }
    val grid = (0..ShadeGrid).map { it / ShadeGrid.toFloat() }
    fun at(l: Float, s: Int) = Oklch(l, nominal[s - 1].c, h)
    // Bounds of the readable band on each side of the ground; contrast grows monotonically away from it.
    val floor = grid.reversed().takeWhile { contrast(at(it, 2)) >= ShadeContrast }.lastOrNull()
    val ceiling = grid.takeWhile { contrast(at(it, 2)) >= ShadeContrast }.lastOrNull()
    val lights: List<Float> = when {
        floor != null && (ceiling == null || floor - .5f <= .5f - ceiling) -> {
            val cap = if (gray) .97f else grid.lastOrNull { maxChroma(it, h) >= ShadeMinChroma } ?: .97f
            val l3 = maxOf(nominal[2].l, floor)
            var l2 = maxOf(nominal[1].l, l3 + ShadeStep)
            var l1 = maxOf(nominal[0].l, l2 + ShadeStep)
            if (l1 > cap) { l1 = maxOf(cap, l3 + 2 * ShadeMinStep); l2 = minOf(l2, (l1 + l3) / 2) }
            listOf(l1, l2, l3)
        }
        ceiling != null -> {
            val l1 = minOf(nominal[0].l, ceiling)
            val l2 = minOf(nominal[1].l, l1 - ShadeStep)
            listOf(l1, l2, minOf(nominal[2].l, l2 - ShadeStep))
        }
        else -> List(3) { grid.maxBy { contrast(at(it, 2)) } }
    }
    // Each step's own chroma shifts its luminance a little: nudge away from the ground until it clears.
    val dark = floor == null || (ceiling != null && floor - .5f > .5f - ceiling)
    var l = lights[shade - 1].coerceIn(0f, 1f)
    while (contrast(at(l, shade)) < ShadeContrast && l in .005f..0.995f) l += if (dark) -.005f else .005f
    val result = at(l, shade).toColor()
    if (shadeCache.size > 1024) shadeCache.clear()
    shadeCache[key] = result
    return result
}
