package org.sigil

import androidx.compose.animation.*
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsFocusedAsState
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.isSecondaryPressed
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.FirstBaseline
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.*
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlin.math.roundToInt

private const val PollRows=5
// Rows reach past the text edge so the press fill has room while radio and title share one start edge.
private val PollBleed=8.dp

internal fun pollFoot(part:MessagePart):String {
    val voters=part.voters
    fun count(n:Long)=if(part.multiple)"$n ${if(n==1L)"voter" else "voters"}" else "$n ${if(n==1L)"vote" else "votes"}"
    val status=when {
        part.closed->listOfNotNull("Voting closed",voters?.let(::count)).joinToString(" · ")
        voters==null->"Results after you vote"
        voters==0L->"No votes yet"
        else->count(voters)
    }
    return if(part.multiple && !part.closed)"Multiple choice · $status" else status
}

@Composable internal fun PollCard(message:ChatMessage,part:MessagePart,analyze:(String)->String,command:Command?) {
    var expanded by remember(part.id){mutableStateOf(false)}
    val motion=LocalMotion.current
    val ink=LocalContentColor.current
    val voters=part.voters
    val results=voters!=null && voters>0L
    val leading=if(part.closed && results)part.items.mapNotNull {it.count}.maxOrNull()?.takeIf {it>0L} else null
    // A vote shows at the tap and stands until the stored poll agrees, or for a few seconds if it never does.
    var pending by remember(part.id){mutableStateOf<Set<String>?>(null)}
    LaunchedEffect(part.items) {pending?.let {chosen->if(part.items.all {(it.id in chosen)==it.checked})pending=null}}
    LaunchedEffect(pending) {if(pending!=null) {delay(8000);pending=null}}
    val shown=part.items.map {item->pending?.let {item.copy(checked=item.id in it)} ?: item}
    fun act(item:CardItem) {
        val choices=if(part.multiple)shown.filter {if(it.id==item.id)!it.checked else it.checked}.map {it.id} else if(item.checked)emptyList() else listOf(item.id)
        pending=choices.toSet()
        command?.invoke("card_action",mapOf("peer" to message.peer,"author" to message.author,"message" to message.id,"card" to part.id,"choices" to choices))
    }
    val title=MaterialTheme.typography.titleMedium
    val titleLine=with(LocalDensity.current){title.lineHeight.toDp()}
    val press=LocalMaterialPress.current
    // A poll is not text: a right-click opens the message menu, where End poll lives, not the browser's text menu.
    Column(Modifier.widthIn(min=MessageCardMinWidth,max=MessageCardMaxWidth).fillMaxWidth().padding(vertical=4.dp).animateContentSize(motion.tween(MotionMillis))
        .pointerInput(press) {awaitPointerEventScope {while(true) {val event=awaitPointerEvent();if(press!=null && event.type==PointerEventType.Press && event.buttons.isSecondaryPressed) {event.changes.forEach {it.consume()};press()}}}}) {
        Box(Modifier.clearAndSetSemantics {contentDescription="Poll. ${part.text}";heading()}) {
            if(part.rich!=null)RichMessageText(part.rich,Modifier.heightIn(max=titleLine*3).clipToBounds(),title)
            else Text(part.text,style=MaterialTheme.typography.titleMedium,maxLines=3,overflow=TextOverflow.Ellipsis)
        }
        // Rows carry 8dp of their own padding; these top it up to the 12dp title gap.
        Spacer(Modifier.height(4.dp))
        if(part.items.isEmpty())Text("No options yet.",Modifier.padding(vertical=8.dp),style=MaterialTheme.typography.bodyMedium,color=ink.copy(alpha=.68f))
        (if(expanded)shown else shown.take(PollRows)).forEach {item->key(item.id) {
            val votes=item.count?:0L
            val share=if(results)(votes.toFloat()/voters!!).coerceIn(0f,1f)else 0f
            PollRow(item,part.multiple,results,share,votes,voters,leading!=null && item.count==leading,expanded,command!=null && item.enabled,press) {act(item)}
        }}
        // Chevron sits in the radio column and the label on the option text edge.
        if(part.items.size>PollRows)SigilTextButton({expanded=!expanded},Modifier.offset(x=-PollBleed),contentPadding=PaddingValues(horizontal=PollBleed)) {
            Box(Modifier.size(24.dp),contentAlignment=Alignment.Center) {Glyph(if(expanded)"expand_less" else "expand_more",20)}
            Spacer(Modifier.width(12.dp));Text(if(expanded)"Show fewer" else "Show all ${part.items.size}")
        }
        Text(pollFoot(part),style=MaterialTheme.typography.labelMedium.copy(fontFeatureSettings="tnum, lnum"),color=ink.copy(alpha=.68f))
    }
}

@Composable private fun PollRow(item:CardItem,multiple:Boolean,results:Boolean,share:Float,votes:Long,voters:Long?,won:Boolean,expanded:Boolean,enabled:Boolean,hold:(()->Unit)?,act:()->Unit) {
    val motion=LocalMotion.current
    val ink=LocalContentColor.current
    val body=MaterialTheme.typography.bodyMedium
    val line=with(LocalDensity.current){body.lineHeight.toDp()}
    val markLift=with(LocalDensity.current){(body.fontSize*.3f).toPx().roundToInt()}
    val interaction=remember {MutableInteractionSource()}
    val pressed by interaction.collectIsPressedAsState()
    val hovered by interaction.collectIsHoveredAsState()
    val focused by interaction.collectIsFocusedAsState()
    val fill by animateFloatAsState(if(!enabled)0f else if(focused && !pressed).10f else if(pressed || hovered).07f else 0f,motion.tween(MotionFeedback),label="Poll row")
    // Revealed results grow from zero; results already there on first sight are simply drawn.
    val amount by animateFloatAsState(if(results)share else 0f,motion.tween(MotionSettle),label="Poll result")
    val percent=(share*100).roundToInt()
    // A row takes the press, so its long press opens the message menu as the bubble's would.
    val choose=Modifier.combinedClickable(interaction,null,enabled,role=if(multiple)Role.Checkbox else Role.RadioButton,onLongClick=hold,onClick=act)
        .semantics {if(multiple)toggleableState=ToggleableState(item.checked) else selected=item.checked}
    Box(Modifier.bleed(PollBleed).fillMaxWidth().heightIn(min=48.dp).clip(RoundedCornerShape(14.dp)).background(ink.copy(alpha=fill))
        .then(choose).semantics {if(results)stateDescription="$percent%, $votes of $voters ${if(multiple)"voters" else "votes"}"+if(won)", most votes" else ""}
        .padding(horizontal=PollBleed,vertical=8.dp),contentAlignment=Alignment.CenterStart) {
        Column {
            Row(horizontalArrangement=Arrangement.spacedBy(12.dp),verticalAlignment=Alignment.Top) {
                // The mark centres on the first line's letters (.3em above its baseline), whatever the font, wrap or scale.
                Box(Modifier.size(24.dp).alignBy {it.measuredHeight/2+markLift},contentAlignment=Alignment.Center) {
                    AnimatedContent(item.checked,transitionSpec={(fadeIn(motion.enter(MotionInline))+scaleIn(motion.enter(MotionInline),initialScale=.8f)) togetherWith fadeOut(motion.exit(MotionExit))},label="Poll choice") {checked->
                        CompositionLocalProvider(LocalContentColor provides if(checked)ink else ink.copy(alpha=.68f)) {
                            Glyph(if(multiple)if(checked)"check_box" else "check_box_outline_blank" else if(checked)"radio_button_checked" else "radio_button_unchecked",24,filled=checked)
                        }
                    }
                }
                Box(Modifier.weight(1f).alignBy(FirstBaseline)) {
                    if(item.rich!=null)RichMessageText(item.rich,Modifier.heightIn(max=if(expanded)Dp.Unspecified else line*3).clipToBounds(),body)
                    else Text(item.text,style=body,maxLines=if(expanded)Int.MAX_VALUE else 3,overflow=TextOverflow.Ellipsis)
                }
                if(results)Text("${(amount*100).roundToInt()}%",Modifier.alignBy(FirstBaseline).clearAndSetSemantics {},style=MaterialTheme.typography.labelMedium.copy(fontFeatureSettings="tnum, lnum"),
                    color=if(item.checked)ink else ink.copy(alpha=.68f),maxLines=1)
            }
            if(results)Box(Modifier.padding(start=36.dp,top=6.dp).fillMaxWidth().height(4.dp).clip(RoundedCornerShape(2.dp)).background(ink.copy(alpha=.12f))) {
                if(amount>0f)Box(Modifier.fillMaxHeight().fillMaxWidth(amount).clip(RoundedCornerShape(2.dp)).background(ink.copy(alpha=if(item.checked)1f else .4f)))
            }
            if(won)Text("Most votes",Modifier.padding(start=36.dp,top=4.dp).clearAndSetSemantics {},style=MaterialTheme.typography.labelMedium,color=ink.copy(alpha=.68f))
        }
    }
}

// Widens a row by `by` on both sides without moving its neighbours, so a fill can run past the shared text edge.
private fun Modifier.bleed(by:Dp)=layout {measurable,constraints->
    val extra=by.roundToPx()*2
    val widened=if(constraints.hasBoundedWidth)constraints.copy(minWidth=constraints.minWidth+extra,maxWidth=constraints.maxWidth+extra) else constraints
    val placeable=measurable.measure(widened)
    layout(maxOf(0,placeable.width-extra).coerceIn(constraints.minWidth,if(constraints.hasBoundedWidth)constraints.maxWidth else Constraints.Infinity),placeable.height) {placeable.place(-extra/2,0)}
}
