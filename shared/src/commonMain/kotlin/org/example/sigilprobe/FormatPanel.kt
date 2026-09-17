package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.input.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.*
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.*
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.unit.dp

private val effects=listOf("shake","wave","pulse","glow","typewriter","sparkle","glitch","scatter","assemble","flip","barrel")

@Composable
private fun FormatSymbol(icon:String,label:String,armed:Boolean,action:()->Unit) {
    val scheme=MaterialTheme.colorScheme
    Surface(action,Modifier.semantics {role=Role.Button;contentDescription=label;toggleableState=if(armed)ToggleableState.On else ToggleableState.Off},
        shape=SigilButtonShape,color=if(armed)scheme.primaryContainer else Color.Transparent,contentColor=if(armed)scheme.onPrimaryContainer else LocalContentColor.current) {
        Box(Modifier.size(48.dp),contentAlignment=Alignment.Center) {Glyph(icon)}
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable internal fun FormatPanel(draft:TextFieldState,analyze:(String)->String,showSource:Boolean,setSource:(Boolean)->Unit,back:()->Unit,code:()->Unit,write:()->Unit) {
    var menu by remember {mutableStateOf("")}
    val editorAnalysis=LocalEditorAnalysis.current ?: analyze
    val source=draft.text.toString()
    val formats=remember(source,editorAnalysis) {spans(editorAnalysis(source))}
    val active=activeFormats(formats,draft.selection.start)
    var color by rememberSaveable {mutableStateOf("gray")}
    var second by rememberSaveable {mutableStateOf("blue")}
    var gradient by rememberSaveable {mutableStateOf(false)}
    var highlight by rememberSaveable {mutableStateOf(false)}
    fun apply(name:String) {draft.edit {val range=selection;insert(range.max,";");insert(range.min,"$name::");selection=TextRange(range.min+name.length+2,range.max+name.length+2)};write()}
    Column(Modifier.fillMaxWidth().padding(start=8.dp,end=8.dp,top=8.dp)) {
        Row(Modifier.fillMaxWidth(),verticalAlignment=Alignment.CenterVertically) {
            Symbol("chevron_left","Back to attachments",back)
            listOf(Triple("format_bold","Bold","bold"),Triple("format_italic","Italic","italic"),Triple("format_underlined","Underline","underline"),Triple("strikethrough_s","Strike","strike"),Triple("code","Monospace","mono"),Triple("visibility_off","Spoiler","spoiler")).forEach {(icon,label,name)->FormatSymbol(icon,label,(if(name=="mono")"code" else name) in active){apply(name)}}
        }
        Row(Modifier.fillMaxWidth(),verticalAlignment=Alignment.CenterVertically) {
            Box {
                FormatSymbol("format_color_text","Text color","color" in active || "highlight" in active){menu="color"}
                DropdownMenu(menu=="color",{menu=""}) {
                    Column(Modifier.widthIn(max=300.dp).padding(horizontal=12.dp,vertical=8.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                        FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {listOf("red","orange","yellow","green","cyan","blue","purple","pink","gray","rainbow").forEach {name->FilterChip(color==name,{color=name},label={Text(name.replaceFirstChar {it.uppercase()})},shape=RoundedCornerShape(12.dp))}}
                        Toggle("Gradient",gradient){gradient=it}
                        if(gradient && color!="rainbow")FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {listOf("red","orange","yellow","green","blue","purple","pink","gray").forEach {name->FilterChip(second==name,{second=name},label={Text(name.replaceFirstChar {it.uppercase()})},shape=RoundedCornerShape(12.dp))}}
                        Toggle("Highlight",highlight){highlight=it}
                        SigilTextButton({apply((if(highlight)"mark::" else "")+color+if(gradient && color!="rainbow")"-$second" else "");menu=""}) {Text("Apply color")}
                    }
                }
            }
            Box {
                FormatSymbol("animation","Text effect","animation" in active){menu="effects"}
                DropdownMenu(menu=="effects",{menu=""}) {effects.forEach {effect->DropdownMenuItem(text={Text(effect.replaceFirstChar {it.uppercase()})},onClick={apply(effect);menu=""})}}
            }
            Box {
                FormatSymbol("format_size","Text size","size" in active){menu="size"}
                DropdownMenu(menu=="size",{menu=""}) {
                    (1..3).forEach {size->DropdownMenuItem(text={Text("Smaller $size")},onClick={apply("small$size");menu=""})}
                    (1..3).forEach {size->DropdownMenuItem(text={Text("Larger $size")},onClick={apply("big$size");menu=""})}
                }
            }
            Box {
                Symbol("more_horiz","More formatting"){menu="more"}
                DropdownMenu(menu=="more",{menu=""}) {
                    DropdownMenuItem(text={Text("Scratch")},leadingIcon={Glyph("ink_eraser",20)},onClick={apply("scratch");menu=""})
                    DropdownMenuItem(text={Text("Redact")},leadingIcon={Glyph("block",20)},onClick={apply("redact");menu=""})
                    DropdownMenuItem(text={Text("Code block")},leadingIcon={Glyph("data_object",20)},onClick={menu="";code()})
                    DropdownMenuItem(text={Text("Show formatting syntax")},leadingIcon={Glyph(if(showSource)"code_off" else "code",20)},onClick={menu="";setSource(!showSource);write()})
                    DropdownMenuItem(text={Text("Continue writing")},leadingIcon={Glyph("keyboard",20)},onClick={menu="";write()})
                }
            }
        }
    }
}
