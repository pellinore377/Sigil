package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.*
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import kotlinx.serialization.json.*

class ServiceResponse(val json:String,val preview:MessagePart?=null)
val LocalServiceAccess=staticCompositionLocalOf<(suspend (String)->ServiceResponse)?> {null}
val LocalDraftId=staticCompositionLocalOf<(() -> String)?> {null}
@Composable internal fun ServiceBuilder(kind:String,back:()->Unit,discard:(String)->Unit,stage:(String,MessagePart,Boolean)->Unit) {
    val access=LocalServiceAccess.current;val newId=LocalDraftId.current;val scope=rememberCoroutineScope()
    var catalog by remember {mutableStateOf<JsonObject?>(null)}
    var provider by rememberSaveable {mutableStateOf("")}
    var text by rememberSaveable {mutableStateOf("")}
    var language by rememberSaveable {mutableStateOf("en")}
    var forecast by rememberSaveable {mutableStateOf(false)}
    var place by remember {mutableStateOf<JsonObject?>(null)}
    var places by remember {mutableStateOf(emptyList<JsonObject>())}
    var busy by remember {mutableStateOf(false)}
    var issue by remember {mutableStateOf<String?>(null)}
    var retry by remember {mutableIntStateOf(0)}
    var pending by remember {mutableStateOf<Pair<String,String>?>(null)}
    var transferred by remember {mutableStateOf(false)}
    DisposableEffect(Unit) {onDispose {if(!transferred)pending?.second?.let(discard)}}
    LaunchedEffect(access,retry) {if(access!=null && kind!="Contact") {busy=true;try {catalog=Json.parseToJsonElement(access("{\"command\":\"service\",\"action\":\"catalog\"}").json).jsonObject["catalog"]?.jsonObject;issue=null} catch(e:kotlinx.coroutines.CancellationException){throw e} catch(_:Exception){issue="Could not load providers. Try again."};busy=false}}
    val locating=kind=="Weather" && place==null
    val eligible=when {locating->listOf("geocoder");kind=="Weather"->listOf("open_meteo");kind=="Translation"->listOf("google_translate","libre_translate");else->listOf("wiktionary","dictionary_index")}
    val providers=catalog?.get("providers")?.jsonArray?.map {it.jsonObject}?.filter {it["kind"]?.jsonPrimitive?.content in eligible}.orEmpty()
    val selected=providers.firstOrNull {it["id"]?.jsonPrimitive?.content==provider} ?: providers.firstOrNull()
    Column(Modifier.fillMaxSize().padding(horizontal=20.dp,vertical=8.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment=Alignment.CenterVertically) {Symbol("chevron_left","Back to create",back);Text(kind,Modifier.weight(1f),style=MaterialTheme.typography.titleLarge)}
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(10.dp)) {
            if(access==null)Text("Connect to your account to use configured providers.")
            else if(kind!="Contact" && catalog!=null && providers.isEmpty())Text(if(locating)"Your server needs an address lookup provider to find a place." else "Your server has no provider configured for this tool.")
            else {
                providers.forEach {p->val name=p.getValue("id").jsonPrimitive.content;FilterChip(selected==p,{provider=name},label={Text(name)},shape=RoundedCornerShape(12.dp))}
                if(place!=null) {Text(place?.get("name")?.jsonPrimitive?.content.orEmpty());SigilTextButton({place=null;places=emptyList()}){Text("Change place")};Toggle("Include forecast",forecast){forecast=it}}
                else FormField(if(kind=="Contact")"Account address" else if(locating)"Place" else if(kind=="Definition")"Word" else "Text",text,{text=it},multiline=kind=="Translation")
                if(kind!="Weather" && kind!="Contact")FormField(if(kind=="Translation")"Translate to (language code)" else "Language code",language,{language=it})
                selected?.let {Text("This lookup sends your query through your server to ${it.getValue("endpoint").jsonPrimitive.content}. The resulting card is encrypted when sent to the conversation.",style=MaterialTheme.typography.bodySmall)}
                places.forEach {p->SigilTextButton({place=p;places=emptyList();provider=""}){Text(listOfNotNull(p["name"]?.jsonPrimitive?.content,p["region"]?.jsonPrimitive?.content,p["country"]?.jsonPrimitive?.content).joinToString(", "))}}
            }
            issue?.let {Text(it,color=MaterialTheme.colorScheme.error);SigilTextButton({retry++}){Text("Reload providers")}}
        }
        SigilButton({scope.launch {
            if(kind=="Contact") {busy=true;try {val result=access!!.invoke(buildJsonObject {put("command","contact_preview");put("address",text.trim())}.toString());result.preview?.let {stage(Json.parseToJsonElement(result.json).jsonObject.getValue("contact").toString(),it,true)}} catch(e:kotlinx.coroutines.CancellationException){throw e} catch(_:Exception){issue="Could not find this account. Check the full address and discovery settings."} finally {busy=false};return@launch}
            val form=buildJsonObject {put("kind",if(locating)"Locate" else kind);put("text",text);put("language",language);put("forecast",forecast);place?.let {put("place",it)}}
            val definition=buildJsonObject {put("command","service");put("action","resolve");put("catalog",catalog!!);put("provider",selected!!.getValue("id"));put("form",form)}
            val fingerprint=definition.toString()
            val request=pending?.takeIf {it.first==fingerprint}?.second ?: newId!!.invoke().also {pending?.second?.let(discard);pending=fingerprint to it}
            busy=true;issue=null
            try {
                val result=access!!.invoke(JsonObject(definition+mapOf("request" to JsonPrimitive(request))).toString())
                val data=Json.parseToJsonElement(result.json).jsonObject
                if(locating) {places=data.getValue("places").jsonArray.map {it.jsonObject};if(places.isEmpty())issue="No places found. Try a more specific name.";access.invoke("{\"command\":\"service\",\"action\":\"discard\",\"request\":\"$request\"}");pending=null}
                else result.preview?.let {transferred=true;stage(request,it,false)} ?: run {issue="The provider returned no usable card."}
            } catch(e:kotlinx.coroutines.CancellationException){throw e} catch(_:Exception){issue="The lookup failed. Your text is kept; try again."} finally {busy=false}
        }},Modifier.fillMaxWidth(),enabled=!busy && (selected!=null || kind=="Contact" && access!=null) && newId!=null && (place!=null || text.isNotBlank())) {if(busy)Text("Working…")else Text(if(locating)"Find place" else "Look up and add")}
    }
}
