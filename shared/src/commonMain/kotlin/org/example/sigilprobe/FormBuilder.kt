package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.unit.dp
import kotlinx.serialization.json.*

val LocalStructuredPreview=staticCompositionLocalOf<((String)->MessagePart?)?> {null}
val LocalBuilderTimezone=staticCompositionLocalOf {"UTC"}
internal val LocalBuilderAction=staticCompositionLocalOf {"Send"}
internal data class FormSpec(val fields:List<String>,val defaults:List<String> = emptyList(),val modes:List<String> = emptyList(),val title:Boolean=false,val rows:List<String> = emptyList())
internal val formSpecs=mapOf(
    "Chart" to FormSpec(emptyList(),modes=listOf("bar","line","area","pie","donut","scatter"),title=true,rows=listOf("Label / X","Value / Y")),
    "Diagram" to FormSpec(emptyList(),modes=listOf("flow","sequence","timeline","mindmap","org","state"),title=true,rows=listOf("From / Date","To / Event","Label")),
    "Recipe" to FormSpec(listOf("Servings (optional)","Total time (optional)"),title=true),
    "Recurring checklist" to FormSpec(emptyList(),modes=listOf("weekly","monthly","yearly"),title=true,rows=listOf("Item")),
    "Countdown" to FormSpec(listOf("When"),title=true),"Elapsed time" to FormSpec(listOf("Since when"),title=true),
    "Calculation" to FormSpec(listOf("Expression")),"Conversion" to FormSpec(listOf("Value","Unit"),listOf("","miles")),
    "Rating" to FormSpec(listOf("Rating","Out of"),listOf("","5")),"Progress" to FormSpec(listOf("Percent"),listOf("0")),
    "Color swatch" to FormSpec(listOf("Color"),listOf("#808080")),"Keyboard shortcut" to FormSpec(listOf("Keys, e.g. Ctrl+Shift+P")),
    "Quote" to FormSpec(listOf("Author","Source (optional)","Quotation")),"QR code" to FormSpec(listOf("Content / Network name","Wi-Fi password"),modes=listOf("text","link","wifi")),
    "Math" to FormSpec(listOf("Formula (LaTeX)")),"ASCII art" to FormSpec(listOf("Your artwork"))

)
@Composable internal fun FormBuilder(kind:String,enabled:Boolean,back:()->Unit,send:(String,String?)->Unit) {
    val spec=formSpecs.getValue(kind)
    var title by rememberSaveable(kind){mutableStateOf("")}
    var mode by rememberSaveable(kind){mutableStateOf(spec.modes.firstOrNull().orEmpty())}
    var fields by rememberSaveable(kind){mutableStateOf(spec.fields.indices.map {spec.defaults.getOrElse(it){""}})}
    var rows by rememberSaveable(kind){mutableStateOf(listOf(List(spec.rows.size.coerceAtLeast(1)){""}))}
    var ingredients by rememberSaveable(kind){mutableStateOf(listOf(""))}
    var steps by rememberSaveable(kind){mutableStateOf(listOf(""))}
    var recurring by rememberSaveable(kind){mutableStateOf(true)}
    var syntax by rememberSaveable(kind){mutableStateOf(false)}
    var showPreview by rememberSaveable(kind){mutableStateOf(false)}
    val timezone=LocalBuilderTimezone.current
    val temporal=kind in listOf("Countdown","Elapsed time")
    val resolveTime=LocalTemporalPreview.current
    var date by remember(kind,fields){mutableStateOf<TemporalPreview?>(null)}
    LaunchedEffect(kind,fields,resolveTime) {if(temporal) {kotlinx.coroutines.delay(150);date=kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.Default){resolveTime?.invoke("Reminder",fields[0])}}}
    val input=buildJsonObject {
        put("kind",kind);put("title",title);put("mode",mode)
        put("fields",JsonArray(fields.mapIndexed {i,v->JsonPrimitive(if(temporal && i==0)date?.source.orEmpty() else v)}))
        val data=if(kind=="Recipe")ingredients.filter {it.isNotBlank()}.map {listOf("ingredients",it)}+steps.filter {it.isNotBlank()}.map {listOf("steps",it)} else rows.filter {it.any(String::isNotBlank)}.map {if(kind=="Recurring checklist")listOf(it[0],recurring.toString())else it}
        put("rows",JsonArray(data.map {JsonArray(it.map(::JsonPrimitive))}))
    }.toString()
    val resolve=LocalBuilderSource.current;val render=LocalStructuredPreview.current
    var validatedInput by remember(kind){mutableStateOf("")}
    var source by remember(kind){mutableStateOf("")}
    var preview by remember(kind){mutableStateOf<MessagePart?>(null)}
    var checking by remember(kind){mutableStateOf(false)}
    LaunchedEffect(input,resolve,render) {checking=true;kotlinx.coroutines.delay(120);val result=kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.Default) {val s=resolve?.invoke(input).orEmpty();s to s.takeIf {it.isNotEmpty()}?.let {render?.invoke(it)}};source=result.first;preview=result.second;validatedInput=input;checking=false}
    Column(Modifier.fillMaxSize().padding(horizontal=20.dp,vertical=8.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment=Alignment.CenterVertically) {Symbol("chevron_left","Back to create",back);Text(kind,Modifier.weight(1f),style=MaterialTheme.typography.titleLarge);SigilTextButton({showPreview=!showPreview},enabled=preview!=null) {Text(if(showPreview)"Edit" else "Preview")};Symbol("code","Show syntax") {syntax=!syntax}}
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(12.dp)) {
            if(showPreview && preview!=null)BuilderPreview(preview!!)
            else {
                if(spec.modes.isNotEmpty())LazyRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {items(spec.modes){m->FilterChip(mode==m,{mode=m},label={Text(m.replaceFirstChar {it.uppercase()})},shape=RoundedCornerShape(12.dp))}}
                if(spec.title)FormField("Title",title,{title=it})
                spec.fields.forEachIndexed {i,label->
                    if(!(kind=="QR code" && i==1 && mode!="wifi")) {
                        if(kind=="Color swatch")AccentPicker(parseAccent(fields[0]) ?: 0x808080) {fields=listOf("#"+accentText(it))}
                        else if(kind=="Progress") {Text("${fields[0]}%",style=MaterialTheme.typography.titleLarge);Slider(fields[0].toFloatOrNull() ?: 0f,{fields=listOf(it.toInt().toString())},valueRange=0f..100f)}
                        else FormField(label,fields[i],{v->fields=fields.toMutableList().also {it[i]=v}},multiline=kind in listOf("Math","ASCII art") || kind=="Quote"&&i==2 || kind=="Translation"&&i==1)
                    }
                }
                if(temporal)Text(date?.label ?: "Enter a date to preview the exact time.",style=MaterialTheme.typography.bodySmall)
                if(kind=="Recipe") {
                    Text("Ingredients",style=MaterialTheme.typography.titleMedium);BuilderEntries(ingredients,"Ingredient","restaurant"){ingredients=it}
                    Text("Steps",style=MaterialTheme.typography.titleMedium);BuilderEntries(steps,"Step","format_list_numbered"){steps=it}
                }
                if(spec.rows.isNotEmpty()) {
                    if(kind=="Recurring checklist")Toggle("Keep items after reset",recurring){recurring=it}
                    rows.forEachIndexed {i,row->key(i) {
                        val visible=remember {androidx.compose.animation.core.MutableTransitionState(i==0).apply {targetState=true}}
                        androidx.compose.animation.AnimatedVisibility(visible,enter=androidx.compose.animation.expandVertically()+androidx.compose.animation.fadeIn()) {
                            Surface(shape=RoundedCornerShape(16.dp),color=MaterialTheme.colorScheme.surfaceVariant) {
                                Column(Modifier.padding(12.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                                    Row(verticalAlignment=Alignment.CenterVertically) {Text("${i+1}",Modifier.weight(1f),style=MaterialTheme.typography.labelLarge);if(i<rows.lastIndex)Symbol("close","Remove row ${i+1}"){rows=rows.filterIndexed {n,_->n!=i}}}
                                    spec.rows.forEachIndexed {j,label->if(!(kind=="Diagram" && mode in listOf("timeline","mindmap","org") && j==2))FormField(label,row[j],{v->rows=rows.toMutableList().also {r->r[i]=row.toMutableList().also {it[j]=v};if(i==r.lastIndex && v.isNotBlank() && r.size<256)r.add(List(spec.rows.size){""})}})}
                                }
                            }
                        }
                    }}
                }
            }
            if(syntax && source.isNotEmpty())Text(source,fontFamily=LocalCodeFont.current,style=MaterialTheme.typography.bodySmall)
        }
        if(!checking && source.isEmpty() && (title.isNotEmpty() || fields.any {it.isNotEmpty()} || rows.any {r->r.any {it.isNotEmpty()}}))Text("Complete the fields with valid values to continue.",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
        SigilButton({send(source,if(temporal)date?.timezone else if(kind=="Recurring checklist")timezone else null)},Modifier.fillMaxWidth(),enabled=enabled && validatedInput==input && !checking && source.isNotEmpty() && (!temporal||date!=null)) {Text(LocalBuilderAction.current)}
    }
}
@Composable internal fun FormField(label:String,value:String,change:(String)->Unit,multiline:Boolean=false) {
    OutlinedTextField(value,{if(it.length<=8192)change(if(multiline)it else it.replace('\n',' '))},Modifier.fillMaxWidth(),label={Text(label)},singleLine=!multiline,minLines=if(multiline)3 else 1,shape=RoundedCornerShape(16.dp))
}
@Composable internal fun BuilderPreview(part:MessagePart) {
    val message=remember(part){ChatMessage("preview","preview","",true,"","sent",false,emptyList(),emptyList(),null,true,parts=listOf(part))}
    Surface(shape=RoundedCornerShape(20.dp),color=MaterialTheme.colorScheme.surfaceVariant) {Box(Modifier.padding(16.dp)) {CompositionLocalProvider(LocalRecipeScale provides null) {MessageCards(message,{""},null)}}}
}
