package org.sigil

import androidx.compose.animation.*
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.*
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.input.key.*
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp

val LocalHelpCatalog=staticCompositionLocalOf<((String)->String)?> {null}
internal data class HelpTopic(val name:String,val category:String,val description:String,val syntax:String,val options:String,val example:String)
internal fun helpTopics(raw:String)=raw.split('\u001e').mapNotNull {row->
    val fields=row.split('\u001f')
    if(fields.size==6)HelpTopic(fields[0],fields[1],fields[2],fields[3],fields[4],fields[5]) else null
}

@Composable
internal fun HelpPanel(enabled:Boolean,back:()->Unit,initialQuery:String?=null,send:(String)->Unit) {
    val catalog=LocalHelpCatalog.current
    val all=remember(catalog) {helpTopics(catalog?.invoke("").orEmpty())}
    var query by rememberSaveable {mutableStateOf(initialQuery.orEmpty())}
    var category by rememberSaveable {mutableStateOf("")}
    var selected by rememberSaveable {mutableStateOf<String?>(null)}
    val matches=remember(catalog,query,category) {helpTopics(catalog?.invoke("$category $query".trim()).orEmpty())}
    val clipboard=LocalClipboardManager.current
    val motion=LocalMotion.current
    val focus=remember {FocusRequester()}
    var cursor by remember(query,category) {mutableIntStateOf(-1)}
    val list=rememberLazyListState()
    LaunchedEffect(cursor) {if(cursor>=0)list.scrollToItem(cursor)}
    LaunchedEffect(initialQuery) {if(initialQuery!=null) {query=initialQuery;focus.requestFocus()}}
    BackAction(selected!=null) {selected=null}
    Column(Modifier.fillMaxSize().padding(horizontal=20.dp,vertical=8.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment=Alignment.CenterVertically) {
            Symbol("chevron_left",if(selected!=null)"Back to help" else if(initialQuery==null)"Back to create" else "Close help") {if(selected==null)back() else selected=null}
            Text("SigilText help",Modifier.weight(1f),style=MaterialTheme.typography.titleMedium)
        }
        AnimatedContent(selected,transitionSpec={
            (slideInHorizontally(motion.tween(MotionMillis)) {if(targetState==null)-it else it}+fadeIn(motion.tween(MotionMillis))) togetherWith
                (slideOutHorizontally(motion.tween(MotionMillis)) {if(targetState==null)it else -it}+fadeOut(motion.tween(MotionMillis)))
        },label="Help topic") {name->
            val topic=all.firstOrNull {it.name==name}
            if(topic==null)Column(verticalArrangement=Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(query,{query=it.take(128)},Modifier.fillMaxWidth().focusRequester(focus).onPreviewKeyEvent {event->
                    if(event.type!=KeyEventType.KeyDown)false else when(event.key) {
                        Key.DirectionDown->{cursor=(cursor+1).coerceAtMost(matches.lastIndex);true}
                        Key.DirectionUp->{cursor=(cursor-1).coerceAtLeast(if(matches.isEmpty())-1 else 0);true}
                        Key.Enter,Key.Tab->{if(event.key==Key.Tab && event.isShiftPressed)false else {matches.getOrNull(cursor.coerceAtLeast(0))?.let {selected=it.name};true}}
                        Key.Escape->{back();true}
                        else->false
                    }
                },singleLine=true,shape=RoundedCornerShape(16.dp),label={Text("Search SigilText")},
                    keyboardOptions=KeyboardOptions(imeAction=ImeAction.Search),keyboardActions=KeyboardActions(onSearch={matches.firstOrNull()?.let {selected=it.name}}))
                LazyRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                    items(listOf("")+all.map {it.category}.distinct()) {value->
                        FilterChip(selected=category==value,onClick={category=value},label={Text(value.replaceFirstChar {it.uppercase()}.ifEmpty {"All"})},shape=RoundedCornerShape(12.dp))
                    }
                }
                LazyColumn(state=list,verticalArrangement=Arrangement.spacedBy(6.dp)) {
                    if(matches.isEmpty())item {Text(if(catalog==null)"The reference is unavailable." else "No matching topics.",style=MaterialTheme.typography.bodyMedium)}
                    items(matches,key={it.name}) {entry->
                        Surface(shape=RoundedCornerShape(14.dp),color=if(matches.getOrNull(cursor)?.name==entry.name)MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceVariant) {
                            Column(Modifier.fillMaxWidth().clickable(role=Role.Button) {selected=entry.name}.padding(12.dp),verticalArrangement=Arrangement.spacedBy(4.dp)) {
                                Text(entry.name,style=MaterialTheme.typography.titleSmall)
                                Text(entry.description,style=MaterialTheme.typography.bodySmall)
                            }
                        }
                    }
                }
            } else Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                Text(topic.name,style=MaterialTheme.typography.titleLarge)
                Text(topic.description)
                Text(topic.syntax,fontFamily=LocalCodeFont.current,style=MaterialTheme.typography.bodyMedium)
                if(topic.options.isNotEmpty())Text(topic.options.replace('\n',' '),style=MaterialTheme.typography.bodySmall)
                Surface(shape=RoundedCornerShape(14.dp),color=MaterialTheme.colorScheme.surfaceVariant) {
                    Text(topic.example,Modifier.fillMaxWidth().padding(12.dp),fontFamily=LocalCodeFont.current,style=MaterialTheme.typography.bodyMedium)
                }
                SigilTextButton({clipboard.setText(AnnotatedString(topic.example))}) {Glyph("content_copy",18);Spacer(Modifier.width(8.dp));Text("Copy example")}
                SigilButton({send("help::${topic.name};")},enabled=enabled,modifier=Modifier.fillMaxWidth()) {Text("Send cheat sheet")}
            }
        }
    }
}
