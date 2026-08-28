package com.sigil.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.json.JSONObject
import uniffi.sigil_engine.engineVersion
import uniffi.sigil_engine.sigiltextRender

/// The engine parses SigilText and resolves the palette; this file only draws.
/// Nothing here decides what `red` looks like — see core/src/timeline/palette.rs.
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent { MaterialTheme(colorScheme = androidx.compose.material3.darkColorScheme()) { Screen() } }
    }
}

private val SAMPLES = listOf(
    "red::danger; and green::safe; and blue::calm;",
    "red1-blue3::a gradient across these characters;",
    "rainbow::every colour of the rainbow;",
    "bold::bold; italic::italic; strike::struck; underline::underlined;",
    "big3::huge; normal small3::tiny;",
    "mark::highlighted; and mark::red::red highlight;",
    "shake::shaking; wave::waving; pulse::pulsing;",
    "**markdown bold** and `inline code` still work",
    "std::vector is not a modifier, so it stays literal",
)

@Composable
private fun Screen() {
    Column(
        Modifier.fillMaxSize()
            .background(Color(0xFF141416))
            .verticalScroll(rememberScrollState())
            .padding(20.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Text("Sigil", color = Color.White, fontSize = 30.sp, fontWeight = FontWeight.Bold)
        Text(
            "SigilText rendered by sigil-engine ${engineVersion()} — parsed in Rust, " +
                "colours resolved in Rust, drawn by Compose.",
            color = Color(0xFF99999E), fontSize = 13.sp,
        )
        Spacer(Modifier.height(4.dp))
        SAMPLES.forEach { src ->
            Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                Text(src, color = Color(0xFF6A6A6D), fontSize = 11.sp)
                Text(render(src), fontSize = 19.sp, color = Color.White)
            }
        }
    }
}

/// Walk the engine's spans onto an AnnotatedString. Character offsets index the
/// plain body, so the two stay aligned without re-parsing anything here.
private fun render(source: String): AnnotatedString {
    val out = JSONObject(sigiltextRender(source))
    val body = out.getString("body")
    val effects = out.getJSONArray("effects")

    return buildAnnotatedString {
        append(body)
        for (i in 0 until effects.length()) {
            val e = effects.getJSONObject(i)
            val start = e.getInt("start").coerceIn(0, body.length)
            val end = e.getInt("end").coerceIn(start, body.length)
            if (start == end) continue

            val c = e.optJSONObject("color")
            val kind = c?.optString("type")

            // Gradient and rainbow colour each character differently, so they are
            // styled per character rather than as one span. The engine ships the
            // resolved stops and the rainbow constants; nothing is invented here.
            if (kind == "gradient" || kind == "rainbow") {
                val span = (end - start).coerceAtLeast(1)
                for (n in start until end) {
                    val frac = if (span > 1) (n - start).toFloat() / (span - 1) else 0f
                    val col = if (kind == "rainbow") {
                        rainbow(frac, c.optDouble("saturation", 0.62), c.optDouble("lightness", 0.62))
                    } else {
                        gradient(c.optJSONArray("rgb"), frac)
                    }
                    if (col != null) addStyle(SpanStyle(color = col), n, n + 1)
                }
            }

            val colour = if (kind == "solid") {
                c?.optJSONObject("rgb")?.let { hex(it.getString("dark")) }
            } else null

            addStyle(
                SpanStyle(
                    color = colour ?: Color.Unspecified,
                    background = e.optJSONObject("markRgb")
                        ?.let { hex(it.getString("dark")).copy(alpha = 0.32f) } ?: Color.Unspecified,
                    fontWeight = if (e.optBoolean("bold")) FontWeight.Bold else null,
                    fontStyle = if (e.optBoolean("italic")) FontStyle.Italic else null,
                    fontSize = e.optDouble("sizeScale", 1.0).let { if (it == 1.0) androidx.compose.ui.unit.TextUnit.Unspecified else (19 * it).sp },
                    textDecoration = when {
                        e.optBoolean("strike") && e.optBoolean("underline") ->
                            TextDecoration.combine(listOf(TextDecoration.LineThrough, TextDecoration.Underline))
                        e.optBoolean("strike") -> TextDecoration.LineThrough
                        e.optBoolean("underline") -> TextDecoration.Underline
                        else -> null
                    },
                ),
                start, end,
            )
        }
    }
}

private fun hex(s: String): Color = Color(("ff" + s.removePrefix("#")).toLong(16))

/// Linear interpolation between the engine's resolved stops.
private fun gradient(stops: org.json.JSONArray?, frac: Float): Color? {
    if (stops == null || stops.length() == 0) return null
    if (stops.length() == 1) return hex(stops.getJSONObject(0).getString("dark"))
    val f = frac.coerceIn(0f, 1f) * (stops.length() - 1)
    val i = f.toInt().coerceAtMost(stops.length() - 2)
    val t = f - i
    val a = hex(stops.getJSONObject(i).getString("dark"))
    val b = hex(stops.getJSONObject(i + 1).getString("dark"))
    return Color(
        red = a.red + (b.red - a.red) * t,
        green = a.green + (b.green - a.green) * t,
        blue = a.blue + (b.blue - a.blue) * t,
    )
}

/// Hue cycled across the run at the saturation and lightness the spec fixes.
private fun rainbow(frac: Float, sat: Double, lum: Double): Color {
    val h = (frac.coerceIn(0f, 0.999f) * 6f)
    val c = (1 - kotlin.math.abs(2 * lum - 1)) * sat
    val x = c * (1 - kotlin.math.abs((h % 2f) - 1))
    val m = lum - c / 2
    val (r, g, b) = when (h.toInt()) {
        0 -> Triple(c, x.toDouble(), 0.0)
        1 -> Triple(x.toDouble(), c, 0.0)
        2 -> Triple(0.0, c, x.toDouble())
        3 -> Triple(0.0, x.toDouble(), c)
        4 -> Triple(x.toDouble(), 0.0, c)
        else -> Triple(c, 0.0, x.toDouble())
    }
    return Color((r + m).toFloat(), (g + m).toFloat(), (b + m).toFloat())
}
