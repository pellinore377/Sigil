package org.sigil

import androidx.compose.animation.core.*
import androidx.compose.foundation.lazy.LazyItemScope
import androidx.compose.foundation.lazy.staggeredgrid.LazyStaggeredGridItemScope
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp

const val MotionFeedback = 90
const val MotionExit = 120
const val MotionQuick = 160
const val MotionInline = 180
const val MotionArrival = 200
const val MotionMillis = 240
const val MotionSettle = 360
const val MotionStagger = 80
// One mechanical half-turn, and the single expiry cue: two staggered waves, a halo pulse and a bell shake.
const val MotionFlap = 680
const val MotionRing = 1100
const val MotionRingStagger = 180
const val MotionCue = 1300
const val MotionBell = 740
const val MotionBellDelay = 250
const val MotionLoop = 1000

val MotionStandardEasing: Easing = FastOutSlowInEasing
val MotionEnterEasing: Easing = LinearOutSlowInEasing
val MotionExitEasing: Easing = FastOutLinearInEasing
// The reference flap: almost linear through the vertical, eased at both ends.
val MotionFlapEasing: Easing = CubicBezierEasing(.35f, .01f, .65f, 1f)
// CSS ease-out and ease-in-out, as the reference's expiry cue uses them.
val MotionOutEasing: Easing = CubicBezierEasing(0f, 0f, .58f, 1f)
val MotionInOutEasing: Easing = CubicBezierEasing(.42f, 0f, .58f, 1f)

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
/// How far a newly arrived message rises into place.
val MessageArrivalRise = 12.dp

/// A message that has just arrived rises and fades into place, once; one scrolled back to is simply there.
@Composable
internal fun arrivalMotion(arrivals: TimelineArrivals, key: String): Modifier {
    val motion = LocalMotion.current
    val play = remember(key) { arrivals.claim(key) }
    if (!play || motion.reduced) return Modifier
    val progress = remember { Animatable(0f) }
    LaunchedEffect(Unit) { progress.animateTo(1f, motion.enter(MotionArrival)) }
    val rise = with(LocalDensity.current) { MessageArrivalRise.toPx() }
    return Modifier.graphicsLayer { alpha = progress.value; translationY = rise * (1f - progress.value) }
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
/// False for items the buffer has composed but the reader cannot see; nothing off screen should animate.
val LocalItemVisible = staticCompositionLocalOf { true }
