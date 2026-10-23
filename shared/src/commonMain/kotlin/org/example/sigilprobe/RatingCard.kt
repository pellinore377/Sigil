package org.sigil

import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsFocusedAsState
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.isSecondaryPressed
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.*
import androidx.compose.ui.graphics.drawscope.clipRect
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.isSpecified
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp

// Stars fill one after another over at most five stagger steps, after the bubble's rise.
internal const val RatingMotionMillis = MotionStagger * 5 + MotionInline

// "4/5" or "3.5/5"; a scale past ten stars falls back to five scaled by the ratio.
internal data class RatingScale(val value: String, val max: String, val stars: Int, val filled: Float)

internal fun ratingScale(display: String, ratio: Float?): RatingScale {
    val share = (ratio ?: 0f).coerceIn(0f, 1f)
    val parts = display.split('/').map { it.trim() }
    val max = parts.getOrNull(1)?.toFloatOrNull()
    val whole = max != null && max == kotlin.math.floor(max) && max in 1f..10f
    val stars = if (whole) max!!.toInt() else 5
    // Halves are the finest step a star shows; anything finer lives in the figure.
    val filled = kotlin.math.round(share * stars * 2) / 2
    return RatingScale(parts.getOrElse(0) { display }, parts.getOrElse(1) { "" }, stars, filled)
}

// Speaks "stars" only when the stars drawn match the scale, so 70/100 is not read as 100 stars.
internal fun ratingSpoken(display: String, scale: RatingScale): String = when {
    scale.max.isEmpty() -> display
    scale.max.toFloatOrNull() == scale.stars.toFloat() -> "${scale.value} out of ${scale.max} stars"
    else -> "${scale.value} out of ${scale.max}"
}

@Composable
internal fun RatingCard(value: UtilityContent) {
    val scale = ratingScale(value.display, value.ratio)
    val ink = LocalContentColor.current
    val motion = LocalMotion.current
    val playback = LocalTextMotion.current?.clock.takeIf { !motion.reduced && LocalAppearance.current.messageEffects }
    val elapsed = playback?.elapsed ?: TextMotionCap.toFloat()
    val glyph = if (scale.stars > 5) 20 else 24
    val lit = kotlin.math.ceil(scale.filled).toInt().coerceAtLeast(1)
    val step = MotionStagger * minOf(lit, 5) / lit.toFloat()
    val spoken = ratingSpoken(value.display, scale)
    Column(Modifier.widthIn(min = MessageCardMinWidth, max = MessageCardMaxWidth).padding(vertical = 4.dp)
        .clearAndSetSemantics { contentDescription = "Rating. $spoken" }, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(if (scale.stars > 5) 2.dp else 4.dp), verticalAlignment = Alignment.CenterVertically) {
            repeat(scale.stars) { star ->
                val amount = (scale.filled - star).coerceIn(0f, 1f)
                val shown = if (amount <= 0f) 0f else MotionStandardEasing.transform(((elapsed - MotionStagger - star * step) / MotionInline).coerceIn(0f, 1f))
                Box(contentAlignment = Alignment.Center) {
                    CompositionLocalProvider(LocalContentColor provides ink.copy(alpha = .56f)) { Glyph("star", glyph) }
                    // A half star is the full star clipped at its middle, from the start edge.
                    if (shown > 0f) Box(Modifier.graphicsLayer { alpha = shown; scaleX = .8f + .2f * shown; scaleY = scaleX }.drawWithContent {
                        val w = size.width * amount
                        if (layoutDirection == LayoutDirection.Rtl) clipRect(left = size.width - w) { this@drawWithContent.drawContent() }
                        else clipRect(right = w) { this@drawWithContent.drawContent() }
                    }) { Glyph("star", glyph, filled = true) }
                }
            }
        }
        Text(value.display, style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum, lnum"), color = ink.copy(alpha = .68f), maxLines = 1)
    }
}


// Average to one place, without a trailing ".0".
internal fun ratingMean(part:MessagePart):Double? {
    val voters=part.voters?.takeIf {it>0L} ?: return null
    return part.items.withIndex().sumOf {(i,item)->(item.count?:0L)*(i+1)}.toDouble()/voters
}
internal fun ratingAverage(part:MessagePart):String? {
    val tenths=kotlin.math.round((ratingMean(part) ?: return null)*10).toLong()
    return if(tenths%10==0L)"${tenths/10}" else "${tenths/10}.${tenths%10}"
}

internal fun ratingFoot(part:MessagePart,own:Int):String {
    val count=part.voters?.let {"$it ${if(it==1L)"rating" else "ratings"}"}
    val result=ratingAverage(part)?.let {"$it average · $count"}
    val mine="You rated $own".takeIf {own>0}
    return when {
        // Unknown counts (pages pending or hidden) are left out rather than read as none.
        part.closed->listOfNotNull("Rating closed",mine,result ?: "No ratings".takeIf {part.voters==0L})
        result!=null->listOfNotNull(mine,result)
        mine!=null->listOf(mine)
        part.voters==null->listOf("Results after you rate")
        else->listOf("Tap to rate")
    }.joinToString(" · ")
}

// A closed request with a known average shows that average on the scale instead of the controls.
internal fun ratingClosedSpoken(part:MessagePart):String? =
    ratingAverage(part)?.takeIf {part.closed}?.let {"Rating closed, $it out of ${part.scale} average"}

// A rating request is a single-choice poll over the scores 1..scale; a tap sends that score as the vote.
@Composable internal fun RatingRequestCard(message:ChatMessage,part:MessagePart,command:Command?) {
    val motion=LocalMotion.current
    val ink=LocalContentColor.current
    val stored=part.items.indexOfFirst {it.checked}+1
    // A score shows at the tap and stands until the stored rating agrees, or for a few seconds if it never does.
    var pending by remember(part.id){mutableStateOf<Int?>(null)}
    LaunchedEffect(stored) {if(pending==stored)pending=null}
    LaunchedEffect(pending) {if(pending!=null) {kotlinx.coroutines.delay(8000);pending=null}}
    val own=pending ?: stored
    val enabled=command!=null && !part.closed && part.items.all {it.enabled}
    fun rate(score:Int) {
        if(score==own)return
        pending=score
        command?.invoke("card_action",mapOf("peer" to message.peer,"author" to message.author,"message" to message.id,"card" to part.id,"choices" to listOf(part.items[score-1].id)))
    }
    val title=MaterialTheme.typography.titleMedium
    val press=LocalMaterialPress.current
    val stars=part.scale<=5
    val final=ratingClosedSpoken(part)
    val average=ratingMean(part)?.toFloat()
    // Like a poll, a right-click opens the message menu, where End rating lives.
    Column((if(stars)Modifier.widthIn(min=MessageCardMinWidth,max=MessageCardMaxWidth) else Modifier.widthIn(min=MessageCardMinWidth,max=MessageCardMaxWidth).fillMaxWidth())
        .padding(vertical=4.dp).animateContentSize(motion.tween(MotionMillis))
        .pointerInput(press) {awaitPointerEventScope {while(true) {val event=awaitPointerEvent();if(press!=null && event.type==PointerEventType.Press && event.buttons.isSecondaryPressed) {event.changes.forEach {it.consume()};press()}}}}) {
        // The question is what is being rated, so it is never cut short.
        Box(Modifier.clearAndSetSemantics {contentDescription="Rating. ${part.text}";heading()}) {
            if(part.rich!=null)RichMessageText(part.rich,Modifier,title) else Text(part.text,style=title)
        }
        Spacer(Modifier.height(if(stars)4.dp else 12.dp))
        if(final!=null && average!=null)Box(Modifier.clearAndSetSemantics {contentDescription=final}) {
            if(stars)RatingStars(part.scale,0,false,null,{},average) else RatingScaleRow(part.scale,0,false,null,{},average)
        }
        else if(stars)RatingStars(part.scale,own,enabled,press,::rate) else RatingScaleRow(part.scale,own,enabled,press,::rate)
        Spacer(Modifier.height(if(stars)4.dp else 8.dp))
        Text(ratingFoot(part,own),style=MaterialTheme.typography.labelMedium.copy(fontFeatureSettings="tnum, lnum"),color=ink.copy(alpha=.68f))
    }
}

// Five 48dp star targets; the first star's glyph sits on the card's start edge. A result fills to the average in half stars.
@Composable private fun RatingStars(scale:Int,own:Int,enabled:Boolean,hold:(()->Unit)?,rate:(Int)->Unit,result:Float?=null) {
    val ink=LocalContentColor.current
    val motion=LocalMotion.current
    var hovered by remember {mutableStateOf(0)}
    val shown=if(enabled && hovered>0)hovered else own
    val half=result?.let {kotlin.math.round(it*2)/2}
    Row(Modifier.offset(x=(-8).dp).selectableGroup()) {
        for(score in 1..scale) {
            val interaction=remember {MutableInteractionSource()}
            val hover by interaction.collectIsHoveredAsState()
            val pressed by interaction.collectIsPressedAsState()
            val focused by interaction.collectIsFocusedAsState()
            LaunchedEffect(hover) {if(hover)hovered=score else if(hovered==score)hovered=0}
            val amount=half?.let {(it-score+1).coerceIn(0f,1f)} ?: if(score<=shown)1f else 0f
            val grow by animateFloatAsState(if(pressed)1.12f else 1f,motion.tween(MotionFeedback),label="Star press")
            val fill by animateFloatAsState(if(enabled && focused && !pressed).10f else 0f,motion.tween(MotionFeedback),label="Star focus")
            val target=Modifier.size(48.dp).clip(RoundedCornerShape(14.dp)).background(ink.copy(alpha=fill))
            Box((if(result!=null)target else target.combinedClickable(interaction,null,enabled,role=Role.RadioButton,onLongClick=hold) {rate(score)}
                .clearAndSetSemantics {contentDescription="$score out of $scale ${if(score==1)"star" else "stars"}";selected=score==own;role=Role.RadioButton
                    if(enabled)onClick {rate(score);true} else disabled()}),contentAlignment=Alignment.Center) {
                // A lit star fades in and grows from .8 over the empty one.
                val lit by animateFloatAsState(if(amount>0f)1f else 0f,motion.tween(MotionInline),label="Star")
                Box(Modifier.graphicsLayer {scaleX=grow;scaleY=grow}) {
                    CompositionLocalProvider(LocalContentColor provides ink.copy(alpha=.56f)) {Glyph("star",32)}
                    if(lit>0f)Box(Modifier.graphicsLayer {alpha=lit;scaleX=.8f+.2f*lit;scaleY=scaleX}.drawWithContent {
                        val w=size.width*amount
                        if(layoutDirection==LayoutDirection.Rtl)clipRect(left=size.width-w) {this@drawWithContent.drawContent()}
                        else clipRect(right=w) {this@drawWithContent.drawContent()}
                    }) {CompositionLocalProvider(LocalContentColor provides ink) {Glyph("star",32,filled=true)}}
                }
            }
        }
    }
}

// Numbered 48dp cells: one row of ten when it fits, else two rows of five. A result fills the cells up to the rounded average.
@Composable private fun RatingScaleRow(scale:Int,own:Int,enabled:Boolean,hold:(()->Unit)?,rate:(Int)->Unit,result:Float?=null) {
    val ink=LocalContentColor.current
    val ground=LocalMessageSurface.current.takeIf {it.isSpecified} ?: MaterialTheme.colorScheme.surface
    val motion=LocalMotion.current
    val upTo=result?.let {kotlin.math.round(it).toInt()} ?: 0
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        val across=if(maxWidth>=48.dp*scale+4.dp*(scale-1))scale else (scale+1)/2
        Column(Modifier.selectableGroup(),verticalArrangement=Arrangement.spacedBy(4.dp)) {
            for(row in (1..scale).chunked(across)) Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.spacedBy(4.dp)) {
                for(score in row) {
                    val interaction=remember {MutableInteractionSource()}
                    val hover by interaction.collectIsHoveredAsState()
                    val pressed by interaction.collectIsPressedAsState()
                    val focused by interaction.collectIsFocusedAsState()
                    val chosen=if(result!=null)score<=upTo else score==own
                    val tone by animateFloatAsState(when {chosen->1f;!enabled->.06f;pressed->.16f;focused||hover->.14f;else->.08f},motion.tween(MotionFeedback),label="Score cell")
                    val cell=Modifier.weight(1f).heightIn(min=48.dp).clip(RoundedCornerShape(10.dp)).background(ink.copy(alpha=tone))
                    Box(if(result!=null)cell else cell.combinedClickable(interaction,null,enabled,role=Role.RadioButton,onLongClick=hold) {rate(score)}
                        .clearAndSetSemantics {contentDescription="$score out of $scale";selected=chosen;role=Role.RadioButton
                            if(enabled)onClick {rate(score);true} else disabled()},contentAlignment=Alignment.Center) {
                        Text("$score",style=MaterialTheme.typography.bodyMedium.copy(fontFeatureSettings="tnum, lnum"),color=if(chosen)ground else ink.copy(alpha=if(enabled).86f else .68f),maxLines=1)
                    }
                }
            }
        }
    }
}
