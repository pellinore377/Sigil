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
import org.sigil.LocalTextPlatformStyle
import androidx.compose.ui.text.PlatformTextStyle

internal fun ComponentActivity.setSigilContent(content: @Composable () -> Unit) {
    enableEdgeToEdge()
    setContent {
        var reducedMotion by remember { mutableStateOf(!android.animation.ValueAnimator.areAnimatorsEnabled()) }
        DisposableEffect(Unit) {
            val observer = object : android.database.ContentObserver(android.os.Handler(android.os.Looper.getMainLooper())) {
                override fun onChange(selfChange: Boolean) { reducedMotion = android.provider.Settings.Global.getFloat(contentResolver, android.provider.Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f }
            }
            contentResolver.registerContentObserver(android.provider.Settings.Global.getUriFor(android.provider.Settings.Global.ANIMATOR_DURATION_SCALE), false, observer)
            onDispose { contentResolver.unregisterContentObserver(observer) }
        }
        if (Build.VERSION.SDK_INT <= 30) {
            val view = LocalView.current
            val ime = WindowInsets.ime.getBottom(LocalDensity.current)
            // Older Android retains its window pan until it rechecks the focused field's bounds.
            LaunchedEffect(ime) { withFrameNanos { }; view.rootView.requestLayout() }
        }
        CompositionLocalProvider(org.sigil.LocalKeepScreenAwake provides { enabled ->
            val view = LocalView.current
            DisposableEffect(view, enabled) {
                val previous = view.keepScreenOn
                if (enabled) view.keepScreenOn = true
                onDispose { view.keepScreenOn = previous }
            }
        }, org.sigil.LocalSystemReducedMotion provides reducedMotion, LocalTextPlatformStyle provides PlatformTextStyle(includeFontPadding = false), LocalSystemAppearance provides { dark ->
            WindowCompat.getInsetsController(window, window.decorView).apply {
                isAppearanceLightStatusBars = !dark
                isAppearanceLightNavigationBars = !dark
            }
        }) { content() }
    }
}
