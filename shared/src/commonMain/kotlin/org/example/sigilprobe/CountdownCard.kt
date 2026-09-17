package org.sigil

import androidx.compose.animation.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.unit.dp

@Composable internal fun CountdownCard(part:MessagePart,analyze:(String)->String) {
    val motion=LocalMotion.current
    val now=temporalNow(part.at)
    val reached=part.at in 1..now
    TemporalFrame("calendar_month","Countdown",part,analyze) {
        if(part.at>0)AnimatedContent(reached,transitionSpec={(fadeIn(motion.enter(MotionMillis))+scaleIn(motion.enter(MotionMillis),initialScale=.96f)) togetherWith fadeOut(motion.exit(MotionExit)) using SizeTransform(false) {_,_->motion.tween(MotionMillis)}},label="Countdown state") {passed->
            Column(verticalArrangement=Arrangement.spacedBy(4.dp)) {
                temporalScale(if(passed)now-part.at else part.at-now).let {(count,unit)->TemporalFigure("$count",if(passed)"$unit ago" else "$unit to go")}
                TemporalCaption(temporalBreakdown(if(passed)part.at else now,if(passed)now else part.at))
                TemporalCaption(if(part.date.isEmpty())"" else if(passed)"Reached ${part.date}" else "Until ${part.date}")
            }
        }
        else Text("No target time set.",style=MaterialTheme.typography.bodyMedium)
    }
}
