package org.sigil.compose

import android.os.Build
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.ime
import androidx.compose.runtime.*
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.core.view.WindowCompat
import org.sigil.LocalSystemAppearance

internal fun ComponentActivity.setSigilContent(content: @Composable () -> Unit) {
    enableEdgeToEdge()
    setContent {
        if (Build.VERSION.SDK_INT <= 30) {
            val view = LocalView.current
            val ime = WindowInsets.ime.getBottom(LocalDensity.current)
            // Older Android retains its window pan until it rechecks the focused field's bounds.
            LaunchedEffect(ime) { withFrameNanos { }; view.rootView.requestLayout() }
        }
        CompositionLocalProvider(LocalSystemAppearance provides { dark ->
            WindowCompat.getInsetsController(window, window.decorView).apply {
                isAppearanceLightStatusBars = !dark
                isAppearanceLightNavigationBars = !dark
            }
        }) { content() }
    }
}
