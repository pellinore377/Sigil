package org.sigil

import androidx.compose.animation.*
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.unit.dp

@Composable internal fun TimerCard(part:MessagePart) {
    val motion=LocalMotion.current
    val compact=LocalAppearance.current.compact
    val now=temporalNow(part.at,Long.MAX_VALUE,settles=true)
    val ended=part.at in 1..now
    val remaining=(part.at-now).coerceAtLeast(0)
    val total=(part.at-part.startedAt).coerceAtLeast(1)
    val settled by animateFloatAsState(if(ended).75f else 1f,motion.tween(MotionMillis),label="Timer completion")
    val swept by animateFloatAsState(if(part.startedAt<=0)0f else (1f-remaining.toFloat()/total).coerceIn(0f,1f),motion.tween(MotionMillis),label="Timer progress")
    CardFrame("timer","Timer") {
        if(part.at>0) {
            Row(Modifier.alpha(settled),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(12.dp)) {
                CircularProgressIndicator(progress={swept},modifier=Modifier.size(if(compact)40.dp else 44.dp),color=LocalContentColor.current,trackColor=LocalContentColor.current.copy(alpha=.16f))
                AnimatedContent(ended,transitionSpec={(fadeIn(motion.enter(MotionMillis))+scaleIn(motion.enter(MotionMillis),initialScale=.96f)) togetherWith fadeOut(motion.exit(MotionExit)) using SizeTransform(false) {_,_->motion.tween(MotionMillis)}},label="Timer state") {done->
                    if(done)TemporalFigure("0:00","finished") else temporalClock(remaining).let {(value,unit)->TemporalFigure(value,unit)}
                }
            }
            TemporalCaption(listOfNotNull(part.date.takeIf {it.isNotEmpty()}?.let {"${if(ended)"Ended" else "Ends"} $it"},temporalSpan(total).takeIf {part.startedAt>0}).joinToString(" · "))
        }
        else Text("No timer set.",style=MaterialTheme.typography.bodyMedium)
    }
}
