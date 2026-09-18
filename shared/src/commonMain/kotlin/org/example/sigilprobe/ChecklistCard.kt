package org.sigil

import androidx.compose.animation.*
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.takeOrElse
import androidx.compose.ui.semantics.*
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

// The reference caps the rendered list at eight rows (cards.js buildChecklist); the wire allows 256, and a
// timeline bubble is no place for them. The remainder is counted, never expanded.
private const val ChecklistRows=8

@Composable internal fun ChecklistCard(message:ChatMessage,part:MessagePart,analyze:(String)->String,command:Command?) {
    val task=part.kind=="task"
    var confirming by remember(part.id){mutableStateOf<CardItem?>(null)}
    val motion=LocalMotion.current
    val compact=LocalAppearance.current.compact
    val done=part.items.count {it.checked}
    fun act(item:CardItem) {command?.invoke("card_action",mapOf("peer" to message.peer,"author" to message.author,"message" to message.id,"card" to part.id,"item" to item.id,"checked" to !item.checked))}
    CardColumn {
        Row(verticalAlignment=Alignment.Top,horizontalArrangement=Arrangement.spacedBy(12.dp)) {
            Box(Modifier.weight(1f)) {
                if(part.rich!=null)RichMessageText(part.rich,style=MaterialTheme.typography.titleMedium)
                else MessageText(part.text,analyze)
            }
            if(part.items.isNotEmpty())Text("$done/${part.items.size}",style=MaterialTheme.typography.labelMedium,color=LocalContentColor.current.copy(alpha=.68f),maxLines=1)
        }
        if(part.items.isNotEmpty()) {if(task && part.items.size<=ChecklistRows)TaskPips(part,done) else ChecklistRule(part,done)}
        Column(verticalArrangement=Arrangement.spacedBy(if(compact)2.dp else 4.dp)) {
            part.items.take(ChecklistRows).forEachIndexed {index,item->key(item.id) {
                if(task)TaskRow(index,item,command,{confirming=item},{act(item)})
                else CheckRow(item,part.kind=="recurring",command) {act(item)}
            }}
        }
        if(part.items.size>ChecklistRows)Text("+${part.items.size-ChecklistRows} more",style=MaterialTheme.typography.labelMedium,color=LocalContentColor.current.copy(alpha=.68f),maxLines=1)
        if(task)TemporalCaption("Confirm to complete · 30s undo")
        else if(part.kind=="recurring")TemporalCaption(part.date)
    }
    confirming?.let {item->AlertDialog(onDismissRequest={confirming=null},title={Text("Complete this task?")},text={Text("You can undo your completion for 30 seconds.")},confirmButton={SigilTextButton({confirming=null;part.items.firstOrNull {it.id==item.id && !it.checked && it.enabled}?.let(::act)}){Text("Complete")}},dismissButton={SigilTextButton({confirming=null}){Text("Cancel")}})}
}

// A continuous rule under the title, as in the reference: progress belongs to the list, not to any row.
@Composable private fun ChecklistRule(part:MessagePart,done:Int) {
    val fraction by animateFloatAsState(if(part.items.isEmpty())0f else done.toFloat()/part.items.size,LocalMotion.current.tween(MotionMillis),label="Checklist progress")
    Box(Modifier.fillMaxWidth().height(2.dp).clip(RoundedCornerShape(2.dp)).background(LocalContentColor.current.copy(alpha=.18f))) {
        Box(Modifier.fillMaxHeight().fillMaxWidth(fraction).clip(RoundedCornerShape(2.dp)).background(LocalContentColor.current))
    }
}
// Tasks get one segment each, so a task list never reads as a checklist at a glance; past the row cap it falls back to the rule.
@Composable private fun TaskPips(part:MessagePart,done:Int) {
    Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.spacedBy(3.dp)) {
        part.items.forEachIndexed {index,_->
            val filled by animateFloatAsState(if(index<done)1f else .18f,LocalMotion.current.tween(MotionMillis),label="Task segment")
            Box(Modifier.weight(1f).height(3.dp).clip(RoundedCornerShape(2.dp)).background(LocalContentColor.current.copy(alpha=filled)))
        }
    }
}

@Composable private fun CheckRow(item:CardItem,recurring:Boolean,command:Command?,act:()->Unit) {
    val motion=LocalMotion.current
    val mark=LocalContentColor.current.copy(alpha=if(item.checked).55f else .34f)
    Row(Modifier.fillMaxWidth().heightIn(min=44.dp).clip(RoundedCornerShape(10.dp))
        .clickable(enabled=command!=null && item.enabled,role=Role.Checkbox,onClick=act)
        .padding(horizontal=2.dp,vertical=6.dp).semantics {toggleableState=ToggleableState(item.checked)},
        verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(12.dp)) {
        Box(Modifier.size(20.dp).clip(RoundedCornerShape(6.dp)).border(1.5.dp,mark,RoundedCornerShape(6.dp)),contentAlignment=Alignment.Center) {
            AnimatedContent(item.checked,transitionSpec={(fadeIn(motion.enter(MotionInline))+scaleIn(motion.enter(MotionInline),initialScale=.6f)) togetherWith fadeOut(motion.exit(MotionExit))},label="Check mark") {checked->if(checked)Glyph("check",14) else Spacer(Modifier.size(14.dp))}
        }
        Box(Modifier.weight(1f).alpha(if(item.checked).6f else 1f)) {
            if(item.rich!=null)RichMessageText(item.rich,style=MaterialTheme.typography.bodyMedium)
            else Text(item.text,style=MaterialTheme.typography.bodyMedium)
        }
        // Rows that survive a scheduled reset, marked without the reference's replay glyph.
        if(recurring && item.persistent)Glyph("push_pin",14,"Kept through resets")
    }
}

// Numbered tiles and a struck-through completion: a task list is recognisable before a word is read.
@Composable private fun TaskRow(index:Int,item:CardItem,command:Command?,confirm:()->Unit,undo:()->Unit) {
    val motion=LocalMotion.current
    val surface=LocalMessageSurface.current.takeOrElse {MaterialTheme.colorScheme.surface}
    val tile=lerp(surface,LocalContentColor.current,if(item.checked).04f else .09f)
    val undoable=item.checked && item.enabled && command!=null
    Row(Modifier.fillMaxWidth().heightIn(min=52.dp).clip(RoundedCornerShape(14.dp)).background(tile)
        .clickable(enabled=command!=null && item.enabled,role=Role.Button,onClick={if(item.checked)undo() else confirm()})
        .padding(horizontal=12.dp,vertical=8.dp),
        verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(12.dp)) {
        AnimatedContent(item.checked,transitionSpec={(fadeIn(motion.enter(MotionInline))+scaleIn(motion.enter(MotionInline),initialScale=.7f)) togetherWith fadeOut(motion.exit(MotionExit))},label="Task marker") {complete->
            Box(Modifier.size(26.dp).clip(CircleShape).then(if(complete)Modifier.background(LocalContentColor.current.copy(alpha=.18f)) else Modifier.border(1.5.dp,LocalContentColor.current.copy(alpha=.34f),CircleShape)),contentAlignment=Alignment.Center) {
                if(complete)Glyph("check",16) else Text("${index+1}",style=MaterialTheme.typography.labelMedium,maxLines=1)
            }
        }
        Box(Modifier.weight(1f).alpha(if(item.checked).55f else 1f)) {
            if(item.rich!=null)RichMessageText(item.rich,style=MaterialTheme.typography.bodyMedium)
            else Text(item.text,style=MaterialTheme.typography.bodyMedium,textDecoration=if(item.checked)TextDecoration.LineThrough else null,maxLines=3,overflow=TextOverflow.Ellipsis)
        }
        if(undoable)Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(6.dp)) {Glyph("undo",16);Text("Undo",style=MaterialTheme.typography.labelSmall)}
        else if(item.checked)Glyph("lock",14,"Final")
    }
}
