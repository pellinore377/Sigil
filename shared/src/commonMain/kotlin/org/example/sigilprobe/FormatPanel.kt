package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
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
    var menu by remember {mutableStateOf("")}
    var color by rememberSaveable {mutableStateOf("gray")}
    var second by rememberSaveable {mutableStateOf("blue")}
    var gradient by rememberSaveable {mutableStateOf(false)}
    var highlight by rememberSaveable {mutableStateOf(false)}
    fun apply(name:String) {draft.edit {val range=selection;insert(range.max,";");insert(range.min,"$name::");selection=TextRange(range.min+name.length+2,range.max+name.length+2)};write()}
    Column(Modifier.fillMaxWidth().padding(start=8.dp,end=8.dp,top=8.dp)) {
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),verticalAlignment=Alignment.CenterVertically) {
            Symbol("chevron_left","Back to attachments",back)
            listOf(Triple("format_bold","Bold","bold"),Triple("format_italic","Italic","italic"),Triple("format_underlined","Underline","underline"),Triple("strikethrough_s","Strike","strike"),Triple("code","Monospace","mono"),Triple("visibility_off","Spoiler","spoiler"),Triple("ink_eraser","Scratch","scratch"),Triple("block","Redact","redact")).forEach {(icon,label,name)->Symbol(icon,label){apply(name)}}
        }
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),verticalAlignment=Alignment.CenterVertically) {
            Box {
                Symbol("format_color_text","Text color"){menu="color"}
                DropdownMenu(menu=="color",{menu=""}) {
                    Column(Modifier.widthIn(max=300.dp).padding(horizontal=12.dp)) {
                        FlowRow(horizontalArrangement=Arrangement.spacedBy(4.dp)) {listOf("red","orange","yellow","green","cyan","blue","purple","pink","gray","rainbow").forEach {name->FilterChip(color==name,{color=name},label={Text(name.replaceFirstChar {it.uppercase()})})}}
                        Toggle("Gradient",gradient){gradient=it}
                        if(gradient && color!="rainbow")FlowRow(horizontalArrangement=Arrangement.spacedBy(4.dp)) {listOf("red","orange","yellow","green","blue","purple","pink","gray").forEach {name->FilterChip(second==name,{second=name},label={Text(name.replaceFirstChar {it.uppercase()})})}}
                        Toggle("Highlight",highlight){highlight=it}
                        SigilTextButton({apply((if(highlight)"mark::" else "")+color+if(gradient && color!="rainbow")"-$second" else "");menu=""}) {Text("Apply color")}
                    }
                }
            }
            Box {
                Symbol("animation","Text effect"){menu="effects"}
                DropdownMenu(menu=="effects",{menu=""}) {effects.forEach {effect->DropdownMenuItem(text={Text(effect.replaceFirstChar {it.uppercase()})},onClick={apply(effect);menu=""})}}
            }
            Box {
                Symbol("format_size","Text size"){menu="size"}
                DropdownMenu(menu=="size",{menu=""}) {
                    (1..3).forEach {size->DropdownMenuItem(text={Text("Smaller $size")},onClick={apply("small$size");menu=""})}
                    (1..3).forEach {size->DropdownMenuItem(text={Text("Larger $size")},onClick={apply("big$size");menu=""})}
                }
            }
            Symbol("data_object","Code block",code)
            Symbol(if(showSource)"code_off" else "code","Show formatting syntax"){setSource(!showSource);write()}
            Symbol("keyboard","Continue writing",write)
        }
    }
}
