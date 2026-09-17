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
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.*
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.math.roundToInt

@Composable internal fun PollCard(message:ChatMessage,part:MessagePart,analyze:(String)->String,command:Command?) {
    var expanded by remember(part.id){mutableStateOf(false)}
    val motion=LocalMotion.current
    val compact=LocalAppearance.current.compact
    val voters=part.voters
    val leading=if(part.closed && voters!=null)part.items.mapNotNull {it.count}.maxOrNull()?.takeIf {it>0L} else null
    val lines=with(LocalDensity.current){MaterialTheme.typography.bodyMedium.lineHeight.toDp()*3}
    fun act(item:CardItem) {
        val choices=if(part.multiple)part.items.filter {if(it.id==item.id)!it.checked else it.checked}.map {it.id} else if(item.checked)emptyList() else listOf(item.id)
        command?.invoke("card_action",mapOf("peer" to message.peer,"author" to message.author,"message" to message.id,"card" to part.id,"choices" to choices))
    }
    CardFrame("ballot","Poll") {
        if(part.rich!=null)RichMessageText(part.rich,style=MaterialTheme.typography.titleMedium)else MessageText(part.text,analyze)
        if(part.items.isEmpty())Text("No options yet.",style=MaterialTheme.typography.bodyMedium)
        else {
            Text(listOf(if(part.closed)"Final results"else if(part.multiple)"Choose any"else"Choose one","${part.items.size} options").joinToString(" · "),style=MaterialTheme.typography.labelMedium)
            Column(verticalArrangement=Arrangement.spacedBy(if(compact)4.dp else 6.dp)) {
                (if(expanded)part.items else part.items.take(5)).forEach {item->key(item.id) {
                    val votes=item.count?:0L
                    val share=if(voters!=null && voters>0L)(votes.toFloat()/voters).coerceIn(0f,1f)else 0f
                    val amount by animateFloatAsState(share,motion.tween(MotionMillis),label="Poll result")
                    val tint by animateColorAsState(LocalContentColor.current.copy(alpha=if(item.checked).13f else .04f),motion.tween(MotionInline),label="Poll option")
                    val won=leading!=null && item.count==leading
                    Box(Modifier.fillMaxWidth().heightIn(min=44.dp).clip(RoundedCornerShape(12.dp)).background(tint)
                        .clickable(enabled=command!=null && item.enabled,role=if(part.multiple)Role.Checkbox else Role.RadioButton) {act(item)}
                        .semantics {
                            if(part.multiple)toggleableState=ToggleableState(item.checked)else selected=item.checked
                            if(voters!=null)stateDescription="${(share*100).roundToInt()}%, $votes of $voters"+if(won)", most votes"else""
                        }) {
                        if(amount>0f)Box(Modifier.matchParentSize().padding(2.dp)) {Box(Modifier.fillMaxHeight().fillMaxWidth(amount).clip(RoundedCornerShape(10.dp)).background(LocalContentColor.current.copy(alpha=if(won).16f else .12f)))}
                        Row(Modifier.padding(horizontal=10.dp,vertical=if(compact)8.dp else 10.dp),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                            AnimatedContent(item.checked,transitionSpec={(fadeIn(motion.enter(MotionInline))+scaleIn(motion.enter(MotionInline),initialScale=.8f)) togetherWith fadeOut(motion.exit(MotionExit))},label="Poll choice") {checked->
                                Glyph(if(part.multiple)if(checked)"check_box"else"check_box_outline_blank"else if(checked)"radio_button_checked"else"radio_button_unchecked",20)
                            }
                            if(voters!=null)Column(Modifier.widthIn(min=44.dp),horizontalAlignment=Alignment.Start) {
                                Text("${(amount*100).roundToInt()}%",style=MaterialTheme.typography.labelMedium)
                                if(!compact)Text("$votes ${if(votes==1L)"vote"else"votes"}",style=MaterialTheme.typography.labelSmall,maxLines=1)
                            }
                            Column(Modifier.weight(1f),verticalArrangement=Arrangement.spacedBy(4.dp)) {
                                if(item.rich!=null)RichMessageText(item.rich,Modifier.heightIn(max=if(expanded)Dp.Unspecified else lines).clipToBounds(),MaterialTheme.typography.bodyMedium)
                                else Text(item.text,style=MaterialTheme.typography.bodyMedium,maxLines=if(expanded)Int.MAX_VALUE else 3,overflow=TextOverflow.Ellipsis)
                                if(won)Text("Most votes",Modifier.clearAndSetSemantics{},style=MaterialTheme.typography.labelSmall)
                            }
                        }
                    }
                }}
            }
            Text(if(part.closed)"Voting closed"+(voters?.let {" · $it ${if(it==1L)"voter"else"voters"}"}?:"")
                else if(voters==null)"Vote to see results"else if(voters==0L)"No votes yet"else"$voters ${if(voters==1L)"voter"else"voters"}",style=MaterialTheme.typography.labelSmall)
            if(part.items.size>5)SigilTextButton({expanded=!expanded}) {Glyph(if(expanded)"expand_less"else"expand_more",18);Spacer(Modifier.width(8.dp));Text(if(expanded)"Show less"else"Show all ${part.items.size}")}
        }
    }
}
