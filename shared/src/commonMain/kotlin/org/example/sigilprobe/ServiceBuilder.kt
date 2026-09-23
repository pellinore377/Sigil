package org.sigil

import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import kotlinx.serialization.json.*

class ServiceResponse(val json:String,val preview:MessagePart?=null)
val LocalServiceAccess=staticCompositionLocalOf<(suspend (String)->ServiceResponse)?> {null}
val LocalDraftId=staticCompositionLocalOf<(() -> String)?> {null}
@Composable internal fun ServiceBuilder(kind:String,back:()->Unit,discard:(String)->Unit,initial:PreviewIntent?=null,stage:(String,MessagePart,Boolean)->Unit) {
    val access=LocalServiceAccess.current;val newId=LocalDraftId.current;val scope=rememberCoroutineScope()
    var catalog by remember {mutableStateOf<JsonObject?>(null)}
    var provider by rememberSaveable {mutableStateOf("")}
    var text by rememberSaveable(kind,initial) {mutableStateOf(initial?.text.orEmpty())}
    var language by rememberSaveable(kind,initial) {mutableStateOf(initial?.language?.takeIf {it.isNotEmpty()} ?: androidx.compose.ui.text.intl.Locale.current.language.takeIf {it.isNotEmpty()} ?: "en")}
    var forecast by rememberSaveable(kind,initial) {mutableStateOf(initial?.forecast ?: false)}
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
    val loadingCatalog=kind!="Contact" && catalog==null && busy
    val motion=LocalMotion.current
    val sizing=rememberBuilderSizing(16.dp)
    Column(Modifier.fillMaxSize().padding(start=8.dp,end=8.dp,top=8.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        Row(sizing.measure("header"),verticalAlignment=Alignment.CenterVertically) {Symbol("chevron_left","Back to create",back);Text(kind,Modifier.weight(1f),style=MaterialTheme.typography.titleLarge)}
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).wrapContentHeight(unbounded=true).then(sizing.measure("body")).animateContentSize(motion.tween(MotionMillis)),verticalArrangement=Arrangement.spacedBy(10.dp)) {
            if(access==null)BuilderNotice("Connect to your account to use configured providers.")
            else if(loadingCatalog)BuilderProgress("Loading providers")
            else if(kind!="Contact" && catalog!=null && providers.isEmpty())BuilderNotice(if(locating)"Weather needs a place lookup provider on this server. A server admin can add one in Services." else "$kind isn't set up on this server yet. A server admin can add a provider in Services.")
            else {
                if(providers.isNotEmpty())LazyRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                    items(providers,key={it.getValue("id").jsonPrimitive.content}) {p->val name=p.getValue("id").jsonPrimitive.content;FilterChip(selected==p,{provider=name},label={Text(name,maxLines=1,overflow=TextOverflow.Ellipsis)},shape=RoundedCornerShape(12.dp))}
                }
                if(place!=null) {
                    Text(place?.get("name")?.jsonPrimitive?.content.orEmpty(),style=MaterialTheme.typography.titleMedium,maxLines=2,overflow=TextOverflow.Ellipsis)
                    SigilTextButton({place=null;places=emptyList()}){Text("Change place")}
                    Toggle("Include forecast",forecast){forecast=it}
                }
                else FormField(if(kind=="Contact")"Account address" else if(locating)"Place" else if(kind=="Definition")"Word" else "Text",text,{text=it},multiline=kind=="Translation")
                if(kind!="Weather" && kind!="Contact")LanguagePicker(if(kind=="Translation")"Translate to" else "Dictionary language",language) {language=it}
                selected?.let {Text("This lookup sends your query through your server to ${it.getValue("endpoint").jsonPrimitive.content}. The resulting card is encrypted when sent to the conversation.",
                    style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant,maxLines=4,overflow=TextOverflow.Ellipsis)}
                Expandable(places.isNotEmpty()) {
                    SettingsSectionLabel("Places")
                    places.forEach {p->PlaceRow(p["name"]?.jsonPrimitive?.content.orEmpty(),listOfNotNull(p["region"]?.jsonPrimitive?.content,p["country"]?.jsonPrimitive?.content).joinToString(", ")) {place=p;places=emptyList();provider=""}}
                }
            }
            if(busy && !loadingCatalog)BuilderProgress(if(locating)"Finding places" else "Looking up")
            issue?.let {Text(it,Modifier.semantics {liveRegion=LiveRegionMode.Polite},style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.error);if(catalog==null)SigilTextButton({retry++}){Text("Reload providers")}}
        }
        BuilderConfirm(if(busy)"Working…" else if(locating)"Find place" else "Look up and attach",enabled=!busy && (selected!=null || kind=="Contact" && access!=null) && newId!=null && (place!=null || text.isNotBlank())) {scope.launch {
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
            } catch(e:kotlinx.coroutines.CancellationException){throw e} catch(_:Exception){issue=lookupIssue(kind,locating)} finally {busy=false}
        }}
    }
}
private val recentLanguages=mutableStateListOf<String>()
internal fun pickerName(code:String)=pickerLanguages.firstOrNull {it.first.equals(code,true)}?.second ?: languageName(code)
// Names that start with the query first, then names containing it, then an exact code.
internal fun languageMatches(query:String):List<Pair<String,String>> {
    val q=query.trim()
    if(q.isEmpty())return emptyList()
    return (pickerLanguages.filter {it.second.startsWith(q,true)}+pickerLanguages.filter {it.second.contains(q,true)}+pickerLanguages.filter {it.first.equals(q,true)}).distinct()
}
@Composable internal fun LanguagePicker(label:String,code:String,pick:(String)->Unit) {
    var query by remember(code) {mutableStateOf(pickerName(code))}
    val matches=remember(query,code) {if(query==pickerName(code))emptyList() else languageMatches(query).take(5)}
    val choose={c:String->recentLanguages.remove(c);recentLanguages.add(0,c);while(recentLanguages.size>6)recentLanguages.removeAt(recentLanguages.lastIndex);query=pickerName(c);pick(c)}
    FormField(label,query,{query=it},isError=query.isNotBlank() && query!=pickerName(code) && matches.isEmpty())
    if(query!=pickerName(code) && matches.isEmpty())Text(if(query.isBlank())"Type a language name." else "No language matches. ${pickerName(code)} is still selected.",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
    val recents=recentLanguages.filter {!it.equals(code,true)}.take(4)
    if(recents.isNotEmpty() && query==pickerName(code))LazyRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
        items(recents,key={it}) {c->FilterChip(false,{choose(c)},label={Text(pickerName(c),maxLines=1)},shape=RoundedCornerShape(12.dp))}
    }
    Expandable(matches.isNotEmpty()) {
        matches.forEach {(c,name)->
            Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(18.dp)).clickable(onClickLabel="Choose $name",role=Role.Button) {choose(c)}.heightIn(min=48.dp).padding(horizontal=12.dp,vertical=8.dp),verticalAlignment=Alignment.CenterVertically) {
                Text(name,Modifier.weight(1f),style=MaterialTheme.typography.bodyLarge,maxLines=1,overflow=TextOverflow.Ellipsis)
                Text(c,style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}
@Composable private fun BuilderNotice(message:String) {
    Box(Modifier.fillMaxWidth().padding(32.dp),contentAlignment=Alignment.Center) {Text(message,style=MaterialTheme.typography.bodyMedium,color=MaterialTheme.colorScheme.onSurfaceVariant)}
}
@Composable private fun BuilderProgress(label:String) {
    Box(Modifier.fillMaxWidth().padding(vertical=12.dp).semantics {liveRegion=LiveRegionMode.Polite;contentDescription=label},contentAlignment=Alignment.Center) {CircularProgressIndicator(Modifier.size(22.dp),strokeWidth=2.dp)}
}
@Composable private fun PlaceRow(name:String,detail:String,choose:()->Unit) {
    Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(18.dp)).clickable(onClickLabel="Use this place",role=Role.Button,onClick=choose).heightIn(min=48.dp).padding(horizontal=12.dp,vertical=12.dp),verticalAlignment=Alignment.CenterVertically) {
        CompositionLocalProvider(LocalContentColor provides MaterialTheme.colorScheme.onSurfaceVariant) {Glyph("place",24)}
        Column(Modifier.weight(1f).padding(horizontal=12.dp),verticalArrangement=Arrangement.spacedBy(4.dp)) {
            Text(name,style=MaterialTheme.typography.titleMedium,maxLines=1,overflow=TextOverflow.Ellipsis)
            if(detail.isNotEmpty())Text(detail,style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant,maxLines=1,overflow=TextOverflow.Ellipsis)
        }
    }
}
// The server reports failures without detail, so each names what to try rather than guessing a cause.
internal fun lookupIssue(kind:String,locating:Boolean)=when {
    locating->"Couldn't look up places. Check the name and try again."
    kind=="Definition"->"No definition came back. Check the spelling and language, then try again."
    kind=="Translation"->"The translation didn't come through. Your text is kept; try again."
    kind=="Weather"->"Weather isn't available right now. Try again in a moment."
    else->"The lookup failed. Your text is kept; try again."
}
