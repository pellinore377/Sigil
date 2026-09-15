@file:JsModule("./sigil_browser_maps.js")
@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)
package org.sigil
import kotlin.js.*
import org.w3c.dom.HTMLCanvasElement
@JsName("default") external fun initializeMapsWasm():Promise<JsAny>
@JsName("create_map") external fun createWebMap(canvas:HTMLCanvasElement,dark:Boolean,sans:Boolean):Promise<WebMapRenderer>
@JsName("MapRenderer") external class WebMapRenderer:JsAny {
    fun configure(resource:JsAny)
    fun metadata(resource:JsAny):String
    fun view(lat:Double,lon:Double,zoom:Int,width:Int,height:Int,dpr:Double):String
    fun tile(z:Int,x:Int,y:Int,resource:JsAny)
    fun point(x:Double,y:Double):String
    fun project(lat:Double,lon:Double):String
    fun snapshot():String
    fun free()
}
