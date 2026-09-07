package org.sigil.compose

import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.runtime.*
import androidx.compose.ui.graphics.toArgb
import org.sigil.NativeCore
import org.sigil.SigilApp

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val preferences = getSharedPreferences("appearance", MODE_PRIVATE)
        setContent {
            var backAvailable by remember { mutableStateOf(false) }
            var goBack by remember { mutableStateOf<() -> Unit>({}) }
            BackHandler(backAvailable) { goBack() }
            val dynamicAccent = if (Build.VERSION.SDK_INT >= 31) dynamicLightColorScheme(this).primary.toArgb() and 0xffffff else null
            SigilApp(NativeCore::palette, NativeCore::analyze, NativeCore::redact,
                read = { preferences.getString(it, null) }, write = { key, value -> preferences.edit().putString(key, value).apply() },
                dynamicAccent = dynamicAccent, onBackAvailable = { available, action -> backAvailable = available; goBack = action })
        }
    }
}
