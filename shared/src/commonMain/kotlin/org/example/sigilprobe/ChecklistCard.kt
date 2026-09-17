package org.sigil

import androidx.compose.animation.*
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.*
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

@Composable internal fun ChecklistCard(message:ChatMessage,part:MessagePart,analyze:(String)->String,command:Command?) {
    val task=part.kind=="task"
    var expanded by remember(part.id){mutableStateOf(false)}
    var confirming by remember(part.id){mutableStateOf<CardItem?>(null)}
    val motion=LocalMotion.current
    val compact=LocalAppearance.current.compact
    val done=part.items.count {it.checked}
    val fraction by animateFloatAsState(if(part.items.isEmpty())0f else done.toFloat()/part.items.size,motion.tween(MotionMillis),label="Checklist progress")
    val lines=with(LocalDensity.current){MaterialTheme.typography.bodyMedium.lineHeight.toDp()*3}
    fun act(item:CardItem) {command?.invoke("card_action",mapOf("peer" to message.peer,"author" to message.author,"message" to message.id,"card" to part.id,"item" to item.id,"checked" to !item.checked))}
    CardFrame(if(task)"assignment" else "checklist",if(task)"Task" else "Checklist") {
        if(part.rich!=null)RichMessageText(part.rich,style=MaterialTheme.typography.titleMedium)else MessageText(part.text,analyze)
        if(part.items.isNotEmpty()) {
            Text(if(done==part.items.size)"All ${part.items.size} complete"else"$done of ${part.items.size} complete",style=MaterialTheme.typography.labelMedium)
            Box(Modifier.fillMaxWidth().height(4.dp).clip(RoundedCornerShape(6.dp)).background(LocalContentColor.current.copy(alpha=.16f))) {
                Box(Modifier.fillMaxHeight().fillMaxWidth(fraction).clip(RoundedCornerShape(6.dp)).background(LocalContentColor.current))
            }
        }
        Column(verticalArrangement=Arrangement.spacedBy(if(compact)4.dp else 6.dp)) {
            (if(expanded)part.items else part.items.take(5)).forEach {item->key(item.id) {
                val tint by animateColorAsState(LocalContentColor.current.copy(alpha=if(item.checked).13f else .04f),motion.tween(MotionInline),label="Checklist item")
                val undoable=task && item.checked && item.enabled && command!=null
                Box(Modifier.fillMaxWidth().heightIn(min=44.dp).clip(RoundedCornerShape(12.dp)).background(tint)
                    .clickable(enabled=command!=null && item.enabled,role=Role.Checkbox) {if(task && !item.checked)confirming=item else act(item)}
                    .semantics {toggleableState=ToggleableState(item.checked);if(task && item.checked)stateDescription=if(undoable)"Completed, undo available"else"Completed"}) {
                    Row(Modifier.padding(horizontal=10.dp,vertical=if(compact)8.dp else 10.dp),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                        AnimatedContent(item.checked,transitionSpec={(fadeIn(motion.enter(MotionInline))+scaleIn(motion.enter(MotionInline),initialScale=.8f)) togetherWith fadeOut(motion.exit(MotionExit))},label="Checklist mark") {checked->
                            Glyph(if(checked)"check_box"else"check_box_outline_blank",20)
                        }
                        Box(Modifier.weight(1f).alpha(if(item.checked).65f else 1f)) {
                            if(item.rich!=null)RichMessageText(item.rich,Modifier.heightIn(max=if(expanded)Dp.Unspecified else lines).clipToBounds(),MaterialTheme.typography.bodyMedium)
                            else Text(item.text,style=MaterialTheme.typography.bodyMedium,maxLines=if(expanded)Int.MAX_VALUE else 3,overflow=TextOverflow.Ellipsis)
                        }
                        if(undoable)Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {Glyph("undo",18);Text("Undo",style=MaterialTheme.typography.labelSmall)}
                        else if(task && item.checked)Glyph("lock",14)
                    }
                }
            }}
        }
        if(part.items.size>5)SigilTextButton({expanded=!expanded}) {Glyph(if(expanded)"expand_less"else"expand_more",18);Spacer(Modifier.width(8.dp));Text(if(expanded)"Show less"else"Show all ${part.items.size}")}
    }
    confirming?.let {item->AlertDialog(onDismissRequest={confirming=null},title={Text("Complete this task?")},text={Text("You can undo your completion for 30 seconds.")},confirmButton={SigilTextButton({confirming=null;part.items.firstOrNull {it.id==item.id && !it.checked && it.enabled}?.let(::act)}){Text("Complete")}},dismissButton={SigilTextButton({confirming=null}){Text("Cancel")}})}
}
