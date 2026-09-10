package org.sigil

import android.animation.ValueAnimator
import com.airbnb.lottie.LottieAnimationView
import com.airbnb.lottie.LottieComposition
import com.airbnb.lottie.LottieCompositionFactory
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.viewinterop.AndroidView
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.jetbrains.compose.resources.ExperimentalResourceApi
import sigil.shared.generated.resources.Res

@OptIn(ExperimentalResourceApi::class)
@Composable
internal actual fun EmojiArtwork(emoji: EmojiToken, modifier: Modifier) {
    val allowed = !LocalMotion.current.reduced && LocalAppearance.current.messageEffects && ValueAnimator.areAnimatorsEnabled()
    val composition by produceState<LottieComposition?>(null, emoji.key) {
        value = withContext(Dispatchers.IO) {
            Res.readBytes("files/emoji/${emoji.key}.json").inputStream().use { LottieCompositionFactory.fromJsonInputStreamSync(it, "noto:${emoji.key}").value }
        }
    }
    val animation = composition
    if (animation == null) StaticEmoji(emoji, modifier)
    else AndroidView(factory = { context -> LottieAnimationView(context) }, modifier = modifier.testTag("animated-emoji:${emoji.key}").semantics { contentDescription = emoji.text },
        onReset = null, onRelease = { it.cancelAnimation() }, update = { view ->
            val changed = view.composition !== animation
            if (changed) {
                view.setComposition(animation); view.repeatCount = 1
            }
            if (!allowed) { view.cancelAnimation(); view.progress = 0f }
            else if (changed) view.playAnimation()
        })
}
