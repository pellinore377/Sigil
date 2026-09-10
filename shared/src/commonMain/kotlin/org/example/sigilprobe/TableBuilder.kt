package org.sigil

import androidx.compose.animation.*
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.*
import androidx.compose.ui.focus.FocusDirection
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp

val LocalBuilderSource=staticCompositionLocalOf<((String)->String)?> {null}

@Composable
internal fun TableBuilder(enabled:Boolean,back:()->Unit,send:(String)->Unit) {
    var columns by rememberSaveable {mutableStateOf(listOf("",""))}
    var rows by rememberSaveable {mutableStateOf(listOf(listOf("","")))}
    var page by rememberSaveable {mutableStateOf("Columns")}
    var row by rememberSaveable {mutableIntStateOf(0)}
    var remove by remember {mutableIntStateOf(-1)}
    var tooLarge by remember {mutableStateOf(false)}
    val data=rows.filter {values->values.any {it.isNotBlank()}}
    val input=(listOf("Table",columns.joinToString("\t"))+data.map {it.joinToString("\t")}).joinToString("\n")
    val resolve=LocalBuilderSource.current
    val source=remember(input,resolve) {resolve?.invoke(input).orEmpty()}
    val preview=remember(columns,data,source) {
        if(source.isEmpty())null else TableContent(columns.map {RichText(it.trim())},data.map {values->values.map {RichText(it.trim())}},List(columns.size){null},List(data.size){null},null)
    }
    val motion=LocalMotion.current
    val focus=LocalFocusManager.current
    val keyboard=LocalSoftwareKeyboardController.current
    val pages=listOf("Columns","Rows","Preview")
    fun update(names:List<String>,values:List<List<String>>) {
        val fields=names+values.flatten()
        if(fields.sumOf {it.length}>16384 || (listOf("Table",names.joinToString("\t"))+values.filter {v->v.any {it.isNotBlank()}}.map {it.joinToString("\t")}).joinToString("\n").encodeToByteArray().size>16384) {
            tooLarge=true
        } else {tooLarge=false;columns=names;rows=values}
    }
    fun change(value:String) {focus.clearFocus();keyboard?.hide();page=value}
    fun deleteColumn(index:Int) {
        columns=columns.filterIndexed {i,_->i!=index}
        rows=rows.map {values->values.filterIndexed {i,_->i!=index}}
        remove=-1
    }
    Column(Modifier.fillMaxSize().padding(horizontal=20.dp,vertical=8.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment=Alignment.CenterVertically) {
            Symbol("chevron_left","Back to create",back)
            Text("Table",Modifier.weight(1f),style=MaterialTheme.typography.titleLarge)
        }
        Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
            pages.forEach {value->FilterChip(selected=page==value,onClick={change(value)},label={Text(value)},shape=RoundedCornerShape(12.dp))}
        }
        if(tooLarge)Text("That edit is too large. Shorten the text and try again.",color=MaterialTheme.colorScheme.error,style=MaterialTheme.typography.bodySmall)
        AnimatedContent(page,Modifier.weight(1f),transitionSpec={
            (slideInHorizontally(motion.tween(MotionMillis)){if(pages.indexOf(targetState)>pages.indexOf(initialState))it else -it}+fadeIn(motion.tween(MotionMillis))) togetherWith
                (slideOutHorizontally(motion.tween(MotionMillis)){if(pages.indexOf(targetState)>pages.indexOf(initialState))-it else it}+fadeOut(motion.tween(MotionMillis)))
        },label="Table form") {shown->
            when(shown) {
                "Columns"->LazyColumn(verticalArrangement=Arrangement.spacedBy(12.dp)) {
                    item {Text("Name the columns, then enter your rows.",style=MaterialTheme.typography.bodySmall)}
                    itemsIndexed(columns) {index,value->
                        Row(Modifier.animateItem(fadeInSpec=motion.tween(MotionMillis),placementSpec=motion.tween(MotionMillis)),verticalAlignment=Alignment.CenterVertically) {
                            TableField(value,{update(columns.toMutableList().also {v->v[index]=it},rows)},"Column ${index+1}",Modifier.weight(1f))
                            if(columns.size>1)SigilIconButton({if(rows.any {it[index].isNotBlank()})remove=index else deleteColumn(index)}) {Glyph("close",20,"Remove column ${index+1}")}
                        }
                    }
                    if(columns.size<64)item {SigilTextButton({columns=columns+"";rows=rows.map {it+""}}) {Glyph("add",20);Text("Add column")}}
                    item {SigilButton({change("Rows")},Modifier.fillMaxWidth(),enabled=columns.any {it.isNotBlank()}) {Text("Enter rows")}}
                }
                "Rows"->Column(verticalArrangement=Arrangement.spacedBy(8.dp)) {
                    Row(verticalAlignment=Alignment.CenterVertically) {
                        SigilIconButton({focus.clearFocus();row--},enabled=row>0) {Glyph("chevron_left",24,"Previous row")}
                        Text("Row ${row+1} of ${rows.size}",Modifier.weight(1f),style=MaterialTheme.typography.titleSmall)
                        SigilIconButton({focus.clearFocus();row++},enabled=row<rows.lastIndex) {Glyph("chevron_right",24,"Next row")}
                        if(rows.size>1)SigilIconButton({rows=rows.filterIndexed {i,_->i!=row};row=row.coerceAtMost(rows.lastIndex)}) {Glyph("delete",20,"Remove row ${row+1}")}
                    }
                    AnimatedContent(row,Modifier.weight(1f),transitionSpec={
                        (slideInHorizontally(motion.tween(MotionMillis)){if(targetState>initialState)it else -it}+fadeIn(motion.tween(MotionMillis))) togetherWith
                            (slideOutHorizontally(motion.tween(MotionMillis)){if(targetState>initialState)-it else it}+fadeOut(motion.tween(MotionMillis)))
                    },label="Table row") {shownRow->
                    val cells=rows.getOrNull(shownRow)
                    if(cells!=null)LazyColumn(verticalArrangement=Arrangement.spacedBy(12.dp)) {
                        itemsIndexed(columns) {index,name->
                            TableField(cells[index],{value->if(shownRow in rows.indices)update(columns,rows.toMutableList().also {v->v[shownRow]=v[shownRow].toMutableList().also {it[index]=value}})},name.ifBlank {"Column ${index+1}"},Modifier.fillMaxWidth())
                        }
                        if(rows.size<256)item {
                            SigilTextButton({focus.clearFocus();rows=rows+listOf(List(columns.size){""});row=rows.lastIndex},enabled=cells.any {it.isNotBlank()}) {Glyph("add",20);Text("Add row")}
                        }
                        item {Text("Empty rows are left out. Each cell keeps its text literally.",style=MaterialTheme.typography.bodySmall)}
                        item {SigilButton({change("Preview")},Modifier.fillMaxWidth(),enabled=source.isNotEmpty()) {Text("Preview table")}}
                    }
                    }
                }
                else->Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                    if(preview!=null) {
                        Surface(shape=RoundedCornerShape(20.dp),color=MaterialTheme.colorScheme.primary) {
                            CompositionLocalProvider(LocalMessageSurface provides MaterialTheme.colorScheme.primary) {
                                Box(Modifier.fillMaxWidth().padding(12.dp),contentAlignment=Alignment.Center) {TableCard(preview)}
                            }
                        }
                        SigilButton({send(source)},Modifier.fillMaxWidth(),enabled=enabled) {Text("Send table")}
                    } else Text(if(resolve==null)"The builder is unavailable." else "Add a column name and at least one row. Tables support up to 64 columns and 256 rows within the message size limit.",style=MaterialTheme.typography.bodyMedium)
                }
            }
        }
    }
    if(remove>=0)AlertDialog(onDismissRequest={remove=-1},title={Text("Remove column?")},text={Text("The values in this column will also be removed from your draft.")},confirmButton={SigilTextButton({deleteColumn(remove)}) {Text("Remove")}},dismissButton={SigilTextButton({remove=-1}) {Text("Keep column")}})
}

@Composable
private fun TableField(value:String,change:(String)->Unit,label:String,modifier:Modifier) {
    val focus=LocalFocusManager.current
    OutlinedTextField(value,{change(it.replace('\n',' ').replace('\r',' ').replace('\t',' '))},modifier,shape=RoundedCornerShape(16.dp),singleLine=true,label={Text(label)},keyboardOptions=KeyboardOptions(imeAction=ImeAction.Next),keyboardActions=KeyboardActions(onNext={focus.moveFocus(FocusDirection.Next)}))
}
