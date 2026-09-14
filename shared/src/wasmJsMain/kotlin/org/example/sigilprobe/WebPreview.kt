@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)
package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.unit.dp
import kotlinx.browser.window
import kotlinx.serialization.json.*
import kotlin.js.*

@JsName("Date") private external class WebDate(value:Double) : JsAny {
    fun toLocaleString():String
    companion object {fun now():Double}
}
private fun context(source:String,timezone:String)=buildJsonObject {put("source",source);put("now",(WebDate.now()/1000).toLong());put("timezone",timezone)}.toString()
private fun preview(source:String)=runCatching {ContentDecoder.part(rustPreview(context(source,"UTC"))) {WebDate(it*1000.0).toLocaleString()}}.getOrNull()

@Composable internal fun WebPreview() {
    var narrow by remember {mutableStateOf(true)}
    var dark by remember {mutableStateOf(false)}
    var sans by remember {mutableStateOf(false)}
    var error by remember {mutableStateOf("")}
    var sent by remember {mutableStateOf(0L)}
    var sentText by remember {mutableStateOf<String?>(null)}
    var messages by remember {mutableStateOf(emptyList<ChatMessage>())}
    var preferences by remember {mutableStateOf(emptyMap<String,String>())}
    val appearance="${if(sans)"Google Sans Flex" else "Newsreader"}|${if(dark)"Dark" else "Light"}|555555|false"
    val chat=ChatSummary("preview","Preview","","",true,emptyList())
    val command:Command={name,fields->
        error=""
        when(name) {
            "post"->{
                val source=fields["text"] as? String ?: ""
                val parts=runCatching {Json.parseToJsonElement(rustPlayground(context(source,fields["timezone"] as? String ?: "UTC"))).jsonArray.mapIndexed {i,p->ContentDecoder.part(p.toString()) {WebDate(it*1000.0).toLocaleString()}.copy(id="part-$i")}}.getOrNull()
                if(parts==null)error="This content could not be previewed. Your draft is preserved."
                else {sent++;sentText=source;messages=listOf(ChatMessage("preview-$sent","local","",true,"","sent",false,emptyList(),emptyList(),null,true,timestamp=(WebDate.now()/1000).toLong(),parts=parts))+messages.take(49)}
            }
            "record_start","attachment_pick"->error="Capture and file transfers need the browser messaging adapter. This workbench previews text and cards locally."
            "call_start","call_prepare"->error="Calls are unavailable in the design workbench."
        }
    }
    CompositionLocalProvider(LocalBuilderSource provides ::rustBuilder,LocalStructuredPreview provides ::preview,LocalBuilderTimezone provides "UTC",LocalCodePreview provides ::rustCode,
        LocalEditorAnalysis provides ::rustEditor,LocalHelpCatalog provides ::rustHelp,LocalTextMotionSeeds provides ::rustMotionSeeds,
        LocalTemporalPreview provides {kind,input->
            val result=rustTemporal("$kind\n${(WebDate.now()/1000).toLong()}\nUTC\nday\n$input").split('\n')
            if(result.size!=3)null else result[1].toLongOrNull()?.let {TemporalPreview(result[0],"UTC",if(kind=="Timer")"$it seconds" else WebDate(it*1000.0).toLocaleString()+" · UTC")}
        }) {
        SigilTheme(decodeAppearance(appearance),palette=::rustPalette) {
            Surface(Modifier.fillMaxSize(),color=MaterialTheme.colorScheme.surfaceVariant) {
                Column {
                    Row(Modifier.fillMaxWidth().padding(horizontal=20.dp,vertical=12.dp),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                        Column(Modifier.weight(1f)) {Text("Sigil · Design workbench",style=MaterialTheme.typography.titleMedium);Text("Local previews only · nothing is sent or saved",style=MaterialTheme.typography.bodySmall)}
                        Symbol(if(narrow)"desktop_windows" else "smartphone",if(narrow)"Wide layout" else "Phone layout") {narrow=!narrow}
                        Symbol("text_fields","Switch font") {sans=!sans}
                        Symbol(if(dark)"light_mode" else "dark_mode","Switch appearance") {dark=!dark}
                        Symbol("delete","Clear previews") {messages=emptyList()}
                    }
                    if(error.isNotEmpty())Row(Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.errorContainer).padding(horizontal=20.dp),verticalAlignment=Alignment.CenterVertically) {Text(error,Modifier.weight(1f),color=MaterialTheme.colorScheme.onErrorContainer,style=MaterialTheme.typography.bodyMedium);Symbol("close","Dismiss notice") {error=""}}
                    Box(Modifier.weight(1f).fillMaxWidth(),contentAlignment=Alignment.TopCenter) {
                        Box(Modifier.fillMaxHeight().widthIn(max=if(narrow)440.dp else 1120.dp).fillMaxWidth()) {
                            SigilApp(::rustPalette,::rustAnalyze,MessengerState(phase="connected",chats=listOf(chat),selected="preview",timelineLoaded=true,messages=messages,sent=sent,sentText=sentText,ui=mapOf("appearance" to appearance)),command,
                                read={preferences[it]},write={key,value->preferences=preferences+(key to value)})
                        }
                    }
                }
            }
        }
    }
}
