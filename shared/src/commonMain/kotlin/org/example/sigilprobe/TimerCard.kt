package org.sigil

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.takeOrElse
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

private data class FlapPlan(val style:TextStyle,val width:Dp,val height:Dp)
// Skipped seconds after backgrounding land without a flip; the clock is reconciled, not replayed.
private class TimerTick(private var last:Long) {fun step(value:Long):Boolean {val regular=last-value==1L;last=value;return regular}}

@Composable internal fun TimerCard(part:MessagePart) {
    val motion=LocalMotion.current
    val now=temporalNow(part.at,Long.MAX_VALUE,settles=true)
    val ended=part.at in 1..now
    val remaining=(part.at-now).coerceAtLeast(0)
    val total=(part.at-part.startedAt).coerceAtLeast(1)
    val settled by animateFloatAsState(if(ended).75f else 1f,motion.tween(MotionMillis),label="Timer completion")
    val swept by animateFloatAsState(if(part.startedAt<=0)0f else (1f-remaining.toFloat()/total).coerceIn(0f,1f),motion.tween(MotionMillis),label="Timer progress")
    val regular=remember(part.id){TimerTick(remaining)}.step(remaining)
    CardFrame(if(ended)"notifications_active" else "timer","Timer") {
        if(part.at>0) {
            Row(Modifier.alpha(settled),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(10.dp)) {
                TimerFlap(temporalDigits(remaining),regular && !ended,if(ended)"Timer ended" else "${temporalSpan(remaining)} remaining")
                Text(if(ended)"finished" else "remaining",style=MaterialTheme.typography.labelMedium,maxLines=1,overflow=TextOverflow.Ellipsis)
            }
            Box(Modifier.fillMaxWidth().height(4.dp).clip(RoundedCornerShape(6.dp)).background(LocalContentColor.current.copy(alpha=.16f))) {
                Box(Modifier.fillMaxHeight().fillMaxWidth(swept).clip(RoundedCornerShape(6.dp)).background(LocalContentColor.current))
            }
            TemporalCaption(listOfNotNull(part.date.takeIf {it.isNotEmpty()}?.let {"${if(ended)"Ended" else "Ends"} $it"},temporalSpan(total).takeIf {part.startedAt>0}).joinToString(" · "))
        }
        else Text("No timer set.",style=MaterialTheme.typography.bodyMedium)
    }
}

@Composable private fun TimerFlap(value:String,animate:Boolean,description:String) {
    val digits=value.count {it!=':'}
    val plan=flapPlan(value.length,digits,value.length-digits)
    val face=lerp(LocalMessageSurface.current.takeOrElse {MaterialTheme.colorScheme.surface},LocalContentColor.current,.10f)
    key(value.length) {
        Row(Modifier.clearAndSetSemantics {contentDescription=description},horizontalArrangement=Arrangement.spacedBy(4.dp),verticalAlignment=Alignment.CenterVertically) {
            value.forEachIndexed {index,character->key(index) {
                if(character==':')Text(":",style=plan.style,color=LocalContentColor.current.copy(alpha=.6f),maxLines=1)
                else FlapCell(character,animate,plan,face)
            }}
        }
    }
}

// One retargetable rotation per cell, never a new element per flip, so an interrupted digit stays continuous.
@Composable private fun FlapCell(digit:Char,animate:Boolean,plan:FlapPlan,face:Color) {
    val motion=LocalMotion.current
    val progress=remember {Animatable(1f)}
    var faces by remember {mutableStateOf(digit to digit)}
    LaunchedEffect(digit) {
        if(faces.second==digit)return@LaunchedEffect
        val settled=!progress.isRunning
        faces=faces.second to digit
        if(animate && settled && !motion.reduced) {progress.snapTo(0f);progress.animateTo(1f,motion.tween(MotionInline))}
        else progress.snapTo(1f)
    }
    val turn=progress.value
    val seam=LocalContentColor.current.copy(alpha=.18f)
    Box(Modifier.size(plan.width,plan.height).clip(RoundedCornerShape(12.dp))) {
        FlapHalf(faces.second.toString(),plan,true,face,Modifier.align(Alignment.TopStart))
        FlapHalf((if(turn<1f)faces.first else faces.second).toString(),plan,false,face,Modifier.align(Alignment.BottomStart))
        Box(Modifier.align(Alignment.Center).fillMaxWidth().height(1.dp).background(seam))
        if(turn<1f)Box(Modifier.align(Alignment.TopStart).graphicsLayer {rotationX=-180f*turn;cameraDistance=8f*density;transformOrigin=TransformOrigin(.5f,1f)}) {
            if(turn<.5f)FlapHalf(faces.first.toString(),plan,true,face)
            else Box(Modifier.graphicsLayer {rotationX=180f}) {FlapHalf(faces.second.toString(),plan,false,face)}
        }
    }
}

@Composable private fun FlapHalf(text:String,plan:FlapPlan,top:Boolean,face:Color,modifier:Modifier=Modifier) {
    Box(modifier.size(plan.width,plan.height/2).background(face).clipToBounds()) {
        Box(Modifier.size(plan.width,plan.height).offset(y=if(top)0.dp else -plan.height/2),contentAlignment=Alignment.Center) {
            Text(text,style=plan.style,maxLines=1)
        }
    }
}

// Cells are measured, never literal, so the flap survives textScale inside the card width cap.
@Composable private fun flapPlan(length:Int,digits:Int,colons:Int):FlapPlan {
    val density=LocalDensity.current
    val measurer=rememberTextMeasurer(cacheSize=8)
    val code=LocalCodeFont.current
    val typography=MaterialTheme.typography
    return remember(length,digits,colons,code,typography,density) {
        var chosen:FlapPlan?=null
        for(role in listOf(typography.titleLarge,typography.titleMedium,typography.bodyMedium)) {
            val style=role.copy(fontFamily=code)
            val digit=measurer.measure("0",style).size
            val colon=measurer.measure(":",style).size
            val plan=with(density) {FlapPlan(style,digit.width.toDp()+10.dp,digit.height.toDp()+8.dp)}
            chosen=plan
            if(plan.width*digits+with(density) {colon.width.toDp()}*colons+4.dp*(length-1)<=272.dp)break
        }
        chosen!!
    }
}
