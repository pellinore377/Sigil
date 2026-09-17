package org.sigil

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.animation.slideInHorizontally
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.focus.FocusDirection
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.input.ImeAction
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
    var options by rememberSaveable(kind){mutableStateOf(false)}
    val timezone=LocalBuilderTimezone.current
    val temporal=kind in listOf("Countdown","Elapsed time")
    val resolveTime=LocalTemporalPreview.current
    var date by remember(kind,fields){mutableStateOf<TemporalPreview?>(null)}
    var checkingTime by remember(kind,fields){mutableStateOf(temporal)}
    LaunchedEffect(kind,fields,resolveTime) {if(temporal) {kotlinx.coroutines.delay(150);date=kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.Default){resolveTime?.invoke("Reminder",fields[0])};checkingTime=false}}
    val input=buildJsonObject {
        put("kind",kind);put("title",title);put("mode",mode)
        put("fields",JsonArray(fields.mapIndexed {i,v->JsonPrimitive(if(temporal && i==0)date?.source.orEmpty() else v)}))
        val data=if(kind=="Recipe")ingredients.filter {it.isNotBlank()}.map {listOf("ingredients",it)}+steps.filter {it.isNotBlank()}.map {listOf("steps",it)} else rows.filter {it.any(String::isNotBlank)}.map {if(kind=="Recurring checklist")listOf(it[0],recurring.toString())else it}
        put("rows",JsonArray(data.map {JsonArray(it.map(::JsonPrimitive))}))
    }.toString()
    val resolve=LocalBuilderSource.current
    var validatedInput by remember(kind){mutableStateOf("")}
    var source by remember(kind){mutableStateOf("")}
    var checking by remember(kind){mutableStateOf(false)}
    LaunchedEffect(input,resolve) {checking=true;kotlinx.coroutines.delay(120);source=kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.Default) {resolve?.invoke(input).orEmpty()};validatedInput=input;checking=false}
    val motion=LocalMotion.current
    val sizing=rememberBuilderSizing(16.dp)
    Column(Modifier.fillMaxSize().padding(start=8.dp,end=8.dp,top=8.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        Row(sizing.measure("header"),verticalAlignment=Alignment.CenterVertically) {Symbol("chevron_left","Back to create",back);Text(kind,Modifier.weight(1f),style=MaterialTheme.typography.titleLarge);SyntaxToggle(syntax) {syntax=!syntax}}
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).wrapContentHeight(unbounded=true).then(sizing.measure("body")),verticalArrangement=Arrangement.spacedBy(12.dp)) {
            run {
                if(spec.modes.isNotEmpty())LazyRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {items(spec.modes){m->FilterChip(mode==m,{mode=m},label={Text(m.replaceFirstChar {it.uppercase()})},shape=RoundedCornerShape(12.dp))}}
                if(spec.title)FormField("Title",title,{title=it})
                if(kind!="Recipe")spec.fields.forEachIndexed {i,label->
                    if(!(kind=="QR code" && i==1 && mode!="wifi")) {
                        if(kind=="Color swatch")AccentPicker(parseAccent(fields[0]) ?: 0x808080) {fields=listOf("#"+accentText(it))}
                        else if(kind=="Progress")Column(verticalArrangement=Arrangement.spacedBy(4.dp)) {
                            Text(label,style=MaterialTheme.typography.labelMedium,color=MaterialTheme.colorScheme.onSurfaceVariant)
                            Text("${fields[0]}%",style=MaterialTheme.typography.labelLarge)
                            Slider(fields[0].toFloatOrNull() ?: 0f,{fields=listOf(it.toInt().toString())},Modifier.semantics {contentDescription=label},valueRange=0f..100f)
                        }
                        else FormField(label,fields[i],{v->fields=fields.toMutableList().also {it[i]=v}},multiline=kind in listOf("Math","ASCII art") || kind=="Quote"&&i==2 || kind=="Translation"&&i==1,isError=temporal && i==0 && !checkingTime && date==null && fields[0].isNotBlank())
                    }
                }
                if(temporal)Text(if(checkingTime)"Checking time…" else date?.label ?: if(resolveTime==null)"Time preview is unavailable." else "Enter a date to preview the exact time.",
                    Modifier.fillMaxWidth().animateContentSize(motion.tween(MotionMillis)).semantics {liveRegion=LiveRegionMode.Polite},style=MaterialTheme.typography.bodySmall,
                    color=if(checkingTime || date!=null || fields[0].isBlank())MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.error)
                if(kind=="Recipe") {
                    SectionLabel("Ingredients");BuilderEntries(ingredients,"Ingredient","restaurant"){ingredients=it}
                    SectionLabel("Steps");BuilderEntries(steps,"Step","format_list_numbered"){steps=it}
                    SigilTextButton({options=!options}) {Glyph(if(options)"expand_less" else "expand_more",20);Spacer(Modifier.width(8.dp));Text("Options")}
                    Expandable(options) {Column(verticalArrangement=Arrangement.spacedBy(8.dp)) {
                        spec.fields.forEachIndexed {i,label->FormField(label.removeSuffix(" (optional)"),fields[i],{v->fields=fields.toMutableList().also {it[i]=v}})}
                    }}
                }
                if(spec.rows.isNotEmpty()) {
                    if(kind=="Recurring checklist")Toggle("Keep items after reset",recurring){recurring=it}
                    rows.forEachIndexed {i,row->key(i) {
                        val visible=remember {MutableTransitionState(i==0).apply {targetState=true}}
                        AnimatedVisibility(visible,enter=expandVertically(motion.enter(MotionMillis),expandFrom=Alignment.Top)+slideInHorizontally(motion.enter(MotionMillis)){it}+fadeIn(motion.enter(MotionMillis)),exit=shrinkVertically(motion.exit(MotionMillis),shrinkTowards=Alignment.Top)+fadeOut(motion.exit(MotionExit)),label="Builder row") {
                            Column(verticalArrangement=Arrangement.spacedBy(8.dp)) {
                                Row(verticalAlignment=Alignment.CenterVertically) {Text("${i+1}",Modifier.weight(1f),style=MaterialTheme.typography.labelLarge);if(i<rows.lastIndex)Symbol("close","Remove row ${i+1}"){rows=rows.filterIndexed {n,_->n!=i}}}
                                spec.rows.forEachIndexed {j,label->if(!(kind=="Diagram" && mode in listOf("timeline","mindmap","org") && j==2))FormField(label,row[j],{v->rows=rows.toMutableList().also {r->r[i]=row.toMutableList().also {it[j]=v};if(i==r.lastIndex && v.isNotBlank() && r.size<256)r.add(List(spec.rows.size){""})}})}
                            }
                        }
                    }}
                }
            }
            SyntaxSource(syntax,source)
        }
        if(!checking && source.isEmpty() && (title.isNotEmpty() || fields.any {it.isNotEmpty()} || rows.any {r->r.any {it.isNotEmpty()}}))Text("Complete the fields with valid values to continue.",
            Modifier.fillMaxWidth().semantics {liveRegion=LiveRegionMode.Polite},style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.error)
        BuilderConfirm(enabled=enabled && validatedInput==input && !checking && source.isNotEmpty() && (!temporal||date!=null)) {send(source,if(temporal)date?.timezone else if(kind=="Recurring checklist")timezone else null)}
    }
}
@Composable internal fun FormField(label:String,value:String,change:(String)->Unit,multiline:Boolean=false,isError:Boolean=false) {
    val focus=LocalFocusManager.current
    OutlinedTextField(value,{if(it.length<=8192)change(if(multiline)it else it.replace('\n',' '))},Modifier.fillMaxWidth(),label={Text(label)},isError=isError,singleLine=!multiline,minLines=if(multiline)3 else 1,shape=RoundedCornerShape(16.dp),
        keyboardOptions=if(multiline)KeyboardOptions.Default else KeyboardOptions(imeAction=ImeAction.Next),keyboardActions=KeyboardActions(onNext={focus.moveFocus(FocusDirection.Next)}))
}
@Composable internal fun SyntaxToggle(shown:Boolean,toggle:()->Unit) {
    SigilIconButton(toggle,Modifier.semantics {contentDescription=if(shown)"Hide syntax" else "Show syntax";stateDescription=if(shown)"Shown" else "Hidden"}) {Glyph(if(shown)"code_off" else "code")}
}
@Composable internal fun SyntaxSource(shown:Boolean,source:String) {
    Expandable(shown && source.isNotEmpty()) {
        SelectionContainer {Text(source,Modifier.fillMaxWidth(),fontFamily=LocalCodeFont.current,style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)}
    }
}
@Composable internal fun SectionLabel(title:String) {
    Text(title,Modifier.padding(start=12.dp,top=20.dp,bottom=8.dp),style=MaterialTheme.typography.labelMedium,color=MaterialTheme.colorScheme.onSurfaceVariant)
}
@Composable internal fun BuilderPreview(part:MessagePart) {
    val message=remember(part){ChatMessage("preview","preview","",true,"","sent",false,emptyList(),emptyList(),null,true,parts=listOf(part))}
    Surface(shape=RoundedCornerShape(20.dp),color=MaterialTheme.colorScheme.primary) {
        CompositionLocalProvider(LocalMessageSurface provides MaterialTheme.colorScheme.primary,LocalRecipeScale provides null) {
            Box(Modifier.padding(horizontal=14.dp,vertical=10.dp)) {MessageCards(message,{""},null)}
        }
    }
}
