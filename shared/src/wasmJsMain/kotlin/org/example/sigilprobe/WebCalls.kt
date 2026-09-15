@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)
package org.sigil

import androidx.compose.runtime.*
import kotlinx.coroutines.*
import kotlinx.browser.window
import kotlinx.serialization.json.*
import kotlin.js.*

@JsName("Uint8Array") private external class CallBytes(value:JsAny):JsAny {fun fill(value:Int)}
@JsName("Date") private external object CallClock:JsAny {fun now():Double}

internal class WebCalls(private val scope:CoroutineScope,private val command:suspend(String,Map<String,Any?>)->JsonObject,private val wake:()->Unit,private val issue:(String)->Unit) {
    var history by mutableStateOf<List<CallSummary>>(emptyList());private set
    var visible by mutableStateOf<ActiveCall?>(null);private set
    var available by mutableStateOf(false);private set
    private var desired:String?=null
    private var generation=0
    private var starting=false
    private var connecting=false
    private var connected:String?=null
    private var members=listOf<String>()
    private var audio:WebAudioEngine?=null
    private var muted=false
    private var start=0.0
    private var retry=0.0
    private var failures=0
    private var pumping:Job?=null
    private var maintenance:Job?=null
    suspend fun initialize(){try{initializeAudioWasm().awaitBrowser<JsAny?>();available=webAudioSupported().awaitBrowser<JsBoolean>().toBoolean()}catch(_:Exception){available=false}}
    private suspend fun control(operation:String,call:String?=null,tracks:Boolean=false):JsonObject {
        val raw=buildJsonObject {put("operation",operation);call?.let{put("call",it)};if(tracks)put("tracks",buildJsonObject{put("audio",!muted);put("camera",false);put("screen",false)})}
        return Json.parseToJsonElement(browserCallControl(raw.toString()).awaitBrowser<JsString>().toString()).jsonObject
    }
    fun refresh(value:JsonObject,clock:(Long)->String,day:(Long)->String={""}){
        history=value["calls"]?.jsonArray?.map {item->val call=item.jsonObject;CallSummary(call.string("id"),call.string("phase"),call.bool("direct"),call.long("created"),call["participants"]!!.jsonArray.map {entry->val person=entry.jsonObject;CallParticipant(person.string("member"),person.string("peer"),person.string("name"),person.bool("own"),person.bool("verified"),person.bool("audio"),person.bool("camera"),person.bool("screen"),person.string("fingerprint"),person.string("address"))},call.bool("can_invite"),call.string("name"),call.bool("outgoing"),clock(call.long("created")),day(call.long("created")),call.bool("missed"),call["duration"]?.jsonPrimitive?.longOrNull,call["video"]?.jsonPrimitive?.booleanOrNull)}.orEmpty()
        val current=desired?.let {id->history.find {it.id==id}}?:history.firstOrNull {it.phase=="ringing"}
        if(current!=null && current.phase !in setOf("active","joining","ringing")){close();return}
        visible=current?.let{ActiveCall(it,it.name.ifBlank{it.participants.filterNot{p->p.own}.joinToString(", "){p->p.name}.ifBlank{"Call"}},visible?.connection?:"connecting",if(start==0.0)0 else ((window.performance.now()-start)/1000).toLong(),muted)}
        if(current?.phase=="active" && desired==current.id && !connecting && connected!=current.id && CallClock.now()>=retry)connect(current)
    }
    private fun createAudio(){
        if(audio!=null)return
        val current=generation
        audio=browserBoundary {WebAudioEngine {timestamp,bytes->
            if(current==generation && connected!=null && !muted){
                val copy=CallBytes(bytes)
                scope.launch {try{browserCallSend(0,timestamp,false,copy).awaitBrowser<JsBoolean>()}catch(_:Exception){}finally{copy.fill(0)}}
            }
        }
    }
    }
    private fun connect(call:CallSummary){
        connecting=true;val current=generation
        scope.launch {
            try {
                members=call.participants.filterNot{it.own}.map{it.id}
                browserCallConnect(call.id){sender,frame->if(current==generation){val index=members.indexOf(sender);if(index>=0)runCatching{audio?.receive_frame(index,frame)}}}.awaitBrowser<JsAny?>()
                check(current==generation){"Call changed"}
                control("call_start",call.id,true)
                check(current==generation){"Call changed"}
                connected=call.id;failures=0
                maintenance?.cancel()
                maintenance=scope.launch {
                    while(isActive && current==generation && desired==call.id){
                        val state=browserCallTransportState()
                        if(state in setOf("failed","closed","disconnected")){reconnect();break}
                        val receivers=runCatching{control("call_refresh",call.id).long("receivers")}.getOrDefault(0)
                        if(state=="connected" && receivers>=members.size && members.isNotEmpty()){
                            if(start==0.0)start=window.performance.now()
                            visible=visible?.copy(connection="connected",seconds=((window.performance.now()-start)/1000).toLong())
                            audio?.mute(muted)
                            if(pumping==null)pumping=scope.launch {
                                try{while(isActive && current==generation){audio?.let{check(!it.failed()){"Audio device failed"};it.pump()};delay(10)}}catch(cancelled:CancellationException){throw cancelled}catch(_:Exception){issue("The audio device stopped. The call has ended.");handle("call_end",emptyMap())}
                            }
                        }else {audio?.mute(true);visible=visible?.copy(connection="securing")}
                        wake();delay(if(receivers<members.size)250 else 1000)
                    }
                }
            }catch(cancelled:CancellationException){throw cancelled}
            catch(_:Exception){if(current==generation){reconnect();issue("Could not connect call media. Retrying…")}}
            finally{if(current==generation)connecting=false}
        }
    }
    private fun reconnect(){audio?.mute(true);pumping?.cancel();pumping=null;connected=null;runCatching{browserCallClose()};failures++;retry=CallClock.now()+(1000L shl failures.coerceAtMost(3));visible=visible?.copy(connection="reconnecting")}
    fun handle(action:String,fields:Map<String,Any?>):Boolean {
        if(!action.startsWith("call_"))return false
        when(action){
            "call_mute"->{muted=!muted;audio?.mute(muted);visible=visible?.copy(muted=muted);desired?.let{id->scope.launch{runCatching{control("call_tracks",id,true)};wake()}}}
            "call_end"->{val id=desired?:fields["call"] as? String;close();if(id!=null)scope.launch{runCatching{command("call_leave",mapOf("call" to id))}.onFailure{issue("Call ended locally; the server update is pending.")};wake()}}
            "call_decline"->scope.launch{runCatching{command("call_answer",mapOf("call" to fields["call"],"accept" to false))}.onFailure{issue("Could not decline the call. Try again.")};wake()}
            "call_start","call_redial","call_answer","call_prepare","call_resume"->{
                if(!available || starting || desired!=null)return true
                if(fields["video"]==true){issue("Browser video calls are not connected yet.");return true}
                starting=true;generation++;val current=generation;muted=false;start=0.0
                scope.launch {
                    try{
                        createAudio()
                        withTimeout(60000) {
                            while((audio?.count()?:0.0)==0.0) {
                                check(audio?.failed()!=true){"Microphone unavailable"}
                                delay(50)
                                check(current==generation){"Call changed"}
                            }
                        }
                        val id=if(action in setOf("call_start","call_redial")){command(action,mapOf((if(action=="call_start")"peer" else "call") to fields[if(action=="call_start")"peer" else "call"],"request" to browserRequestId(),"timestamp" to (CallClock.now()/1000).toLong())).string("call")}
                        else (fields["call"] as String).also {if(action=="call_answer")command("call_answer",mapOf("call" to it,"accept" to true))}
                        if(current!=generation){command("call_leave",mapOf("call" to id));return@launch}
                        desired=id;wake()
                    }catch(cancelled:CancellationException){throw cancelled}
                    catch(_:Exception){if(current==generation){close();issue("Could not start the call. Check microphone permission and try again.")}}
                    finally{if(current==generation)starting=false}
                }
            }
            "call_invite"->scope.launch{runCatching{command(action,fields)}.onFailure{issue("Could not invite this person.")};wake()}
            else->issue("This call control is not available in this browser yet.")
        }
        return true
    }
    fun close(){val stopped=desired;val duration=if(start>0)((window.performance.now()-start)/1000).toLong() else null;generation++;val stoppedGeneration=generation;starting=false;connecting=false;desired=null;connected=null;visible=null;start=0.0;retry=0.0;failures=0;pumping?.cancel();pumping=null;maintenance?.cancel();maintenance=null;audio?.let{it.close();it.free()};audio=null;runCatching{browserCallClose()};scope.launch{if(stopped!=null && duration!=null)runCatching{command("call_history_media",mapOf("call" to stopped,"duration" to duration,"video" to false))};if(generation==stoppedGeneration && stopped!=null)runCatching{control("call_stop",stopped)}}}
}
