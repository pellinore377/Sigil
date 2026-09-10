package org.sigil

import androidx.compose.animation.*
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.grid.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.*
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp

private data class CreateTool(val name:String,val icon:String,val category:String,val terms:String)
private val createTools=listOf(
    CreateTool("Note","description","Plan & Organize","write memo remember"),
    CreateTool("Checklist","checklist","Plan & Organize","list shopping groceries check"),
    CreateTool("Task","assignment","Plan & Organize","todo to-do jobs complete"),
    CreateTool("Reminder","notifications_active","Plan & Organize","date remind alarm later"),
    CreateTool("Timer","timer","Plan & Organize","duration minutes seconds time"),
    CreateTool("Poll","ballot","Ask & Decide","vote question options survey"),
    CreateTool("Randomizer","casino","Ask & Decide","dice roll pick choice number coin flip random"),
    CreateTool("Table","table","Data & Visualize","columns rows spreadsheet cells grid")
)
internal val createItems=createTools.map {it.name to it.icon}+listOf("Help" to "help")

@Composable
internal fun CreatePanel(back:()->Unit,open:(String)->Unit) {
    var category by rememberSaveable {mutableStateOf("")}
    var query by rememberSaveable {mutableStateOf("")}
    val motion=LocalMotion.current
    val words=query.trim().lowercase().split(' ').filter {it.isNotEmpty()}
    val searching=words.isNotEmpty()
    val categories=createTools.map {it.category}.distinct()
    fun previous() {if(searching)query="" else if(category.isNotEmpty())category="" else back()}
    BackAction(searching || category.isNotEmpty(),::previous)
    Column(Modifier.fillMaxSize().padding(horizontal=20.dp,vertical=8.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment=Alignment.CenterVertically) {
            Symbol("chevron_left",if(searching)"Clear tool search" else if(category.isNotEmpty())"Back to categories" else "Back to attachments",::previous)
            Text("Create",Modifier.weight(1f),style=MaterialTheme.typography.titleMedium)
            Symbol("help","Help") {open("Help")}
        }
        OutlinedTextField(query,{query=it.replace('\n',' ').take(128)},Modifier.fillMaxWidth(),singleLine=true,label={Text("Search tools")},shape=RoundedCornerShape(16.dp),leadingIcon={Glyph("search",20)},
            trailingIcon=if(query.isNotEmpty()){{Symbol("close","Clear search") {query=""}}}else null,keyboardOptions=KeyboardOptions(imeAction=ImeAction.Done))
        AnimatedContent(category to query.trim(),Modifier.weight(1f),contentKey={if(it.second.isEmpty())it.first else "Search"},transitionSpec={
            val returning=targetState.first.isEmpty() && targetState.second.isEmpty()
            (slideInHorizontally(motion.tween(MotionMillis)){if(returning)-it else it}+fadeIn(motion.tween(MotionMillis))) togetherWith
                (slideOutHorizontally(motion.tween(MotionMillis)){if(returning)it else -it}+fadeOut(motion.tween(MotionMillis)))
        },label="Create category") {(shownCategory,shownQuery)->
            val root=shownCategory.isEmpty() && shownQuery.isEmpty()
            val terms=shownQuery.lowercase().split(' ').filter {it.isNotEmpty()}
            val matches=createTools.filter {tool->if(terms.isNotEmpty())terms.all {it in "${tool.name} ${tool.category} ${tool.terms}".lowercase()} else tool.category==shownCategory}
            val choices=if(root)categories.map {name->name to when(name){"Plan & Organize"->"event_note";"Ask & Decide"->"ballot";else->"bar_chart"}} else matches.map {it.name to it.icon}
            Column(verticalArrangement=Arrangement.spacedBy(8.dp)) {
                if(!root)Text(if(shownQuery.isNotEmpty())"${matches.size} ${if(matches.size==1)"tool" else "tools"}" else shownCategory,style=MaterialTheme.typography.labelLarge)
                if(!root && choices.isEmpty())Text("No matching tools. Use Help to explore SigilText syntax.",style=MaterialTheme.typography.bodyMedium)
                LazyVerticalGrid(GridCells.Fixed(if(root)2 else 3),contentPadding=PaddingValues(vertical=8.dp),verticalArrangement=Arrangement.spacedBy(12.dp),horizontalArrangement=Arrangement.spacedBy(12.dp)) {
                    items(choices,key={it.first}) {(name,icon)->
                        Column(Modifier.clickable(role=Role.Button){if(root)category=name else open(name)}.semantics {contentDescription=name},horizontalAlignment=Alignment.CenterHorizontally,verticalArrangement=Arrangement.spacedBy(6.dp)) {
                            Surface(Modifier.size(58.dp),shape=RoundedCornerShape(19.dp),color=MaterialTheme.colorScheme.surfaceVariant) {Box(contentAlignment=Alignment.Center) {Glyph(icon,28)}}
                            Text(name,style=MaterialTheme.typography.bodyMedium,textAlign=TextAlign.Center)
                        }
                    }
                }
            }
        }
    }
}
