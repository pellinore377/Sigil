package org.sigil

import androidx.compose.animation.core.*
import androidx.compose.runtime.staticCompositionLocalOf

data class MotionPolicy(val reduced: Boolean = false) {
    fun <T> tween(durationMillis: Int = 240, delayMillis: Int = 0, easing: Easing = FastOutSlowInEasing): TweenSpec<T> =
        androidx.compose.animation.core.tween(if (reduced) 0 else durationMillis, if (reduced) 0 else delayMillis, easing)
    fun delay(milliseconds: Long) = if (reduced) 0L else milliseconds
}
val LocalMotion = staticCompositionLocalOf { MotionPolicy() }
val LocalSystemReducedMotion = staticCompositionLocalOf { false }
