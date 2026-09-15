@file:JsModule("./sigil_materials.js")
@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)
package org.sigil
import kotlin.js.*
@JsName("default") external fun initializeMaterialWasm():Promise<JsAny>
@JsName("initialize_materials") external fun initializeMaterialGpu():Promise<JsBoolean>
@JsName("material_record_async") external fun browserMaterialRecord(value:String):Promise<JsString>
@JsName("material_shutdown") external fun browserMaterialShutdown()
@JsName("material_extent") external fun browserMaterialExtent(sides:Int,face:Int,rotation:String,outgoing:Boolean):Float
@JsName("material_view_available") external fun browserMaterialViewAvailable():Boolean
@JsName("MaterialView") external class BrowserMaterialView:JsAny {
    constructor(canvas:org.w3c.dom.HTMLCanvasElement)
    fun draw(frame:String):Boolean
    fun free()
}
