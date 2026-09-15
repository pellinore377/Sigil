@file:JsModule("./sigil_browser_audio.js")
@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)
package org.sigil
import kotlin.js.*
@JsName("default") external fun initializeAudioWasm():Promise<JsAny>
@JsName("AudioEngine") external class WebAudioEngine(encoded:(Double,JsAny)->Unit):JsAny {
    fun pump():Boolean
    fun mute(value:Boolean)
    fun count():Double
    fun decoded_samples():Double
    fun failed():Boolean
    fun receive_frame(peer:Int,frame:JsAny):Boolean
    fun close()
    fun free()
}

@JsName("audio_supported") external fun webAudioSupported():Promise<JsBoolean>
