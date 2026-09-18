package org.sigil

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.takeOrElse
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.TextMeasurer
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch

private val FlapRadius=6.dp
private val CardRadius=18.dp
// The bubble clips at its own padding, so the reference's scale(1.15,1.3) becomes the widest gutter that fits, in its ratio.
private val CueSpreadX=6.dp
private val CueSpreadY=8.dp
private const val EndedAlpha=.62f
private data class FlapPlan(val style:TextStyle,val width:Dp,val height:Dp,val gap:Dp,val colon:Dp)
// Skipped seconds after backgrounding land without a flip; the clock is reconciled, not replayed.
private class TimerTick(private var last:Long) {fun step(value:Long):Boolean {val regular=last-value==1L;last=value;return regular}}

// Reference keyframes: rotate 0,-19,16,-10,7,0 across the shake.
private val BellAngles=floatArrayOf(0f,-19f,16f,-10f,7f,0f)
private fun bellAngle(t:Float):Float {
    val span=(t.coerceIn(0f,1f))*(BellAngles.size-1)
    val step=span.toInt().coerceAtMost(BellAngles.size-2)
    return BellAngles[step]+(BellAngles[step+1]-BellAngles[step])*(span-step)
}

@Composable internal fun TimerCard(part:MessagePart) {
    val motion=LocalMotion.current
    val now=temporalNow(part.at,Long.MAX_VALUE,settles=true)
    val ended=part.at in 1..now
    val remaining=(part.at-now).coerceAtLeast(0)
    val total=(part.at-part.startedAt).coerceAtLeast(1)
    val swept by animateFloatAsState(if(part.startedAt<=0)0f else (remaining.toFloat()/total).coerceIn(0f,1f),motion.tween(MotionMillis),label="Timer progress")
    val regular=remember(part.id){TimerTick(remaining)}.step(remaining)
    // One cue on expiry, and only for a timer that was seen running.
    val ran=remember(part.id) {!ended}
    val cue=remember(part.id) {Animatable(1f)}
    val bell=remember(part.id) {Animatable(1f)}
    // The digits hold full strength through the cue, then settle to the ended face; a timer already ended starts settled.
    val settled=remember(part.id) {Animatable(if(ended)EndedAlpha else 1f)}
    LaunchedEffect(ended) {
        if(!ended)return@LaunchedEffect
        if(ran && !motion.reduced) {
            cue.snapTo(0f);bell.snapTo(0f)
            launch {bell.animateTo(1f,motion.tween(MotionBell,MotionBellDelay,MotionInOutEasing))}
            cue.animateTo(1f,motion.tween(MotionCue,easing=LinearEasing))
            settled.animateTo(EndedAlpha,motion.tween(MotionSettle))
        }
        else settled.snapTo(EndedAlpha)
    }
    val ink=LocalContentColor.current
    // The flap sets the card's width, as in the reference; nothing is stretched to a card minimum.
    // The cue is the reference's endCue around the whole card, drawn ahead of animateContentSize, which clips to its bounds.
    Column(Modifier.width(IntrinsicSize.Min)
        .drawBehind {
            if(cue.value>=1f)return@drawBehind
            val elapsed=cue.value*MotionCue
            val radius=CardRadius.toPx()
            // The reference's box-shadow pulse: a band spreading outward from the edge, brightest at 0.38 of the cue.
            val halo=(elapsed/MotionCue/.8f).coerceIn(0f,1f)
            val peak=if(halo<=.475f)halo/.475f else (1f-halo)/.525f
            val band=CueSpreadY.toPx()*halo
            if(peak>0f && band>0f)drawCard(ink.copy(alpha=.13f*peak),band/2f,band/2f,radius,Stroke(band))
            for(index in 0..1) {
                val wave=MotionOutEasing.transform(((elapsed-index*MotionRingStagger)/MotionRing).coerceIn(0f,1f))
                if(wave<=0f || wave>=1f)continue
                drawCard(ink.copy(alpha=.45f*(1f-wave)),CueSpreadX.toPx()*wave,CueSpreadY.toPx()*wave,radius,Stroke(1.dp.toPx()))
            }
        }.animateContentSize(motion.tween(MotionMillis)),verticalArrangement=Arrangement.spacedBy(9.dp)) {
        if(part.at<=0) {Text("No timer set.",style=MaterialTheme.typography.bodyMedium);return@Column}
        TimerFlap(temporalDigits(remaining),regular,{settled.value},if(ended)"Timer ended" else "${temporalSpan(remaining)} remaining")
        Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(7.dp)) {
            Box(Modifier.graphicsLayer {rotationZ=if(bell.value>=1f)0f else bellAngle(bell.value);transformOrigin=TransformOrigin(.5f,.2f)}) {Glyph(if(ended)"notifications_active" else "schedule",14)}
            Text(if(ended)"Time's up" else "Remaining",style=MaterialTheme.typography.labelMedium,color=LocalContentColor.current.copy(alpha=.68f),maxLines=1,overflow=TextOverflow.Ellipsis)
        }
        if(!ended)Box(Modifier.fillMaxWidth().height(2.dp).clip(RoundedCornerShape(2.dp)).background(LocalContentColor.current.copy(alpha=.12f))) {
            Box(Modifier.fillMaxHeight().fillMaxWidth(swept).clip(RoundedCornerShape(2.dp)).background(LocalContentColor.current.copy(alpha=.5f)))
        }
    }
}

private fun DrawScope.drawCard(color:Color,growX:Float,growY:Float,radius:Float,stroke:Stroke) {
    drawRoundRect(color,Offset(-growX,-growY),Size(size.width+growX*2,size.height+growY*2),CornerRadius(radius+minOf(growX,growY)),style=stroke)
}

@Composable private fun TimerFlap(value:String,animate:Boolean,settled:()->Float,description:String) {
    val digits=value.count {it!=':'}
    val plan=flapPlan(digits,value.length-digits)
    val measurer=rememberTextMeasurer(cacheSize=12)
    val surface=LocalMessageSurface.current.takeOrElse {MaterialTheme.colorScheme.surface}
    val face=lerp(surface,LocalContentColor.current,.11f)
    val back=lerp(surface,LocalContentColor.current,.075f)
    val ink=LocalContentColor.current
    // Cell count changes only between regimes; a fresh row would otherwise flip from nothing.
    key(value.length) {
        Row(Modifier.clearAndSetSemantics {contentDescription=description}.graphicsLayer {alpha=settled()},
            horizontalArrangement=Arrangement.spacedBy(plan.gap),verticalAlignment=Alignment.CenterVertically) {
            value.forEachIndexed {index,character->key(index) {
                if(character==':')Box(Modifier.width(plan.colon).offset(y=-plan.height*.038f),contentAlignment=Alignment.Center) {
                    Text(":",style=plan.style.copy(fontSize=plan.style.fontSize*.62f),color=ink.copy(alpha=.45f),maxLines=1)
                }
                else FlapCell(character,animate,plan,measurer,face,back,ink)
            }}
        }
    }
}

// One retargetable rotation per cell. Progress is read in the draw phase only, so a turn never recomposes.
@Composable private fun FlapCell(digit:Char,animate:Boolean,plan:FlapPlan,measurer:TextMeasurer,face:Color,back:Color,ink:Color) {
    val motion=LocalMotion.current
    val progress=remember {Animatable(1f)}
    var faces by remember {mutableStateOf(digit to digit)}
    LaunchedEffect(digit) {
        if(faces.second==digit)return@LaunchedEffect
        faces=faces.second to digit
        if(!animate || motion.reduced) {progress.snapTo(1f);return@LaunchedEffect}
        // A settled cell turns from the top; one caught mid-turn keeps its angle and retargets.
        if(progress.value>=1f)progress.snapTo(0f)
        progress.animateTo(1f,motion.tween(MotionFlap,easing=MotionFlapEasing))
    }
    val turn={progress.value}
    val seam=lerp(face,Color.Black,.62f)
    Box(Modifier.size(plan.width,plan.height)) {
        FlapHalf(faces.second.toString(),plan,measurer,false,back,ink,Modifier.align(Alignment.BottomStart))
        FlapHalf(faces.first.toString(),plan,measurer,false,back,ink,Modifier.align(Alignment.BottomStart).graphicsLayer {alpha=if(turn()<1f)1f else 0f})
        FlapHalf(faces.second.toString(),plan,measurer,true,face,ink,Modifier.align(Alignment.TopStart))
        Box(Modifier.align(Alignment.Center).fillMaxWidth().height(1.dp).background(seam.copy(alpha=.8f)))
        // Camera distance is in pixels: the reference's perspective is a touch over five cell heights.
        Box(Modifier.align(Alignment.TopStart).size(plan.width,plan.height/2).graphicsLayer {
            rotationX=-180f*turn();cameraDistance=plan.height.toPx()*5.1f;transformOrigin=TransformOrigin(.5f,1f)
        }) {
            FlapHalf(faces.first.toString(),plan,measurer,true,face,ink,Modifier
                .graphicsLayer {alpha=if(turn()<.5f)1f else 0f}
                .drawWithContent {drawContent();drawRect(Color.Black,alpha=(turn()*2f).coerceAtMost(1f)*.48f)})
            FlapHalf(faces.second.toString(),plan,measurer,false,back,ink,Modifier
                .graphicsLayer {rotationX=180f;alpha=if(turn()<.5f)0f else 1f}
                .drawWithContent {drawContent();drawRect(Color.Black,alpha=(1f-((turn()-.42f)/.58f).coerceIn(0f,1f))*.44f)})
        }
    }
}

// The numeral is painted, not laid out: its baseline sits at the reference's 0.71 of the cell in both halves.
@Composable private fun FlapHalf(text:String,plan:FlapPlan,measurer:TextMeasurer,top:Boolean,fill:Color,ink:Color,modifier:Modifier=Modifier) {
    val shape=if(top)RoundedCornerShape(topStart=FlapRadius,topEnd=FlapRadius) else RoundedCornerShape(bottomStart=FlapRadius,bottomEnd=FlapRadius)
    val glyph=remember(text,plan.style,measurer) {measurer.measure(text,plan.style)}
    Canvas(modifier.size(plan.width,plan.height/2).clip(shape).background(fill)) {
        val cell=plan.height.toPx()
        drawText(glyph,ink,Offset((size.width-glyph.size.width)/2f,cell*.71f-glyph.firstBaseline-(if(top)0f else cell/2f)))
    }
}

// Cell geometry follows the reference's phone-width ratios (34x52 cell, 5 gap, 7 colon on a 37px face).
@Composable private fun flapPlan(digits:Int,colons:Int):FlapPlan {
    val density=LocalDensity.current
    val measurer=rememberTextMeasurer(cacheSize=4)
    val typography=MaterialTheme.typography
    return remember(digits,colons,typography,density) {
        var chosen:FlapPlan?=null
        for(role in listOf(typography.displaySmall,typography.headlineMedium,typography.headlineSmall)) {
            val size=role.fontSize
            val height=measurer.measure("0",role).size.height
            val plan=with(density) {
                val cell=size.toDp()*.919f
                FlapPlan(role,cell,maxOf(size.toDp()*1.405f,height.toDp()+4.dp),cell*.147f,cell*.206f)
            }
            chosen=plan
            if(plan.width*digits+plan.colon*colons+plan.gap*(digits+colons-1)<=MessageCardMaxWidth-16.dp)break
        }
        chosen!!
    }
}
