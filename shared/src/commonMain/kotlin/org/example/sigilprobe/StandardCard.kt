@file:OptIn(kotlin.time.ExperimentalTime::class)
package org.sigil

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.semantics.*
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.unit.dp

@Composable internal fun StandardCard(message:ChatMessage,part:MessagePart,analyze:(String)->String,command:Command?) {
    var expanded by remember(part.id){mutableStateOf(false)}
    var confirming by remember(part.id){mutableStateOf<CardItem?>(null)}
    val icon=when(part.kind){"note"->"description";"checklist"->"checklist";"task"->"assignment";"poll"->"ballot";"reminder"->"notifications_active";else->"timer"}
    fun act(item:CardItem) {
        val fields=mutableMapOf<String,Any?>("peer" to message.peer,"author" to message.author,"message" to message.id,"card" to part.id)
        if(part.kind=="poll")fields["choices"]=if(part.multiple)part.items.filter {if(it.id==item.id)!it.checked else it.checked}.map {it.id}else if(item.checked)emptyList<String>()else listOf(item.id)
        else {fields["item"]=item.id;fields["checked"]=!item.checked}
        command?.invoke("card_action",fields)
    }
    @Composable fun body(full:Boolean) {
        Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {Glyph(icon,18);Text(part.kind.replaceFirstChar {it.uppercase()},style=MaterialTheme.typography.labelMedium)}
        if(part.rich!=null)RichMessageText(part.rich,style=MaterialTheme.typography.titleMedium)else MessageText(part.text,analyze)
        if(part.at>0 && part.kind in listOf("timer","countdown","ago"))CardClock(part)
        if(full || part.kind=="reminder")if(part.date.isNotEmpty())Text(part.date,style=MaterialTheme.typography.bodySmall)
        if(part.kind in listOf("checklist","task") && part.items.isNotEmpty())Text("${part.items.count {it.checked}} of ${part.items.size} completed",style=MaterialTheme.typography.labelSmall)
        (if(full)part.items else part.items.take(5)).forEach {item->key(item.id) {
            val tint by animateColorAsState(LocalContentColor.current.copy(alpha=if(item.checked).13f else .04f),label="Checked item")
            val share=if(part.kind=="poll" && item.count!=null && part.voters!=null && part.voters>0)item.count.toFloat()/part.voters else 0f
            val amount by animateFloatAsState(share.coerceIn(0f,1f),label="Poll result")
            Box(Modifier.fillMaxWidth().heightIn(min=44.dp).background(tint,RoundedCornerShape(12.dp)).clickable(enabled=command!=null && item.enabled,role=if(part.kind=="poll" && !part.multiple)Role.RadioButton else Role.Checkbox) {
                if(part.kind=="task" && !item.checked)confirming=item else act(item)
            }.semantics {if(part.kind=="poll" && !part.multiple)selected=item.checked else toggleableState=ToggleableState(item.checked)}) {
                if(share>0)Box(Modifier.matchParentSize().padding(2.dp)) {Box(Modifier.fillMaxHeight().fillMaxWidth(amount).background(LocalContentColor.current.copy(alpha=.12f),RoundedCornerShape(10.dp)))}
                Row(Modifier.padding(horizontal=10.dp,vertical=10.dp),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                    Glyph(if(part.kind=="poll" && !part.multiple)if(item.checked)"radio_button_checked"else"radio_button_unchecked"else if(item.checked)"check_box"else"check_box_outline_blank",20)
                    if(item.rich!=null)RichMessageText(item.rich,Modifier.weight(1f),MaterialTheme.typography.bodyMedium)else Text(item.text,Modifier.weight(1f),style=MaterialTheme.typography.bodyMedium)
                    item.count?.let {Text(it.toString(),style=MaterialTheme.typography.labelMedium)}
                    if(part.kind=="task" && item.checked && item.enabled && command!=null)Text("Undo",style=MaterialTheme.typography.labelSmall)
                }
            }
        }}
        if(part.kind=="poll")Text(if(part.closed)"Voting closed"else part.voters?.let {"$it ${if(it==1L)"voter"else"voters"}"}?:"Vote to see results",style=MaterialTheme.typography.labelSmall)
        if (part.items.size > 5) SigilTextButton({ expanded = !expanded }) { Text(if (expanded) "Show less" else "Show all ${part.items.size}") }
    }
    Column(Modifier.widthIn(min=200.dp,max=320.dp).animateContentSize(LocalMotion.current.tween(MotionMillis)),verticalArrangement=Arrangement.spacedBy(8.dp)) {body(expanded)}
    confirming?.let {item->AlertDialog(onDismissRequest={confirming=null},title={Text("Complete this task?")},text={Text("You can undo your completion for 30 seconds.")},confirmButton={SigilTextButton({confirming=null;part.items.firstOrNull {it.id==item.id && !it.checked && it.enabled}?.let(::act)}){Text("Complete")}},dismissButton={SigilTextButton({confirming=null}){Text("Cancel")}})}
}
@Composable private fun CardClock(part:MessagePart) {
    var now by remember {mutableLongStateOf(kotlin.time.Clock.System.now().epochSeconds)}
    LaunchedEffect(part.at) {while(true){now=kotlin.time.Clock.System.now().epochSeconds;kotlinx.coroutines.delay(1000)}}
    val seconds=if(part.kind=="ago")(now-part.at).coerceAtLeast(0)else(part.at-now).coerceAtLeast(0)
    val value=when {seconds>=86400->"${seconds/86400}d ${(seconds%86400)/3600}h";seconds>=3600->"${seconds/3600}h ${(seconds%3600)/60}m";else->"${seconds/60}:${(seconds%60).toString().padStart(2,'0')}"}
    Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(16.dp)) {
        if(part.kind=="timer" && part.startedAt>0)CircularProgressIndicator(progress={seconds.toFloat()/(part.at-part.startedAt).coerceAtLeast(1)},modifier=Modifier.size(40.dp),color=LocalContentColor.current,trackColor=LocalContentColor.current.copy(alpha=.12f))
        Text(if(seconds==0L && part.kind!="ago")"Time’s up"else value+if(part.kind=="ago")" ago"else"",style=MaterialTheme.typography.headlineMedium,fontFamily=LocalCodeFont.current)
    }
}
