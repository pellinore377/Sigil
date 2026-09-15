package org.sigil

import androidx.compose.animation.*
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.*
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp

val LocalCodePreview=staticCompositionLocalOf<((String)->String)?> {null}

internal fun codePreview(value:String):Pair<String,RichText>? {
    val parts=value.split('\n',limit=3)
    if(parts.size!=3 || parts[2].isEmpty())return null
    val tokens=parts[1].split(';').filter {it.isNotEmpty()}.map {item->
        val fields=item.split(',')
        if(fields.size!=3)return null
        val start=fields[0].toIntOrNull() ?: return null
        val end=fields[1].toIntOrNull() ?: return null
        if(start<0 || end>parts[2].length || start>=end)return null
        CodeToken(start,end,fields[2])
    }
    return parts[0] to RichText(parts[2],codeTokens=tokens)
}

@Composable
internal fun CodeBuilder(enabled:Boolean,back:()->Unit,send:(String)->Unit) {
    var code by rememberSaveable {mutableStateOf("")}
    var language by rememberSaveable {mutableStateOf("")}
    var custom by rememberSaveable {mutableStateOf(false)}
    var showingPreview by rememberSaveable {mutableStateOf(false)}
    var previewInput by rememberSaveable {mutableStateOf<String?>(null)}
    var languages by remember {mutableStateOf(false)}
    var tooLarge by remember {mutableStateOf(false)}
    val input="Code\n$language\n$code"
    val resolve=LocalBuilderSource.current
    val render=LocalCodePreview.current
    val source=remember(previewInput,resolve) {previewInput?.let {resolve?.invoke(it)}.orEmpty()}
    val preview=remember(previewInput,render) {previewInput?.let {render?.invoke(it)}?.let(::codePreview)}
    val motion=LocalMotion.current
    val keyboard=LocalSoftwareKeyboardController.current
    val focus=LocalFocusManager.current
    val choices=listOf("Plain text" to "","Rust" to "rust","Kotlin" to "kotlin","C" to "c","C++" to "cpp","JSON" to "json")
    fun previous() {if(showingPreview)showingPreview=false else back()}
    BackAction(showingPreview,::previous)
    val sizing=rememberBuilderSizing(16.dp)
    Column(Modifier.fillMaxSize().padding(start=8.dp,end=8.dp,top=8.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        Row(sizing.measure("header"),verticalAlignment=Alignment.CenterVertically) {
            Symbol("chevron_left",if(showingPreview)"Edit code" else "Back to formatting",::previous)
            Text("Code block",Modifier.weight(1f),style=MaterialTheme.typography.titleLarge)
        }
        if(showingPreview)BuilderConfirm(if(LocalBuilderAction.current=="Send")"Send code" else LocalBuilderAction.current,enabled && preview!=null && source.isNotEmpty()) {send(source)}
        else BuilderConfirm("Preview code",code.isNotBlank()) {focus.clearFocus();keyboard?.hide();previewInput=input;showingPreview=true}
        AnimatedContent(showingPreview,Modifier.weight(1f).verticalScroll(rememberScrollState()).wrapContentHeight(unbounded=true).then(sizing.measure("body")),transitionSpec={
            (slideInHorizontally(motion.tween(MotionMillis)){if(targetState)it else -it}+fadeIn(motion.tween(MotionMillis))) togetherWith
                (slideOutHorizontally(motion.tween(MotionMillis)){if(targetState)-it else it}+fadeOut(motion.tween(MotionMillis)))
        },label="Code form") {shown->
            if(shown)Column(Modifier.fillMaxWidth(),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                Column(Modifier.fillMaxWidth(),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                    if(preview!=null)Surface(shape=RoundedCornerShape(20.dp),color=MaterialTheme.colorScheme.primary) {
                        CompositionLocalProvider(LocalMessageSurface provides MaterialTheme.colorScheme.primary) {
                            Box(Modifier.fillMaxWidth().padding(12.dp),contentAlignment=Alignment.Center) {CodeBlock(preview.second,preview.first)}
                        }
                    } else Text(if(resolve==null || render==null)"The code builder is unavailable." else "Check the language name and code. Language names use letters, numbers, hyphens or underscores.",color=MaterialTheme.colorScheme.error,style=MaterialTheme.typography.bodyMedium)
                }

            } else Column(Modifier.fillMaxWidth(),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                Row(verticalAlignment=Alignment.CenterVertically) {
                    Box {
                        SigilTextButton({languages=true}) {Text(if(custom)"Custom language" else choices.firstOrNull {it.second==language}?.first ?: "Language");Glyph("expand_more",20)}
                        DropdownMenu(languages,{languages=false}) {
                            choices.forEach {(label,id)->DropdownMenuItem(text={Text(label)},onClick={language=id;custom=false;languages=false})}
                            DropdownMenuItem(text={Text("Other language")},onClick={custom=true;language="";languages=false})
                        }
                    }
                    if(custom)OutlinedTextField(language,{language=it.replace('\n',' ').replace('\r',' ').take(32)},Modifier.weight(1f),shape=RoundedCornerShape(16.dp),singleLine=true,label={Text("Language")},keyboardOptions=KeyboardOptions(autoCorrectEnabled=false))
                }
                if(tooLarge)Text("That edit is too large. Shorten the code and try again.",color=MaterialTheme.colorScheme.error,style=MaterialTheme.typography.bodySmall)
                OutlinedTextField(code,{value->
                    val normalized=value.replace("\r\n","\n").replace('\r','\n')
                    if(normalized.length>16300 || normalized.encodeToByteArray().size>16300)tooLarge=true else {code=normalized;tooLarge=false}
                },Modifier.fillMaxWidth(),minLines=4,maxLines=10,shape=RoundedCornerShape(16.dp),label={Text("Code")},textStyle=MaterialTheme.typography.bodyMedium.copy(fontFamily=LocalCodeFont.current),keyboardOptions=KeyboardOptions(capitalization=KeyboardCapitalization.None,autoCorrectEnabled=false))

            }
        }
    }
}
