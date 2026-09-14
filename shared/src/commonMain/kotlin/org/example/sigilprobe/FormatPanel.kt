package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.input.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.*
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.unit.dp

private val effects=listOf("shake","wave","pulse","glow","typewriter","sparkle","glitch","scatter","flip","barrel")
@OptIn(ExperimentalLayoutApi::class)
@Composable internal fun FormatPanel(draft:TextFieldState,showSource:Boolean,setSource:(Boolean)->Unit,back:()->Unit,code:()->Unit,write:()->Unit) {
    var section by rememberSaveable {mutableStateOf("Style")}
    var effect by rememberSaveable {mutableStateOf("wave")}
    var sample by rememberSaveable {mutableStateOf("A little everyday magic")}
    var color by rememberSaveable {mutableStateOf("purple")}
    var second by rememberSaveable {mutableStateOf("pink")}
    var gradient by rememberSaveable {mutableStateOf(false)}
    var highlight by rememberSaveable {mutableStateOf(false)}
    var size by rememberSaveable {mutableIntStateOf(2)}
    val previewer=LocalStructuredPreview.current
    var preview by remember {mutableStateOf<MessagePart?>(null)}
    val sampleSource="$effect::${escapeField(sample)};"
    val clock=remember {TextPlayback()}
    LaunchedEffect(sampleSource,previewer) {kotlinx.coroutines.delay(120);preview=kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.Default){previewer?.invoke(sampleSource)};clock.replay()}
    fun apply(name:String) {draft.edit {val range=selection;insert(range.max,";");insert(range.min,"$name::");selection=TextRange(range.min+name.length+2,range.max+name.length+2)}}
    Column(Modifier.fillMaxSize().padding(horizontal=16.dp,vertical=8.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment=Alignment.CenterVertically) {Symbol("chevron_left","Back to attachments",back);Text("Formatting",Modifier.weight(1f),style=MaterialTheme.typography.titleLarge);Symbol("keyboard","Continue writing",write)}
        Row(Modifier.horizontalScroll(rememberScrollState()),horizontalArrangement=Arrangement.spacedBy(8.dp)) {listOf("Style","Color","Effects").forEach {name->FilterChip(section==name,{section=name},label={Text(name)},shape=RoundedCornerShape(12.dp))}}
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(8.dp)) {
            Text(if(draft.selection.collapsed)"Choose a format, then write inside it." else "Apply to the selected text.",style=MaterialTheme.typography.bodySmall)
            when(section) {
                "Style" -> {
                    FlowRow(horizontalArrangement=Arrangement.spacedBy(4.dp)) {listOf("Bold" to "bold","Italic" to "italic","Underline" to "underline","Strike" to "strike","Monospace" to "mono","Spoiler" to "spoiler","Scratch" to "scratch","Redact" to "redact").forEach {(label,name)->SigilTextButton({apply(name)}){Text(label)}}}
                    Row(verticalAlignment=Alignment.CenterVertically) {Text("Size",Modifier.weight(1f));(1..3).forEach {n->FilterChip(size==n,{size=n},label={Text("$n")},shape=RoundedCornerShape(12.dp))}}
                    Row {SigilTextButton({apply("small$size")}){Text("Smaller")};SigilTextButton({apply("big$size")}){Text("Larger")};SigilTextButton(code){Text("Code block")}}
                    Toggle("Show formatting syntax",showSource,setSource)
                }
                "Color" -> {
                    FlowRow(horizontalArrangement=Arrangement.spacedBy(6.dp)) {listOf("red","orange","yellow","green","cyan","blue","purple","pink","gray","rainbow").forEach {name->FilterChip(color==name,{color=name},label={Text(name.replaceFirstChar {it.uppercase()})},shape=RoundedCornerShape(12.dp))}}
                    Toggle("Gradient",gradient){gradient=it};Expandable(gradient) {FlowRow {listOf("red","orange","yellow","green","cyan","blue","purple","pink","gray").forEach {name->FilterChip(second==name,{second=name},label={Text(name)},shape=RoundedCornerShape(12.dp))}}}
                    Toggle("Highlight background",highlight){highlight=it}
                    SigilButton({apply((if(highlight)"mark::" else "")+color+if(gradient && color!="rainbow")"-$second" else "")}){Text("Apply color")}
                }
                else -> {
                    FlowRow(horizontalArrangement=Arrangement.spacedBy(6.dp)) {effects.forEach {name->FilterChip(effect==name,{effect=name},label={Text(name.replaceFirstChar {it.uppercase()})},shape=RoundedCornerShape(12.dp))}}
                    FormField("Try some text",sample,{sample=it})
                    preview?.let {p->MessageMotion("effect-preview",clock,true,p.rich?.motion?.maxOfOrNull {it.duration} ?: 2000) {BuilderPreview(p)}}
                    Row {SigilTextButton({clock.replay()}){Text("Replay")};SigilButton({apply(effect)}){Text("Apply effect")}}
                }
            }
        }
    }
}
