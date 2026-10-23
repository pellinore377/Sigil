@file:OptIn(kotlin.time.ExperimentalTime::class)
package org.sigil

import androidx.compose.animation.*
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsFocusedAsState
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.FirstBaseline
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.*
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlin.math.roundToInt

// The reference shows five rows, then Show all; the wire allows 256.
internal const val ChecklistRows=5
private val ChecklistBleed=8.dp
private val Weekdays=listOf("Monday","Tuesday","Wednesday","Thursday","Friday","Saturday","Sunday")

// ISO weekday (1 Monday .. 7 Sunday) of an instant in the viewer's zone, 0 when unknown.
internal expect fun viewerWeekday(at:Long):Int
internal expect fun viewerZone():String

// Viewer's clock, phrased by interval; the rule's own reset follows when its zone differs.
internal fun resetLabel(interval:String,stamp:String,weekday:Int,ruleTime:String,ruleZone:String,viewer:String):String {
    val day=stamp.substringBeforeLast(", ","")
    val shown=if(interval=="Weekly" && day.isNotEmpty() && day!="Today" && day!="Tomorrow" && weekday in 1..7)"${Weekdays[weekday-1]}, ${stamp.substringAfterLast(", ")}" else stamp
    val place=ruleZone.substringAfterLast('/').replace('_',' ')
    val rule=if(ruleZone.isNotBlank() && viewer.isNotBlank() && ruleZone!=viewer && ruleTime.isNotBlank())" · $ruleTime in $place" else ""
    return "$interval · resets $shown$rule"
}
internal fun checklistCount(items:List<CardItem>):String {
    val done=items.count {it.checked}
    return if(items.isNotEmpty() && done==items.size)"All done" else "$done/${items.size}"
}
internal fun taskDone(item:CardItem,name:String=""):String {
    if(item.doneAt<=0L)return ""
    val who=when {item.doneMine->"Completed by you";name.isNotBlank()->"Completed by $name";else->"Completed"}
    return listOf(who,item.done).filter {it.isNotBlank()}.joinToString(" · ")
}
// Undo belongs to the earliest completion only when it is ours, and only once the core has dated it.
internal fun taskUndo(item:CardItem,now:Long)=item.checked && item.enabled && item.doneMine && item.doneAt>0L && item.undoUntil>now
// After the reset instant: kept rows return unticked, one-off rows leave, until the core's next state arrives.
internal fun afterReset(items:List<CardItem>)=items.filter {it.persistent}.map {it.copy(checked=false,enabled=true)}
private fun clockSeconds()=kotlin.time.Clock.System.now().epochSeconds

// Three lines then an ellipsis; a tap shows the whole title when it overflows.
@Composable internal fun CardTitle(part:MessagePart,description:String,modifier:Modifier=Modifier,state:String?=null) {
    val style=MaterialTheme.typography.titleMedium
    var full by rememberSaveable(part.id) {mutableStateOf(false)}
    var width by remember {mutableIntStateOf(0)}
    val measurer=rememberTextMeasurer()
    val overflows=width>0 && remember(part.text,width,style) {measurer.measure(part.text,style,maxLines=3,constraints=Constraints(maxWidth=width)).hasVisualOverflow}
    val lines=if(full)Int.MAX_VALUE else 3
    Box(modifier.onSizeChanged {width=it.width}
        .then(if(overflows || full)Modifier.clickable(onClickLabel=if(full)"Show less" else "Show the full title") {full=!full} else Modifier)
        .clearAndSetSemantics {contentDescription=description;heading();if(state!=null)stateDescription=state;if(overflows || full)onClick(if(full)"Show less" else "Show the full title") {full=!full;true}}) {
        if(part.rich!=null && part.rich.text.isNotBlank())RichMessageText(part.rich,style=style,maxLines=lines)
        else if(part.text.isNotBlank())Text(part.text,style=style,maxLines=lines,overflow=TextOverflow.Ellipsis)
    }
}

@Composable internal fun ChecklistCard(message:ChatMessage,part:MessagePart,analyze:(String)->String,command:Command?) {
    val task=part.kind=="task"
    val recurring=part.kind=="recurring"
    val motion=LocalMotion.current
    val ink=LocalContentColor.current
    val sender=LocalMediaSender.current
    var expanded by rememberSaveable(part.id){mutableStateOf(false)}
    var confirming by remember(part.id){mutableStateOf<CardItem?>(null)}
    // A live recurring list turns over at its reset even while open; a preview is already current.
    val turns=recurring && part.at>0 && command!=null
    var reset by remember(part.id,part.at){mutableStateOf(turns && clockSeconds()>=part.at)}
    if(turns)LaunchedEffect(part.at) {delay((part.at*1000-kotlin.time.Clock.System.now().toEpochMilliseconds()).coerceAtLeast(0));reset=true}
    val delivered=if(reset)afterReset(part.items) else part.items
    // A tap shows at once; the delivered state takes over as soon as it agrees, or the override lapses if the action never lands.
    var pending by remember(part.id){mutableStateOf<Map<String,Boolean>>(emptyMap())}
    LaunchedEffect(delivered) {pending=pending.filterNot {(id,checked)->delivered.any {it.id==id && it.checked==checked}}}
    LaunchedEffect(pending) {if(pending.isNotEmpty()) {delay(8000);pending=emptyMap()}}
    val items=delivered.map {item->pending[item.id]?.let {item.copy(checked=it)} ?: item}
    // Undo leaves on its own deadline; the clock is read afresh whenever the items change.
    var now by remember(part.id,part.items){mutableLongStateOf(clockSeconds())}
    LaunchedEffect(part.items) {
        while(true) {
            now=clockSeconds()
            val next=part.items.map {it.undoUntil}.filter {it>now}.minOrNull() ?: break
            delay((next-now)*1000)
        }
    }
    fun act(item:CardItem,checked:Boolean) {
        pending=pending+(item.id to checked)
        command?.invoke("card_action",mapOf("peer" to message.peer,"author" to message.author,"message" to message.id,"card" to part.id,"item" to item.id,"checked" to checked))
    }
    val type=when {task->"Task list";recurring->"Recurring checklist";else->"Checklist"}
    val done=items.count {it.checked}
    Column(Modifier.widthIn(min=MessageCardMinWidth,max=MessageCardMaxWidth).fillMaxWidth().padding(vertical=4.dp).animateContentSize(motion.tween(MotionMillis))) {
        Row(horizontalArrangement=Arrangement.spacedBy(12.dp)) {
            CardTitle(part,listOf(type,part.text).filter {it.isNotBlank()}.joinToString(". "),Modifier.weight(1f).alignBy(FirstBaseline))
            if(items.isNotEmpty())Text(checklistCount(items),Modifier.alignBy(FirstBaseline).semantics {contentDescription="$done of ${items.size} done"},
                style=MaterialTheme.typography.labelMedium.copy(fontFeatureSettings="tnum, lnum"),color=ink.copy(alpha=.68f),maxLines=1)
        }
        if(items.isEmpty())Text("No items yet.",Modifier.padding(top=12.dp),style=MaterialTheme.typography.bodyMedium,color=ink.copy(alpha=.68f))
        else {
            val fraction by animateFloatAsState(done.toFloat()/items.size,motion.tween(MotionMillis),label="Checklist progress")
            Box(Modifier.padding(top=12.dp,bottom=4.dp).fillMaxWidth().height(4.dp).clip(RoundedCornerShape(2.dp)).background(ink.copy(alpha=.12f)).clearAndSetSemantics {}) {
                if(fraction>0f)Box(Modifier.fillMaxHeight().fillMaxWidth(fraction).clip(RoundedCornerShape(2.dp)).background(ink))
            }
        }
        (if(expanded)items else items.take(ChecklistRows)).forEach {item->key(item.id) {
            val undo=task && command!=null && taskUndo(item,now)
            val name=if(task && !item.doneMine && item.doneBy.isNotBlank())sender(message.copy(author=item.doneBy,mine=false)) else ""
            // Only the reset unticks a recurring row, and only Undo reverses a task.
            ChecklistRow(item,task,recurring && item.persistent,command!=null && item.enabled && !((task || recurring) && item.checked),if(task && item.checked)taskDone(item,name) else "",if(undo)({act(item,false)}) else null) {
                if(task)confirming=item else act(item,!item.checked)
            }
        }}
        if(items.size>ChecklistRows)SigilTextButton({expanded=!expanded},Modifier.offset(x=-ChecklistBleed),contentPadding=PaddingValues(horizontal=ChecklistBleed)) {
            Box(Modifier.size(24.dp),contentAlignment=Alignment.Center) {Glyph(if(expanded)"expand_less" else "expand_more",20)}
            Spacer(Modifier.width(12.dp));Text(if(expanded)"Show fewer" else "Show all ${items.size}")
        }
        val foot=when {task->"Confirm to complete · 30s undo";recurring && reset->part.date.replaceFirst(" · resets "," · reset ");recurring->part.date;else->""}
        if(foot.isNotBlank())Text(foot,Modifier.padding(top=4.dp),style=MaterialTheme.typography.labelMedium,color=ink.copy(alpha=.68f),maxLines=3,overflow=TextOverflow.Ellipsis)
    }
    confirming?.let {item->AlertDialog(onDismissRequest={confirming=null},title={Text("Complete this task?")},text={Text("“${item.text}”\n\nYou can undo for 30 seconds. After that it stays complete.")},
        confirmButton={SigilTextButton({confirming=null;items.firstOrNull {it.id==item.id && !it.checked && it.enabled}?.let {act(it,true)}}){Text("Complete")}},dismissButton={SigilTextButton({confirming=null}){Text("Cancel")}})}
}

@Composable private fun ChecklistRow(item:CardItem,task:Boolean,kept:Boolean,enabled:Boolean,detail:String,undo:(()->Unit)?,act:()->Unit) {
    val motion=LocalMotion.current
    val ink=LocalContentColor.current
    val quiet=ink.copy(alpha=.68f)
    val body=MaterialTheme.typography.bodyMedium
    val markLift=with(LocalDensity.current){(body.fontSize*.3f).toPx().roundToInt()}
    val interaction=remember {MutableInteractionSource()}
    val pressed by interaction.collectIsPressedAsState()
    val hovered by interaction.collectIsHoveredAsState()
    val focused by interaction.collectIsFocusedAsState()
    val fill by animateFloatAsState(if(!enabled)0f else if(focused && !pressed).10f else if(pressed || hovered).07f else 0f,motion.tween(MotionFeedback),label="Checklist row")
    Box(Modifier.widen(ChecklistBleed).fillMaxWidth().heightIn(min=48.dp).clip(RoundedCornerShape(14.dp)).background(ink.copy(alpha=fill))
        .combinedClickable(interaction,null,enabled,role=Role.Checkbox,onLongClick=LocalMaterialPress.current,onClick=act)
        .semantics {toggleableState=ToggleableState(item.checked)}
        .padding(horizontal=ChecklistBleed,vertical=8.dp),contentAlignment=Alignment.CenterStart) {
        Row(horizontalArrangement=Arrangement.spacedBy(12.dp),verticalAlignment=Alignment.Top) {
            // The mark centres on the first line's letters (.3em above its baseline), whatever the font, wrap or scale.
            Box(Modifier.size(24.dp).alignBy {it.measuredHeight/2+markLift},contentAlignment=Alignment.Center) {
                AnimatedContent(item.checked,transitionSpec={(fadeIn(motion.enter(MotionInline))+scaleIn(motion.enter(MotionInline),initialScale=.8f)) togetherWith fadeOut(motion.exit(MotionExit))},label="Check mark") {checked->
                    CompositionLocalProvider(LocalContentColor provides if(checked)ink else quiet) {
                        Glyph(if(task)if(checked)"check_circle" else "circle" else if(checked)"check_box" else "check_box_outline_blank",24,filled=checked)
                    }
                }
            }
            Column(Modifier.weight(1f).alignBy(FirstBaseline),verticalArrangement=Arrangement.spacedBy(4.dp)) {
                // Done rows recede but stay readable: quiet ink and a strike, never below 60%.
                CompositionLocalProvider(LocalContentColor provides if(item.checked)quiet else ink) {
                    val style=if(item.checked)body.copy(textDecoration=TextDecoration.LineThrough) else body
                    if(item.rich!=null)RichMessageText(item.rich,style=style) else Text(item.text,style=style)
                }
                if(detail.isNotEmpty())Text(detail,style=MaterialTheme.typography.labelMedium,color=quiet)
            }
            if(undo!=null)SigilTextButton(undo,Modifier.align(Alignment.CenterVertically).semantics {contentDescription="Undo completion"},contentPadding=PaddingValues(horizontal=12.dp)) {Text("Undo")}
            if(kept)Box(Modifier.alignBy {it.measuredHeight/2+markLift}) {CompositionLocalProvider(LocalContentColor provides quiet) {Glyph("autorenew",18,"Kept through resets")}}
        }
    }
}
