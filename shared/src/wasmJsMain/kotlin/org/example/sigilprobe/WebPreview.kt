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
private fun seeded(query:String):List<ChatMessage> {
    val raw=query.removePrefix("?").split('&').firstOrNull {it.startsWith("seed=")}?.removePrefix("seed=") ?: return emptyList()
    val json=runCatching {kotlin.io.encoding.Base64.UrlSafe.withPadding(kotlin.io.encoding.Base64.PaddingOption.ABSENT_OPTIONAL).decode(raw).decodeToString()}.getOrNull() ?: return emptyList()
    // "reply" or "thread" names another seed's index, quoted as the core would quote it.
    return runCatching {
        val items=Json.parseToJsonElement(json).jsonArray
        val built=items.mapIndexed {i,item->
            val source=item.jsonObject["text"]?.jsonPrimitive?.content.orEmpty()
            val mine=item.jsonObject["mine"]?.jsonPrimitive?.booleanOrNull ?: true
            val parts=previewService(source)?.let {listOf(MessagePart("seed-$i-0","service","",service=it))} ?: Json.parseToJsonElement(rustPlayground(context(source,"UTC"))).jsonArray.mapIndexed {j,p->ContentDecoder.part(p.toString()) {WebDate(it*1000.0).toLocaleString()}.copy(id="seed-$i-$j")}
            ChatMessage("seed-$i",if(mine)"local" else "maya",source,mine,"","sent",false,emptyList(),emptyList(),null,true,timestamp=(WebDate.now()/1000).toLong()-60*i,parts=parts,peer="preview")
        }
        built.mapIndexed {i,m->
            fun target(key:String)=items[i].jsonObject[key]?.jsonPrimitive?.intOrNull?.takeIf {it in built.indices}?.let {built[it].copy(text=items[it].jsonObject["text"]?.jsonPrimitive?.content.orEmpty())}
            var out=m
            target("reply")?.let {r->out=out.copy(reply=r.text,replyAuthor=r.author,replyMine=r.mine,replyMessage=r.id,replyParts=r.parts)}
            target("thread")?.let {r->out=out.copy(threadAuthor=r.author,threadMessage=r.id,threadPreview=r.text,threadParts=r.parts)}
            out
        }
    }.getOrDefault(emptyList())
}
private fun preview(source:String)=runCatching {ContentDecoder.part(rustPreview(context(source,"UTC"))) {WebDate(it*1000.0).toLocaleString()}}.getOrNull()

@Composable internal fun WebPreview() {
    var materialsReady by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) { try { initializeMaterialWasm().awaitBrowser<kotlin.js.JsAny?>(); materialsReady = initializeMaterialGpu().awaitBrowser<kotlin.js.JsBoolean>().toBoolean() } catch (_: Exception) {} }
    DisposableEffect(Unit) { onDispose { runCatching { browserMaterialShutdown() } } }
    // Automated design reviews preload messages: ?dark=1&seed=<base64url JSON [{"text":…,"mine":bool}]>.
    val query=remember {window.location.search}
    var narrow by remember {mutableStateOf(true)}
    var dark by remember {mutableStateOf(query.contains("dark=1"))}
    var sans by remember {mutableStateOf(false)}
    var error by remember {mutableStateOf("")}
    var sent by remember {mutableStateOf(0L)}
    var sentText by remember {mutableStateOf<String?>(null)}
    var messages by remember {mutableStateOf(seeded(query))}
    var selected by remember {mutableStateOf<String?>("preview")}
    var category by remember {mutableStateOf("Timeline")}
    var thread by remember {mutableStateOf<ThreadTarget?>(null)}
    var preferences by remember {mutableStateOf(emptyMap<String,String>())}
    val appearance="${if(sans)"Google Sans Flex" else "Newsreader"}|${if(dark)"Dark" else "Light"}|555555|false"
    val chat=ChatSummary("preview","Maya Chen","","",true,emptyList(), presence = "active")
    val command:Command={name,fields->
        error=""
        when(name) {
            "open" -> { selected = fields["peer"] as? String; category = "Timeline"; thread = null }
            "close" -> { selected = null; category = "Timeline"; thread = null }
            "timeline_filter" -> { category = fields["category"] as? String ?: "Timeline"; thread = (fields["thread_message"] as? String)?.let { ThreadTarget(fields["thread_author"] as? String ?: "local", it) } }
            "pin", "note" -> messages = messages.map { message -> if (message.id == fields["message"]) { if (name == "pin") message.copy(pinned = fields["active"] == true) else message.copy(noted = fields["active"] == true) } else message }
            "delete" -> messages = messages.filterNot { it.id == fields["message"] }
            "post"->{
                val source=fields["text"] as? String ?: ""
                val parts=runCatching {Json.parseToJsonElement(rustPlayground(context(source,fields["timezone"] as? String ?: "UTC"))).jsonArray.mapIndexed {i,p->ContentDecoder.part(p.toString()) {WebDate(it*1000.0).toLocaleString()}.copy(id="part-$i")}}.getOrNull()
                if(parts==null)error="This content could not be previewed. Your draft is preserved."
                else {sent++;sentText=source;messages=listOf(ChatMessage("preview-$sent","local","",true,"","sent",false,emptyList(),emptyList(),null,true,timestamp=(WebDate.now()/1000).toLong(),parts=parts, peer="preview", threadAuthor=fields["thread_author"] as? String, threadMessage=fields["thread_message"] as? String, threadPreview=messages.firstOrNull { it.id == fields["thread_message"] }?.text).let {m->messages.firstOrNull {it.id==fields["reply_message"]}?.let {r->m.copy(reply=r.text,replyAuthor=r.author,replyMine=r.mine,replyMessage=r.id,replyParts=r.parts)} ?: m})+messages.take(49)}
            }
            "record_start","attachment_pick"->error="Capture and file transfers need the browser messaging adapter. This workbench previews text and cards locally."
            "call_start","call_prepare"->error="Calls are unavailable in the design workbench."
        }
    }
    CompositionLocalProvider(LocalMaterialPlatform provides WebMaterials, LocalSolidMaterial provides (if (materialsReady) { value, progress, modifier -> MaterialMessages(value, progress, modifier) } else null), LocalMaterialOverlay provides (if (materialsReady) { timeline, modifier -> MaterialTimelineOverlay(timeline, modifier) } else null), LocalRecipeScale provides ::previewRecipeScale,LocalBuilderSource provides ::rustBuilder,LocalStructuredPreview provides ::preview,LocalBuilderTimezone provides "UTC",LocalCodePreview provides ::rustCode,
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
                            SigilApp(::rustPalette,::rustAnalyze,MessengerState(phase="connected",chats=listOf(chat.copy(preview=messages.firstOrNull()?.text.orEmpty())),selected=selected,timelineLoaded=true,messages=messages.filter { message -> when { thread != null -> message.threadMessage == thread?.id && message.threadAuthor == thread?.author; category == "Notes" -> message.noted || message.parts.any { it.kind == "note" }; category == "Pins" -> message.pinned; category == "Threads" -> message.threadMessage != null; else -> true } },sent=sent,sentText=sentText,sentMessage="preview-$sent",ui=mapOf("appearance" to appearance)),command,wideLayout=!narrow,
                                read={preferences[it]},write={key,value->preferences=preferences+(key to value)})
                        }
                    }
                }
            }
        }
    }
}
