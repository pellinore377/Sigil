@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)
package org.sigil

import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.ui.input.key.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.background
import androidx.compose.material3.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.blur
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.Alignment
import androidx.compose.ui.unit.dp
import kotlinx.browser.window
import kotlinx.browser.document
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.*
import kotlin.js.*

private external interface BrowserDocument:JsAny {val visibilityState:String}
@JsName("document") private external val browserDocument:BrowserDocument
@JsName("Date") private external class BrowserDate(value:Double):JsAny {
    fun toLocaleTimeString(locale:String,options:BrowserTimeOptions):String
    fun toLocaleDateString(locale:String,options:BrowserCalendarOptions):String
    fun getFullYear():Int
    fun getMonth():Int
    fun getDate():Int
    fun setDate(value:Int):Double
    companion object {fun now():Double}
}
@JsName("Intl") private external object BrowserIntl:JsAny {fun DateTimeFormat(locale:String=definedExternally,options:BrowserCalendarOptions=definedExternally):BrowserDateFormat}
private external interface BrowserDateFormat:JsAny {fun resolvedOptions():BrowserDateOptions;fun formatToParts(value:BrowserDate):JsArray<BrowserDatePart>}
private external interface BrowserDatePart:JsAny {val type:String}
private external interface BrowserDateOptions:JsAny {val timeZone:String}
@JsName("Object") private external class BrowserTimeOptions:JsAny {var hour:String;var minute:String}
@JsName("Object") private external class BrowserCalendarOptions:JsAny {var day:String;var month:String;var year:String}
private val timeOptions=BrowserTimeOptions().apply {hour="numeric";minute="2-digit"}
private fun clock(value:Long)=if(value==0L)"" else BrowserDate(value*1000.0).toLocaleTimeString(window.navigator.language,timeOptions)
private fun calendar(value:Long):String {
    if(value==0L)return ""
    val date=BrowserDate(value*1000.0)
    val today=BrowserDate(BrowserDate.now())
    val yesterday=BrowserDate(BrowserDate.now()).apply{setDate(getDate()-1)}
    fun same(a:BrowserDate,b:BrowserDate)=a.getFullYear()==b.getFullYear() && a.getMonth()==b.getMonth() && a.getDate()==b.getDate()
    val label=when{same(date,today)->"Today";same(date,yesterday)->"Yesterday";else->date.toLocaleDateString(window.navigator.language,BrowserCalendarOptions().apply{day="numeric";month="long";if(date.getFullYear()!=today.getFullYear())year="numeric"})}
    return "$label, ${clock(value)}"
}
private fun dateOrder():String {
    val parts=BrowserIntl.DateTimeFormat(window.navigator.language,BrowserCalendarOptions().apply{day="numeric";month="numeric"}).formatToParts(BrowserDate(981173106000.0)).toList()
    return if(parts.indexOfFirst{it.type=="month"}<parts.indexOfFirst{it.type=="day"})"month" else "day"
}
private fun json(value:Any?):JsonElement=when(value) {
    null->JsonNull;is JsonElement->value;is String->JsonPrimitive(value);is Boolean->JsonPrimitive(value);is Number->JsonPrimitive(value)
    is Map<*,*>->JsonObject(value.entries.associate {it.key.toString() to json(it.value)})
    is List<*>->JsonArray(value.map(::json));else->error("Unsupported command value")
}
private val stamped=setOf("post","place","group_create","react","pin","read","mark_read","snooze","forward","organize","edit","delete","clear_conversation","note","typing","draft")

@JsFun("(m) => console.log(m)") internal external fun browserTimingLog(message:String)

@Composable internal fun WebMessenger() {
    var state by remember {mutableStateOf(MessengerState(loginAddress=window.location.hostname))}
    var visible by remember {mutableStateOf(browserDocument.visibilityState=="visible")}
    DisposableEffect(Unit){val listener:(org.w3c.dom.events.Event)->Unit={visible=browserDocument.visibilityState=="visible"};document.addEventListener("visibilitychange",listener);onDispose{document.removeEventListener("visibilitychange",listener)}}
    var materialsReady by remember {mutableStateOf(false)}
    var notificationsReady by remember {mutableStateOf(false)}
    var notificationNext by remember {mutableStateOf(0.0)}
    DisposableEffect(Unit){val listener:(org.w3c.dom.events.Event)->Unit={runCatching{browserMaterialShutdown()};Unit};window.addEventListener("pagehide",listener);onDispose{window.removeEventListener("pagehide",listener);runCatching{browserMaterialShutdown()}}}
    LaunchedEffect(Unit) {try{initializeMaterialWasm().awaitBrowser<JsAny?>();materialsReady=initializeMaterialGpu().awaitBrowser<JsBoolean>().toBoolean()}catch(_:Exception){}}
    var ready by remember {mutableStateOf(false)}
    var startupError by remember {mutableStateOf<String?>(null)}
    var signOut by remember {mutableStateOf(false)}
    var signOutFailed by remember {mutableStateOf(false)}
    var signOutSaved by remember {mutableStateOf(false)}
    var linking by remember {mutableStateOf<JsonObject?>(null)}
    var linkingBusy by remember {mutableStateOf(false)}
    var linkingIssue by remember {mutableStateOf<String?>(null)}
    var syncIssue by remember {mutableStateOf<String?>(null)}
    val workNotices = remember { WorkNotices() }
    var contactQr by remember {mutableStateOf<JsonObject?>(null)}
    var photoRevision by remember {mutableIntStateOf(0)}
    var wallpaperRevision by remember {mutableIntStateOf(0)}
    var passkeysReady by remember {mutableStateOf(false)}
    var accessNext by remember {mutableStateOf(0L)}
    var viewing by remember {mutableStateOf<WebFile?>(null)}
    var recordingJob by remember {mutableStateOf<Job?>(null)}
    var fileNext by remember {mutableStateOf(0L)}
    var authorization by remember {mutableStateOf<String?>(null)}
    val timezone=remember {runCatching {BrowserIntl.DateTimeFormat().resolvedOptions().timeZone}.getOrDefault("UTC")}
    val dateOrder=remember {runCatching{dateOrder()}.getOrDefault("day")}
    val screens=rememberSaveableStateHolder()
    val scope=rememberCoroutineScope()
    val mutex=remember {Mutex()}
    val wake=remember {Channel<Unit>(Channel.CONFLATED)}
    val fileWake=remember {Channel<Unit>(Channel.CONFLATED)}
    var forceSync by remember {mutableStateOf(false)}
    // Read once the core module is initialised; asking earlier throws inside the glue.
    var videoReady by remember {mutableStateOf(false)}
    var post by remember {mutableStateOf<Pair<Map<String,Any?>,String>?>(null)}
    var sending by remember {mutableStateOf(false)}
    var groupCreate by remember {mutableStateOf<Pair<Map<String,Any?>,String>?>(null)}
    var creatingGroup by remember {mutableStateOf(false)}
    val forwarding=remember {mutableSetOf<Map<String,Any?>>() }
    var filter by remember {mutableStateOf<Map<String,Any?>>(mapOf("category" to "Timeline"))}
    var searchAfter by remember {mutableStateOf<Long?>(null)}
    var searchCategory by remember {mutableStateOf("")}
    var searchGeneration by remember {mutableStateOf(0)}
    var searchJob by remember {mutableStateOf<Job?>(null)}
    var callExpanded by remember {mutableStateOf(false)}
    var viewportEnd by remember {mutableStateOf(0)}
    var timelineWant by remember {mutableStateOf(0)}
    var timelineReloadAt by remember {mutableStateOf(Int.MAX_VALUE)}
    var timelineJob by remember {mutableStateOf<Job?>(null)}
    var viewportChase by remember {mutableStateOf(false)}
    suspend fun native(raw:String):JsonObject {
        val response=Json.parseToJsonElement(browserCommand(raw).awaitBrowser<JsString>().toString()).jsonObject
        check(response.bool("ok")) {response.string("error")}
        return response["value"]?.jsonObject ?: JsonObject(emptyMap())
    }
    fun request(name:String,fields:Map<String,Any?>):String {
        val values=fields.toMutableMap();values["command"]=name
        (values["shared_contact"] as? String)?.let {values["shared_contact"]=Json.parseToJsonElement(it)}
        if(name in stamped) {values["request"]=browserRequestId();values["timestamp"]=(BrowserDate.now()/1000).toLong()}
        if(name=="card_action" || name=="poll_close")values["timestamp"]=(BrowserDate.now()/1000).toLong()
        if(name in setOf("oidc","enroll"))values["label"]="Web browser"
        if(name=="post" && values["timezone"]==null)values["timezone"]=timezone
        return json(values).toString()
    }
    suspend fun execute(name:String,fields:Map<String,Any?> = emptyMap())=native(request(name,fields))
    val locations=remember {WebLocations()}
    val calls=remember {WebCalls(scope,{name,fields->mutex.withLock{execute(name,fields)}},{wake.trySend(Unit)},{message->state=state.copy(issue=message)})}
    DisposableEffect(Unit){val listener:(org.w3c.dom.events.Event)->Unit={calls.close();locations.stop();locations.live=false};window.addEventListener("pagehide",listener);onDispose{window.removeEventListener("pagehide",listener);calls.close()}}
    suspend fun notificationStatus() {
        if(!notificationsReady)return
        val local=webNotificationsEnabled().awaitBrowser<JsBoolean>().toBoolean()
        val permission=webNotificationsPermission()
        val remote=execute("browser_push",mapOf("action" to "status"))
        val status=when {
            permission=="unsupported"->"This browser does not support background notifications."
            permission=="denied"->"Notifications are blocked. Allow them in this site's browser permissions."
            !local->"Background notifications are off."
            remote.string("remote")=="active"->"Background notifications are connected."
            remote.string("remote") in setOf("invalid","expired")->"Notification delivery needs reconnecting."
            else->"Waiting for notification delivery confirmation…"
        }
        state=state.copy(push=PushSettings(local,status,if(local)"browser" else null,if(permission=="unsupported")emptyList()else listOf(PushDistributor("browser","Browser notifications"))))
    }
    suspend fun notificationWork() {
        if(!notificationsReady || BrowserDate.now()<notificationNext)return
        notificationNext=BrowserDate.now()+3000
        val pending=webNotificationsPending().awaitBrowser<JsString>().toString()
        if(pending.isNotEmpty()) {
            val proof=Json.parseToJsonElement(pending).jsonObject
            if(execute("browser_push",mapOf("action" to "receive","endpoint" to proof.string("endpoint"),"payload" to proof.string("payload"))).bool("accepted")){webNotificationsClear(pending).awaitBrowser<JsAny?>();fileNext=0;fileWake.trySend(Unit);wake.trySend(Unit)}
        }
        if(state.push!=null)notificationStatus()
    }
    suspend fun timeline() {
        if(callExpanded)return
        val peer=state.selected ?: return
        val requestFilter=filter
        var before:Long?=null
        val messages=mutableListOf<ChatMessage>()
        var value:JsonObject
        // Publishing fewer messages than are already on screen would drop the reader's place.
        val onScreen=if(state.selected==peer)state.messages.size else 0
        val chats=if(peer=="self" && state.chats.none {it.id==peer})state.chats+ChatSummary("self",state.address,"","",true,emptyList(),displayName="Note to Self") else state.chats
        do {
            value=execute("timeline",requestFilter+mapOf("peer" to peer,"before" to before,"visible_end" to viewportEnd))
            if(callExpanded || state.selected!=peer || filter!=requestFilter)return
            // Read on every page, so a viewport that deepens mid-load extends this scan instead of restarting it.
            timelineWant=value.long("want").toInt();timelineReloadAt=value.long("reload_at").toInt()
            val buffer=TimelineBufferDepth(value.long("cache_ahead")/10f,value.long("cache_behind")/10f)
            messages+=StateDecoder.messages(value,peer,::clock,::calendar)
            before=value["next"]?.jsonPrimitive?.longOrNull
            if(timelinePublishes(messages.size,onScreen,timelineWant,before==null))state=state.copy(chats=chats,messages=messages.toList(),more=before!=null,timelineLoaded=true,typing=value.strings("typing"),people=value.settings("people"),timelineBuffer=buffer)
        }while(before!=null && messages.size<timelineWant)
    }
    fun storage(value:JsonObject) {
        val r=value["recovery"]!!.jsonObject
        state=state.copy(storage=StorageDetails(value.long("database"),value.long("media"),value.long("media_used"),value.long("budget"),r.bool("enabled"),r["last"]?.jsonPrimitive?.longOrNull?.let(::clock),r.long("pending"),r["days"]?.jsonPrimitive?.intOrNull,r.bool("restoring")))
    }
    fun account(value:JsonObject) {
        value["access"]?.takeUnless{it==JsonNull}?.jsonObject?.let {a->state=state.copy(accountAccess=AccountAccess(a.long("configuration_revision"),a.long("transition_revision"),a.optional("issuer"),a.bool("linked"),a.bool("retiring"),a.bool("invitation_fallback_acknowledged"),value.bool("link_pending")))}
    }
    suspend fun service(raw:String):ServiceResponse=mutex.withLock {
        val result=native(raw)
        ServiceResponse(result.toString(),result["preview"]?.jsonObject?.let {ContentDecoder.part(it.toString(),::clock)})
    }
    suspend fun recipe(message:ChatMessage,part:MessagePart,serves:Int):RecipeContent=mutex.withLock {
        ContentDecoder.recipeContent(execute("recipe_view",mapOf("peer" to message.peer,"author" to message.author,"message" to message.id,"card" to part.id,"serves" to serves)).toString())?:error("Recipe unavailable")
    }
    suspend fun transfers() {


        val value=execute("files")
        state=state.copy(transfers=value["uploads"]?.jsonArray?.map {item->item.jsonObject.let {Transfer(it.string("request"),it.string("peer"),it.string("name"),it.long("length"),it.string("phase"),it.bool("draft"),it.string("media_type"))}}.orEmpty())
    }
    suspend fun fileWork() {
        val work=execute("file_work",mapOf("steps" to 1));fileNext=work.long("next_at")
        val notices = work["notices"]?.jsonObject
        if (notices != null) {
            var notice = state.issue
            for ((source, message) in notices) notice = workNotices.update(notice, source, fileWorkNotice(source, message.jsonPrimitive.contentOrNull))
            state = state.copy(issue = notice)
        } else work.optional("issue")?.let {state=state.copy(issue=it)}
        transfers()
        if(work.long("sent")>0){timeline();wake.trySend(Unit)}
    }
    val voiceWaves=remember {mutableStateMapOf<String,Pair<List<Float>,Long>>()}
    val recordedLevels=remember {mutableListOf<Float>()}
    LaunchedEffect(state.transfers.map {it.request}) {voiceWaves.keys.toList().filter {id->state.transfers.none {it.request==id}}.forEach {voiceWaves.remove(it)}}
    suspend fun stageChunks(metadata:JsonObject,fields:Map<String,Any?>,draft:Boolean=true,read:suspend (Int)->JsAny):String {
        val id=browserRequestId()
        try {
            mutex.withLock {
                execute("file_begin",fields.filterKeys {it!="kind"}+mapOf("request" to id,"timestamp" to (BrowserDate.now()/1000).toLong(),"draft" to draft,"name" to metadata.string("name"),"media_type" to metadata.string("media_type"),"length" to metadata.long("length")))
                transfers()
            }
            val count=(metadata.long("length")+1048575)/1048576
            for(index in 0 until count.toInt()) {
                currentCoroutineContext().ensureActive()
                val bytes=read(index)
                try {mutex.withLock {browserFileStage(id,index,bytes).awaitBrowser<JsAny?>()}}
                finally {browserReleaseBytes(bytes)}
                yield()
            }
            mutex.withLock {execute("file_finish",mapOf("request" to id));fileNext=0;fileWake.trySend(Unit);transfers();wake.trySend(Unit)}
        }catch(error:Exception){
            withContext(NonCancellable){mutex.withLock {runCatching {execute("file_cancel",mapOf("request" to id));transfers()}}}
            throw error
        }
        return id
    }
    suspend fun stage(file:JsAny,fields:Map<String,Any?>) = stageChunks(Json.parseToJsonElement(browserFileMetadata(file)).jsonObject,fields) {browserFileSlice(file,it).awaitBrowser<JsAny>()}
    suspend fun download(file:WebFile) {
        if(file.draft.isEmpty())withTimeout(120000) {
            while(true) {
                val phase=mutex.withLock {execute("file_get",mapOf("peer" to file.peer,"author" to file.author,"message" to file.message)).string("phase")}
                if(phase in setOf("Complete","Published","Restored"))break
                mutex.withLock {fileWork()}
                val completed=mutex.withLock {execute("file_get",mapOf("peer" to file.peer,"author" to file.author,"message" to file.message)).string("phase") in setOf("Complete","Published","Restored")}
                if(completed)break
                delay(foregroundSyncWait(fileNext,BrowserDate.now().toLong()))
            }
        }
    }
    suspend fun saveFile(file:WebFile,handle:JsAny) {
        download(file)
        browserFileSave(handle,file.peer,file.author,file.message,file.draft,file.bytes.toDouble()).awaitBrowser<JsAny?>()
    }
    // Delivered files keep their object URL across rows, so scrolling back never decrypts a file again.
    val fileUrls=LinkedHashMap<String,String>()
    suspend fun loadFile(file:WebFile):String {
        val key=if(file.draft.isEmpty())"${file.peer}/${file.author}/${file.message}" else null
        key?.let {fileUrls.remove(it)}?.let {fileUrls[key]=it;return it}
        download(file)
        val promise=browserFileUrl(file.peer,file.author,file.message,file.draft,file.bytes.toDouble(),file.type)
        var claimed=false
        try {
            val value=promise.awaitBrowser<JsString>().toString();claimed=true
            if(key!=null) {
                fileUrls[key]=value
                while(fileUrls.size>48) {val oldest=fileUrls.entries.first();fileUrls.remove(oldest.key);browserRevokeFileUrl(oldest.value)}
            }
            return value
        }
        finally {if(!claimed)promise.then<JsAny?>({browserRevokeFileUrl(it.toString());null},{null})}
    }
    suspend fun refresh() {
state=StateDecoder.state(execute("state"),state,::clock);if(state.phase=="connected"){timeline();transfers();calls.refresh(execute("calls"),::clock,{calendar(it).substringBeforeLast(", ")});if(state.storage!=null)runCatching{storage(execute("storage"))};if((BrowserDate.now()/1000).toLong()>=accessNext){
        // Account standing is a status read, not a way in. It failing used to abort startup and
        // leave the whole client behind "could not open this browser device".
        runCatching{account(execute("account_access"))}.onFailure{browserTimingLog("SigilTiming account_access failed=${it.message?.take(120)}")}
        accessNext=(BrowserDate.now()/1000).toLong()+300}}}
    LaunchedEffect(Unit) {
        try {val t0=BrowserDate.now();initializeBrowser().awaitBrowser<JsAny?>();videoReady=runCatching{browserVideoSupported()}.getOrDefault(false);passkeysReady=runCatching{browserPasskeySupported()}.getOrDefault(false);calls.initialize();val t1=BrowserDate.now();startBrowser().awaitBrowser<JsAny?>();val t2=BrowserDate.now();refresh();val t3=BrowserDate.now();linking=execute("device_link",mapOf("action" to "status")).takeUnless{it.string("stage")=="none"};ready=true
            // Durations only: where a slow start sat, and whether the tab was visible while it did.
            browserTimingLog("SigilTiming startup init=${(t1-t0).toLong()}ms worker=${(t2-t1).toLong()}ms state=${(t3-t2).toLong()}ms link=${(BrowserDate.now()-t3).toLong()}ms visible=${browserDocument.visibilityState}")
            try{initializeNotificationWasm().awaitBrowser<JsAny?>();notificationsReady=webNotificationsSupported();if(state.phase!="connected" && notificationsReady)webNotificationsDisable().awaitBrowser<JsAny?>()}catch(_:Exception){}
        }
        catch(e:Exception){startupError="Could not open this browser device. Close other Sigil tabs and reload. A current browser with private storage and cross-origin isolation is required."+(e.message?.take(160)?.let{" ($it)"}?:"")}
    }
    LaunchedEffect(ready) {
        var nextSync=0L
        if(ready)while(isActive) {
            val active=browserDocument.visibilityState=="visible" || calls.visible!=null
            val nudged=withTimeoutOrNull(if(active && state.phase=="connected") foregroundSyncWait(nextSync,BrowserDate.now().toLong(),if(calls.visible!=null)250 else 1000) else 1500){wake.receive();true}==true
            if(browserDocument.visibilityState=="visible" && state.phase=="oidc") {
                try {mutex.withLock {refresh();if(state.phase in setOf("connected","recover","username")&&authorization!=null){authorization=null;browserSsoClose()}}}
                catch(cancelled:CancellationException){throw cancelled}
                catch(_:Exception){}
            }
            if((browserDocument.visibilityState=="visible" || calls.visible!=null) && state.phase=="connected") {
                try {
                    val force=forceSync;forceSync=false
                    val startedAt=BrowserDate.now()
                    val result=mutex.withLock {execute("sync",mapOf("interactive" to true,"wake" to force,"call_setup" to (calls.visible?.connection in setOf("connecting","securing","reconnecting"))))}
                    // Durations only, like the Android SigilTiming log; no content.
                    browserTimingLog("SigilTiming sync ${(BrowserDate.now()-startedAt).toLong()}ms native=${result["ms"]?.jsonPrimitive?.longOrNull?:0} ran=${result.bool("ran")} wake=$force lanes=${result["lanes"]} ${result["timings"]}")
                    nextSync=result.long("next_at")
                    if(((result.bool("ran") && (result["changed"]?.jsonPrimitive?.booleanOrNull ?: true)) || nudged) && !sending)mutex.withLock {if(!sending)refresh()}
                    val issue=result.optional("issue")
                    if(issue!=null || result.bool("ran")){state=state.copy(issue=workNotices.update(state.issue,"sync",issue));syncIssue=issue}
                } catch(cancelled:CancellationException){throw cancelled}
                catch(e:Exception){nextSync=(BrowserDate.now()/1000).toLong()+1;syncIssue=e.message?:"Synchronization failed. Your queued messages are preserved.";state=state.copy(issue=workNotices.update(state.issue,"sync",syncIssue))}
            }
        }
    }
    LaunchedEffect(ready,visible,state.phase) {
        if(ready && visible && state.phase=="connected")fileWake.trySend(Unit)
    }
    LaunchedEffect(ready,visible,state.phase) {
        if(!(ready && visible && state.phase=="connected"))return@LaunchedEffect
        var fast=0
        while(isActive) {
            val started=BrowserDate.now()
            val hit=try {browserMailboxWatch().awaitBrowser<JsBoolean>().toBoolean()}
            catch(cancelled:CancellationException){throw cancelled}
            catch(_:Exception){delay(5000);continue}
            // Re-arm at once; only a run of instant hits (mail nobody acknowledges) waits.
            fast=if(BrowserDate.now()-started<300)fast+1 else 0
            if(hit){forceSync=true;wake.trySend(Unit);if(fast>=3)delay(1000)}
            else if(fast>=3)delay(2000)
        }
    }
    LaunchedEffect(ready) {
        if(ready)while(isActive) {
            withTimeoutOrNull(if(visible && state.phase=="connected") foregroundSyncWait(fileNext,BrowserDate.now().toLong()) else 1500){fileWake.receive()}
            if(visible && state.phase=="connected") {
                try {
                    if((BrowserDate.now()/1000).toLong()>=fileNext)mutex.withLock {fileWork()}
                    mutex.withLock {notificationWork()}
                } catch(cancelled:CancellationException){throw cancelled}
                catch(_:Exception){fileNext=(BrowserDate.now()/1000).toLong()+1}
            }
        }
    }
    val voiceTarget=remember {VoiceDraftTarget()}
    var pendingVoiceFile by remember {mutableStateOf<JsAny?>(null)}
    fun stopRecording() {
        if(state.voice.phase !in setOf("Recording","Save failed"))return
        state=state.copy(voice=state.voice.copy(phase="Saving"))
        recordingJob=scope.launch {
            try {val file=pendingVoiceFile ?: browserVoiceFinish().awaitBrowser<JsAny>().also {pendingVoiceFile=it};val id=stage(file,voiceTarget.fields);voiceWaves[id]=recordedLevels.toList() to (state.voice.seconds*1000);recordedLevels.clear();pendingVoiceFile=null;voiceTarget.clear();state=state.copy(voice=VoiceState())}
            catch(cancelled:CancellationException){throw cancelled}
            catch(_:Exception){state=state.copy(voice=state.voice.copy(phase=if(pendingVoiceFile!=null)"Save failed" else "Idle"),issue="Could not save the voice recording. Retry or discard it.")}
        }
    }
    LaunchedEffect(state.voice.phase) {
        if(state.voice.phase=="Recording") {
            var elapsed=0.0;var last=BrowserDate.now()
            while(isActive) {
                delay(100)
                try {
                    val voice=state.voice;val now=BrowserDate.now()
                    if(!voice.paused){elapsed+=now-last;val level=browserVoiceLevel();if(recordedLevels.size<6000)recordedLevels.add(level);state=state.copy(voice=voice.copy(seconds=(elapsed/1000).toLong(),levels=(voice.levels+level).takeLast(80)))};last=now
                }catch(_:Exception){stopRecording()}
            }
        }
    }
    DisposableEffect(Unit){onDispose{browserVoiceCancel()}}
    fun finishRemoval() {ready=false;browserCameraStop();val keys=(0 until window.localStorage.length).mapNotNull{window.localStorage.key(it)};keys.filter{it.startsWith("messenger.")}.forEach{window.localStorage.removeItem(it)};window.location.reload()}
    fun search(query: String, category: String, more: Boolean = false) {
        if (!ready || more && (state.searching || !state.searchMore)) return
        searchJob?.cancel()
        val generation = ++searchGeneration
        val previous = state.searchHits
        val sameSearch = state.searchQuery == query && searchCategory == category
        if (!more) searchAfter = null
        searchCategory = category
        state = state.copy(searchQuery = query, searchHits = if (more || sameSearch) previous else emptyList(), searching = true)
        searchJob = scope.launch {
            try {
                var after = searchAfter
                val hits = if (more) previous.toMutableList() else mutableListOf<SearchHit>()
                val initialSize = hits.size
                do {
                    val value = mutex.withLock { execute("search", mapOf("query" to query, "category" to category, "after" to after)) }
                    if (generation != searchGeneration) return@launch
                    hits += StateDecoder.search(value, ::clock)
                    after = value["next"]?.jsonPrimitive?.longOrNull
                    searchAfter = after
                    state = state.copy(searchHits = if (sameSearch && !more && hits.isEmpty() && after != null) previous else hits.toList(),
                        searchMore = after != null, searching = after != null && hits.size - initialSize < 64)
                    yield()
                } while (after != null && hits.size - initialSize < 64)
                state = state.copy(searching = false, searchMore = after != null)
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { if (generation == searchGeneration) state = state.copy(searching = false, issue = "Search could not finish.") }
        }
    }
    val command:Command=command@{name,fields->
        if(name=="call_display"){
            val expanded=fields["expanded"]==true
            if(expanded!=callExpanded){
                callExpanded=expanded
                if(expanded)timelineJob?.cancel() else scope.launch {mutex.withLock {timeline()}}
            }
            return@command
        }
        if(calls.handle(name,fields))return@command
        when(name) {
            "device_link"->{
                if(linkingBusy)return@command
                if(fields["action"]=="pause"){linking=null;return@command}
                linkingBusy=true
                scope.launch {mutex.withLock {
                    try {
                        val result=execute(name,fields)
                        linking=result.takeUnless{it.string("stage")=="none"};linkingIssue=null
                        if(result.string("stage")=="done"){
                            runCatching{refresh()}.onFailure{state=state.copy(issue=it.message)}
                            if(!result.bool("sponsor")){execute(name,mapOf("action" to "close"));linking=null}
                        }
                    }catch(cancelled:CancellationException){throw cancelled}
                    catch(error:Exception){linkingIssue=error.message?:"Could not complete this linking step. Retry when connected."}
                    finally {
                        runCatching{execute(name,mapOf("action" to "status"))}.onSuccess{linking=it.takeUnless{v->v.string("stage")=="none"}}
                        linkingBusy=false
                    }
                }}
                return@command
            }
            "forward"->{
                if(!ready || !forwarding.add(fields.toMap()))return@command
                scope.launch {
                    try {
                        val metadata=mutex.withLock {execute(name,fields)["forward_file"]?.jsonObject}
                        if(metadata!=null) {
                            val source=fields["source"] as String;val author=fields["author"] as String;val message=fields["message"] as String
                            download(WebFile(source,author,message,metadata.string("name"),metadata.string("media_type"),metadata.long("length")))
                            stageChunks(metadata,mapOf("peer" to fields["peer"],"caption" to metadata.string("caption")),draft=false) {index->browserFileRead(source,author,message,index).awaitBrowser<JsAny>()}
                        }
                        mutex.withLock {refresh()};wake.trySend(Unit)
                    }catch(cancelled:CancellationException){throw cancelled}
                    catch(_:Exception){state=state.copy(issue="Could not forward this message. Check its availability and your connection.")}
                    finally {forwarding.remove(fields)}
                };return@command
            }
            "photo_choose","photo_remove"->{
                val selection=if(name=="photo_choose")browserPickFile(true)else null
                scope.launch {
                    try {
                        val file=selection?.awaitBrowser<JsAny?>()
                        if(selection!=null && file==null)return@launch
                        mutex.withLock {state=state.copy(busy=true);browserProfileStage(file).awaitBrowser<JsAny?>();state=state.copy(photoPending=true);photoRevision++;execute("photo_publish");photoRevision++;refresh()}
                    }catch(cancelled:CancellationException){throw cancelled}
                    catch(_:Exception){state=state.copy(issue="Could not update your profile photo. Choose an image under 16 MiB, or retry publishing it.")}
                    finally {state=state.copy(busy=false)}
                };return@command
            }
            "notification_settings"->{scope.launch{try{mutex.withLock{notificationStatus()}}catch(_:Exception){state=state.copy(issue="Could not read notification status.")}};return@command}
            "push_select"->{
                if(state.busy || !notificationsReady)return@command
                val permission=webNotificationsRequest()
                state=state.copy(busy=true)
                scope.launch{try {
                    permission.awaitBrowser<JsAny?>()
                    val providers=mutex.withLock{execute("browser_push",mapOf("action" to "providers"))}
                    check(providers.bool("unified_push")){"Your administrator must enable Web Push in server notification settings."}
                    val target=Json.parseToJsonElement(webNotificationsSubscribe(providers.string("vapid_public_key")).awaitBrowser<JsString>().toString())
                    mutex.withLock{execute("browser_push",mapOf("action" to "register","target" to target));notificationStatus()};notificationNext=0.0;fileNext=0;fileWake.trySend(Unit);wake.trySend(Unit)
                }catch(cancelled:CancellationException){throw cancelled}catch(e:Exception){state=state.copy(issue=e.message?:"Could not enable notifications. Check your browser permissions and server settings.")}
                finally{state=state.copy(busy=false)}};return@command
            }
            "push_disable"->{
                if(state.busy || !notificationsReady)return@command
                state=state.copy(busy=true)
                scope.launch{try{webNotificationsDisable().awaitBrowser<JsAny?>();mutex.withLock{execute("browser_push",mapOf("action" to "disable"));notificationStatus()};fileNext=0;fileWake.trySend(Unit);wake.trySend(Unit)}catch(cancelled:CancellationException){throw cancelled}catch(_:Exception){state=state.copy(issue="Could not finish disabling notifications. Try again.")}finally{state=state.copy(busy=false)}};return@command
            }
            // Opened inside the click so popup blockers allow it; navigated once the core returns the URL.
            "oidc"->if(ready && !state.busy)browserSsoOpen()
            "oidc_reopen"->{authorization?.let {browserSsoNavigate(it)};return@command}
            "passkey_recover","passkey_create"->{
                if(!ready || state.busy)return@command
                if(!browserPasskeyWindowOpen()){state=state.copy(issue="Allow pop-ups for Sigil to use passkeys.");return@command}
                state=state.copy(busy=true,issue=null)
                scope.launch {
                    try {
                        // The ceremony runs outside the lock so sync keeps going while the passkey sheet is open.
                        if(name=="passkey_recover") {
                            val request=mutex.withLock{execute("passkey_request")}
                            val proof=Json.parseToJsonElement(browserPasskeyWindowRun("get",request.toString()).awaitBrowser<JsString>().toString()).jsonObject
                            mutex.withLock{execute("recover",mapOf("credential" to proof.string("credential"),"prf" to proof.string("prf")));refresh()};fileNext=0;fileWake.trySend(Unit)
                        } else {
                            val options=mutex.withLock{execute("passkey_create_options")}
                            val made=Json.parseToJsonElement(browserPasskeyWindowRun("create",options.toString()).awaitBrowser<JsString>().toString()).jsonObject
                            mutex.withLock{execute("passkey_add",mapOf("credential" to made.string("credential"),"salt" to made.string("salt"),"prf" to made.string("prf"),"label" to "Web browser"));refresh()}
                        }
                    }catch(cancelled:CancellationException){throw cancelled}
                    catch(e:Exception){state=state.copy(issue=e.message?:"Could not complete the passkey step. Try again.")}
                    finally{state=state.copy(busy=false);wake.trySend(Unit)}
                };return@command
            }
            "record_start"->{
                if(state.voice.phase!="Idle")return@command
                voiceTarget.start(fields);recordedLevels.clear();state=state.copy(voice=VoiceState(phase="Starting",peer=fields["peer"] as String))
                recordingJob=scope.launch {
                    try {browserVoiceStart().awaitBrowser<JsAny?>();state=state.copy(voice=state.voice.copy(phase="Recording"))}
                    catch(cancelled:CancellationException){throw cancelled}
                    catch(_:Exception){browserVoiceCancel();state=state.copy(voice=VoiceState(),issue="Allow microphone access to record a voice message.")}
                };return@command
            }
            "record_pause"->{runCatching {state=state.copy(voice=state.voice.copy(paused=browserVoicePause()))};return@command}
            "record_cancel"->{recordingJob?.cancel();browserVoiceCancel();pendingVoiceFile=null;recordedLevels.clear();voiceTarget.clear();state=state.copy(voice=VoiceState());return@command}
            "record_stop"->{stopRecording();return@command}
            "attachment_pick"->{
                val wallpaper=fields["kind"]=="Wallpaper"
                val selected=browserPickFile(fields["kind"]=="Photos" || wallpaper)
                scope.launch {try {selected.awaitBrowser<JsAny?>()?.let {if(wallpaper){mutex.withLock{browserWallpaperStage(fields["peer"] as String,it).awaitBrowser<JsAny?>();wallpaperRevision++}}else stage(it,fields)}}
                    catch(cancelled:CancellationException){throw cancelled}
                    catch(_:Exception){state=state.copy(issue="Could not import this attachment. Check access, available space and its size.")}}
                return@command
            }
            "wallpaper_remove"->{scope.launch {try{mutex.withLock{browserWallpaperStage(fields["peer"] as String,null).awaitBrowser<JsAny?>();wallpaperRevision++}}catch(cancelled:CancellationException){throw cancelled}catch(_:Exception){state=state.copy(issue="Could not remove this wallpaper. Try again.")}};return@command}

            "sign_out"->{if(state.voice.phase!="Idle"){state=state.copy(issue="Send or discard your recording before signing out.");return@command};signOut=true;signOutFailed=false;signOutSaved=false;return@command}
            "edit_source_used"->{state=state.copy(editDraft=null);return@command}
            "dismiss"->{workNotices.dismiss();state=state.copy(issue=null);return@command}
            "server_changed"->{state=state.copy(loginAddress=fields["server"] as String,loginMethods=null,discoveryIssue=null);return@command}
            "search"->{search(fields["query"] as String, fields["category"] as? String ?: "");return@command}
            "search_more"->{search(state.searchQuery, searchCategory, true);return@command}
            "close"->{state=state.copy(selected=null,messages=emptyList(),timelineLoaded=false);return@command}
            "open"->{state=state.copy(selected=fields["peer"] as String,messages=emptyList(),timelineLoaded=false);viewportEnd=0;timelineWant=0;timelineReloadAt=Int.MAX_VALUE;viewportChase=false;filter=mapOf("category" to (fields["category"] as? String ?: "Timeline"))+fields.filterKeys{it in setOf("author","message","thread_author","thread_message")};state=state.copy(threadTarget=(fields["thread_author"] as? String)?.let {a->(fields["thread_message"] as? String)?.let{ThreadTarget(a,it)}})}
            "timeline_filter"->{val next=fields.filter { (key,value)->key!="peer" && value!=null };if(filter==next)return@command;filter=next;viewportEnd=0;timelineWant=0;timelineReloadAt=Int.MAX_VALUE;viewportChase=false}
            "older"->{viewportEnd=maxOf(viewportEnd,timelineWant)}
            // Clearing history here as every other reset does: a viewport reported against it would re-deepen the target.
            "latest"->{state=state.copy(messages=emptyList(),timelineLoaded=false);viewportEnd=0;timelineWant=0;timelineReloadAt=Int.MAX_VALUE;viewportChase=false}
            // The list only reports where it is looking; the core decides when that needs more history.
            "viewport"->{
                val end=(fields["end"] as? Int) ?: 0
                if(fields["peer"]!=null && fields["peer"]!=state.selected)return@command
                if(end<=viewportEnd)return@command
                viewportEnd=end
                if(!state.more || end<timelineReloadAt)return@command
                // One scan at a time: a running load reads the viewport again on every page.
                if(timelineJob?.isActive==true) {viewportChase=true;return@command}
                viewportChase=false
                timelineJob=scope.launch {mutex.withLock {runCatching {
                    do {viewportChase=false;timeline()} while(viewportChase && state.more && viewportEnd>=timelineReloadAt)
                }}}
                return@command
            }
        }
        if(!ready)return@command
        if(name=="post" && sending)return@command
        if(name=="post")sending=true
        if(name=="group_create" && creatingGroup)return@command
        if(name=="group_create")creatingGroup=true
        scope.launch {
            try {mutex.withLock {
                state=state.copy(busy=name !in setOf("typing","read","draft","open"),issue=null)
                when(name) {
                    "delete_conversation"->{
                        if(fields["leave"]==true)execute("leave_group",mapOf("peer" to fields["peer"]))
                        execute("clear_conversation",mapOf("peer" to fields["peer"]))
                        if(state.selected==fields["peer"])state=state.copy(selected=null,messages=emptyList())
                        refresh()
                    }
                    "create_collection"->{
                        val id=browserRequestId()
                        execute("organize",mapOf("peer" to null,"value" to mapOf("Collection" to mapOf("id" to id,"name" to fields["name"],"present" to true))))
                        execute("organize",mapOf("peer" to null,"value" to mapOf("UiSetting" to mapOf("key" to "collection_icon.$id","value" to fields["icon"]))))
                        (fields["peers"] as? List<*>)?.filterIsInstance<String>()?.forEach {execute("organize",mapOf("peer" to it,"value" to mapOf("CollectionMember" to mapOf("id" to id,"present" to true))))}
                        refresh()
                    }
                    "open","timeline_filter","older","latest"->timeline()

"contact_qr"->{
    val context=mapOf("peer" to (fields["peer"]?:contactQr?.optional("peer")),"review" to (fields["review"]?:contactQr?.optional("review"))).filterValues{it!=null}
    if(fields["action"]=="scan" && fields["qr"]==null)contactQr=(json(context+mapOf("stage" to "scan")) as JsonObject)
    else {val value=execute(name,context+fields);value.optional("open")?.let{state=state.copy(selected=it);viewportEnd=0;timelineWant=0;timelineReloadAt=Int.MAX_VALUE};contactQr=value.takeUnless{it.string("stage") in listOf("done","none")}?.let{JsonObject(it+(json(context) as JsonObject))};refresh()}
}
                    "discover"->{
                        val address=fields["server"] as String
                        if(address==state.loginAddress) {
                            state=state.copy(discovering=true)
                            val value=execute(name,fields)
                            if(address==state.loginAddress)state=state.copy(loginMethods=LoginMethods(value.string("server_name"),value.bool("sso"),value.bool("password"),value.bool("invitation")))
                        }
                    }
                    "attachment_open","wallpaper_choose"->state=state.copy(issue="This browser integration is not available yet.")
                    else->{
                        if(name in setOf("file_send","file_cancel")){fileNext=0;fileWake.trySend(Unit)}
                        val raw=when {
                            name=="post" && post?.first==fields->post!!.second
                            name=="group_create" && groupCreate?.first==fields->groupCreate!!.second
                            else->request(name,fields).also {if(name=="post")post=fields.toMap() to it;if(name=="group_create")groupCreate=fields.toMap() to it}
                        }
                        val value=native(raw)
                        if(name=="group_create")groupCreate=null
                        if(name in setOf("storage","recovery_policy"))storage(value)
                        if(name in setOf("recover","reset_identity")){fileNext=0;fileWake.trySend(Unit)}
                        if(name in setOf("account_access","acknowledge_access","oidc_account","callback")){account(value);accessNext=0}
                        value.optional("authorization_url")?.let {url->authorization=url;browserSsoNavigate(url)}
                        if(name=="cancel_login"){authorization=null;browserSsoClose()}
                        value.optional("open")?.let {state=state.copy(selected=it);viewportEnd=0;timelineWant=0;timelineReloadAt=Int.MAX_VALUE}
                        if(name in setOf("post","edit","file_send")) {state=state.copy(sent=state.sent+1,sentText=(fields["text"]?:fields["caption"]) as? String,sentMessage=if(name=="post")Json.parseToJsonElement(raw).jsonObject.string("request")else null);post=null}
                        if(name in setOf("photo_publish","photo_retry","photo_cancel"))photoRevision++
                        if(name in setOf("profile","set_profile"))state=state.copy(profileName=value.string("display_name"),profileRevision=value.long("revision"))
if(name=="edit_source")state=state.copy(editDraft=EditDraft(fields["peer"] as String,fields["author"] as String,fields["message"] as String,value.string("edit_source")))
if(name in setOf("devices","revoke_device"))state=StateDecoder.devices(value,state,fields["cursor"]!=null)
if(name=="contact_policy")state=state.copy(allowRequests=value.bool("enabled"))
                        if(name in setOf("post","edit"))timeline() else if(name in setOf("file_send","file_cancel"))transfers() else refresh()
                        if(name=="post"){val flush=execute("flush");if(flush.long("sent")>0)timeline();flush.optional("issue")?.let {syncIssue=it;state=state.copy(issue=workNotices.update(state.issue,"sync",it))}}
                    }
                }
            }}catch(e:Exception) {if(name=="oidc")browserSsoClose();state=state.copy(searching=if(name in setOf("search","search_more"))false else state.searching,issue=e.message?:"Could not complete this action. Your draft is preserved.")}
            finally {state=state.copy(busy=false,discovering=false);if(name=="post")sending=false;if(name=="group_create")creatingGroup=false;if(name !in setOf("open","older","latest","timeline_filter","search","search_more","discover","storage","devices","profile"))wake.trySend(Unit)}
        }
    }
    LaunchedEffect(ready,visible,locations.live,locations.completion) {
        if(!ready){locations.stop();locations.live=false;return@LaunchedEffect}
        val updating=visible && locations.live
        if(!updating) {locations.stop();locations.live=false}
        do {
            try {
            var after:String?=null
            var active=0
            do {
                val result=mutex.withLock {execute("location_work",mapOf("after" to after,"point" to if(updating)locations.fresh() else null,"stop" to !updating))}
                active+=result.long("active").toInt()
                after=result.optional("next")
                if(result.long("queued")>0)wake.trySend(Unit)
            }while(after!=null)
            if(!updating || active==0){locations.live=false;locations.stop();break}
            if(locations.issue!=null){locations.live=false;break}
            }catch(cancelled:CancellationException){throw cancelled}
            catch(_:Exception){state=state.copy(issue="Location updates are waiting to sync.")}
            delay(5000)
        }while(isActive)
    }
    DisposableEffect(Unit){onDispose {locations.stop()}}
    CompositionLocalProvider(LocalMediaCommand provides command, LocalMediaSender provides {message->state.people[message.author] ?: if(message.mine)"You" else state.chats.firstOrNull {it.id==message.peer}?.name ?: ""}, LocalMediaMessage provides {peer,author,id->state.messages.firstOrNull {it.peer==peer && it.author==author && it.id==id}}, LocalLocationContent provides {message,part,action->WebLocationCard(message,part,state.people[message.author] ?: if(message.mine)"You" else "Shared place",action)},LocalPlacePanel provides {target,back,done->WebPlacePanel(target,locations,back,photo=state.profileAvatar) {fields->mutex.withLock {execute("place",fields);locations.shared(if(fields["live"]!=null)"live" else if(fields["pin"]==true)"pin" else "once",visible);runCatching {timeline()}};wake.trySend(Unit);done()}},LocalNotificationPanel provides {value,action->WebNotificationSettings(value,action)},LocalWallpaper provides {peer,modifier->WebWallpaper(peer,wallpaperRevision,modifier)},LocalWebFileSave provides ::saveFile,LocalMaterialPlatform provides WebMaterials,LocalSolidMaterial provides (if(materialsReady) {value,progress,modifier->MaterialMessages(value,progress,modifier)} else null),LocalMaterialOverlay provides (if(materialsReady) {timeline,modifier->MaterialTimelineOverlay(timeline,modifier)} else null),LocalProfilePhoto provides {reference,modifier->WebProfilePhoto(reference,photoRevision,modifier)},LocalMotionVisible provides visible,LocalClientFeatures provides ClientFeatures(calls=calls.available,videoCalls=calls.available && videoReady,files=true,voice=true,locations=true,notifications=notificationsReady,recovery=true,passkeys=passkeysReady,screenShare=false,audioRoute=false),LocalAttachmentThumbnail provides {message,modifier->WebThumbnail(message,::loadFile,modifier)},LocalKeepScreenAwake provides {WebKeepAwake(it)},LocalSystemReducedMotion provides rememberWebReducedMotion(),LocalSystemAppearance provides ::webColorScheme,LocalServiceAccess provides ::service,LocalDraftId provides ::browserRequestId,LocalRecipeScale provides ::recipe,LocalTemporalPreview provides {kind,input->val result=rustTemporal("$kind\n${(BrowserDate.now()/1000).toLong()}\n$timezone\n$dateOrder\n$input").split('\n');if(result.size!=3)null else result[1].toLongOrNull()?.let {TemporalPreview(result[0],timezone,if(kind=="Timer")"$it seconds" else calendar(it)+" · "+timezone)}},LocalCameraPanel provides {target,back,done->WebCameraPanel(back) {file->stage(file,target);done()}},LocalCallVideo provides {member,screen,modifier->WebCallVideo(calls,member,screen,modifier)},LocalAttachmentContent provides {message->message.webFile()?.let {WebAttachment(it,::loadFile,{viewing=it},outgoing=message.mine)}},LocalAttachmentDraft provides {file,modifier->WebAttachment(WebFile(file.peer,"","",file.name,file.mediaType,file.bytes,draft=file.request,levels=voiceWaves[file.request]?.first.orEmpty(),duration=voiceWaves[file.request]?.second ?: 0),::loadFile,{viewing=it},modifier)},LocalBuilderSource provides ::rustBuilder,LocalStructuredPreview provides {source->runCatching {ContentDecoder.part(rustPreview(buildJsonObject {put("source",source);put("now",(BrowserDate.now()/1000).toLong());put("timezone",timezone)}.toString()),::clock)}.getOrNull()},LocalBuilderTimezone provides timezone,LocalCodePreview provides ::rustCode,LocalEditorAnalysis provides ::rustEditor,LocalHelpCatalog provides ::rustHelp,LocalTextMotionSeeds provides ::rustMotionSeeds) {
        SigilTheme(decodeAppearance(state.ui["appearance"]),palette=::rustPalette) { Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
if(signOut)AlertDialog(onDismissRequest={if(!state.busy)signOut=false},title={Text("Sign out of this browser?")},text={Column {
    Text("This removes this browser's messages, drafts, keys and settings. Other devices remain signed in. Unsynced changes will be lost.")
    Row(verticalAlignment=Alignment.CenterVertically){Checkbox(signOutSaved,{signOutSaved=it},enabled=!state.busy);Text("I have saved what I need, or accept losing this browser's history.",Modifier.weight(1f))}
    if(signOutFailed)Text("Server revocation was not confirmed. Retry, or remove local data and revoke this device from another signed-in device.")
}},confirmButton={SigilTextButton(onClick={scope.launch {mutex.withLock {
    state=state.copy(busy=true)
    try {if(notificationsReady)webNotificationsDisable().awaitBrowser<JsAny?>();calls.close();execute("browser_sign_out");finishRemoval()}
    catch(_:Exception){signOutFailed=true;state=state.copy(busy=false)}
}}},enabled=signOutSaved&&!state.busy){Text(if(signOutFailed)"Retry revocation" else "Sign out")}},dismissButton={Row {
    if(signOutFailed)SigilTextButton(onClick={scope.launch {mutex.withLock {
        state=state.copy(busy=true)
        try {if(notificationsReady)webNotificationsDisable().awaitBrowser<JsAny?>();execute("browser_erase");finishRemoval()}
        catch(_:Exception){state=state.copy(busy=false,issue="Local removal could not finish. Reload to retry any pending removal.")}
    }}},enabled=signOutSaved&&!state.busy){Text("Remove local data")}
    SigilTextButton({signOut=false},enabled=!state.busy){Text("Cancel")}
}})
            startupError?.let {Text(it,Modifier.padding(24.dp))}
Box(Modifier.weight(1f).fillMaxWidth(),contentAlignment=Alignment.Center) {
    val qr=linking?:contactQr
    if(qr!=null) {
        val device=linking!=null
        val close={if(!state.busy)command(if(device)"device_link" else "contact_qr",mapOf("action" to if(!device || qr.string("stage")=="done")"close" else if(qr.bool("can_cancel",true))"cancel" else "pause"))}
        Box(Modifier.widthIn(max=600.dp).fillMaxWidth().fillMaxHeight(.94f).onPreviewKeyEvent {if(it.type==KeyEventType.KeyDown && it.key==Key.Escape){close();true}else false}) {
            if(device)LinkPanel(qr.toString(),linkingBusy,linkingIssue,{action,payload->command("device_link",mapOf("action" to action,(if(action=="confirm")"choice" else "qr") to payload))}) {found->WebQrScanner(found)}
            else ContactPanel(qr.toString(),state.busy,state.issue,{action,payload->command("contact_qr",mapOf("action" to action,"qr" to payload))}) {found->WebQrScanner(found)}
        }
    } else screens.SaveableStateProvider("messenger") {
        Box((if(state.phase=="connected")Modifier.fillMaxSize() else Modifier.widthIn(max=540.dp).fillMaxSize()).then(if(viewing!=null)Modifier.blur(16.dp)else Modifier)) {
            SigilApp(::rustPalette,::rustAnalyze,state.copy(calls=calls.history,call=calls.visible),command,wideLayout=true,read={window.localStorage.getItem("messenger.$it")},write={k,v->window.localStorage.setItem("messenger.$k",v)})
        }
    }
    viewing?.let { file ->
        val chat=state.chats.firstOrNull {it.id==file.peer}
        SigilTheme(decodeAppearance(state.ui["appearance"]),decodeChat(chat?.ui?.get("chat_theme") ?: window.localStorage.getItem("messenger.chat.${file.peer}")),palette=::rustPalette,chatKey=file.peer) {
            Dialog({viewing=null},DialogProperties(usePlatformDefaultWidth=false)) {
                val gallery=remember(state.messages,file.peer) {state.messages.asReversed().filter {it.peer==file.peer && it.attachment?.mediaType?.let {type->type.startsWith("image/") || type.startsWith("video/")}==true}.mapNotNull {it.webFile()}}
                WebFileViewer(file,::loadFile,{viewing=null},gallery) {viewing=it}
            }
        }
    }
}
        }}
    }
}
