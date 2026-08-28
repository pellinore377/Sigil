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
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import uniffi.sigil_engine.engineVersion

/// Proof that the engine drives the frontend: SigilText is parsed in Rust, its
/// palette and motion resolved in Rust, and only drawn here.
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent { MaterialTheme(colorScheme = darkColorScheme()) { Screen() } }
    }
}

private val SAMPLES = listOf(
    "red::danger; and green::safe; and blue::calm;",
    "red1-blue3::a gradient across these characters;",
    "rainbow::every colour of the rainbow;",
    "bold::bold; italic::italic; strike::struck; underline::underlined;",
    "big3::huge; normal small3::tiny;",
    "mark::highlighted; and mark::red::red highlight;",
    "shake::this text shakes;",
    "wave::this text waves along;",
    "pulse::this pulses; glow::this glows;",
    "barrel::barrel roll; and flip::flipped;",
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
            "SigilText from sigil-engine ${engineVersion()} — parsed, coloured and " +
                "timed in Rust; Compose only draws.",
            color = Color(0xFF99999E), fontSize = 13.sp,
        )
        Spacer(Modifier.height(4.dp))
        SAMPLES.forEach { src ->
            Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
                Text(src, color = Color(0xFF6A6A6D), fontSize = 11.sp)
                SigilText(src)
            }
        }
    }
}
