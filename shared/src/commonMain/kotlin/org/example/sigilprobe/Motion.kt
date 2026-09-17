package org.sigil

import androidx.compose.animation.core.*
import androidx.compose.foundation.lazy.LazyItemScope
import androidx.compose.foundation.lazy.staggeredgrid.LazyStaggeredGridItemScope
import androidx.compose.runtime.Composable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.IntOffset

const val MotionFeedback = 90
const val MotionExit = 120
const val MotionQuick = 160
const val MotionInline = 180
const val MotionMillis = 240
const val MotionSettle = 360
const val MotionStagger = 80
const val MotionLoop = 1000

val MotionStandardEasing: Easing = FastOutSlowInEasing
val MotionEnterEasing: Easing = LinearOutSlowInEasing
val MotionExitEasing: Easing = FastOutLinearInEasing

data class MotionPolicy(val reduced: Boolean = false) {
    fun <T> tween(durationMillis: Int = MotionMillis, delayMillis: Int = 0, easing: Easing = MotionStandardEasing): TweenSpec<T> =
        androidx.compose.animation.core.tween(if (reduced) 0 else durationMillis, if (reduced) 0 else delayMillis, easing)
    fun <T> enter(durationMillis: Int = MotionMillis, delayMillis: Int = 0): TweenSpec<T> = tween(durationMillis, delayMillis, MotionEnterEasing)
    fun <T> exit(durationMillis: Int = MotionExit, delayMillis: Int = 0): TweenSpec<T> = tween(durationMillis, delayMillis, MotionExitEasing)
    // Callers must branch on `reduced` first; a zero-duration repeat would spin the frame clock.
    fun <T> loop(durationMillis: Int = MotionLoop): InfiniteRepeatableSpec<T> =
        infiniteRepeatable(androidx.compose.animation.core.tween(durationMillis, easing = LinearEasing))
    fun delay(milliseconds: Long) = if (reduced) 0L else milliseconds
}
@Composable
fun LazyItemScope.itemMotion(): Modifier {
    val motion = LocalMotion.current
    return Modifier.animateItem(motion.enter(), motion.tween<IntOffset>(MotionMillis), motion.exit())
}
@Composable
fun LazyStaggeredGridItemScope.itemMotion(): Modifier {
    val motion = LocalMotion.current
    return Modifier.animateItem(motion.enter(), motion.tween<IntOffset>(MotionMillis), motion.exit())
}
val LocalMotion = staticCompositionLocalOf { MotionPolicy() }
val LocalSystemReducedMotion = staticCompositionLocalOf { false }
