@file:JsModule("./sigil_browser.js")
@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)
package org.sigil
import kotlin.js.*
@JsName("default") external fun initializeBrowser():Promise<JsAny>
@JsName("start_browser") external fun startBrowser():Promise<JsAny>
@JsName("browser_command") external fun browserCommand(request:String):Promise<JsString>
@JsName("request_id") external fun browserRequestId():String
@JsName("camera_start") external fun browserCameraStart(video:org.w3c.dom.HTMLVideoElement):Promise<JsAny>
@JsName("camera_stop") external fun browserCameraStop()
@JsName("camera_scan") external fun browserCameraScan():String?
@JsName("pick_file") external fun browserPickFile(photos:Boolean):Promise<JsAny?>
@JsName("file_metadata") external fun browserFileMetadata(file:JsAny):String
@JsName("file_slice") external fun browserFileSlice(file:JsAny,index:Int):Promise<JsAny>
@JsName("file_stage") external fun browserFileStage(request:String,index:Int,bytes:JsAny):Promise<JsAny>
@JsName("file_read") external fun browserFileRead(peer:String,author:String,message:String,index:Int):Promise<JsAny>
@JsName("release_bytes") external fun browserReleaseBytes(bytes:JsAny)
@JsName("file_url") external fun browserFileUrl(peer:String,author:String,message:String,draft:String,length:Double,mediaType:String):Promise<JsString>
@JsName("revoke_file_url") external fun browserRevokeFileUrl(url:String)
@JsName("save_file_url") external fun browserSaveFileUrl(url:String,name:String)
@JsName("voice_start") external fun browserVoiceStart():Promise<JsAny>
@JsName("voice_pause") external fun browserVoicePause():Boolean
@JsName("voice_level") external fun browserVoiceLevel():Float
@JsName("voice_finish") external fun browserVoiceFinish():Promise<JsAny>
@JsName("voice_cancel") external fun browserVoiceCancel()
@JsName("camera_start_photo") external fun browserCameraStartPhoto(video:org.w3c.dom.HTMLVideoElement,front:Boolean):Promise<JsAny>
@JsName("camera_photo") external fun browserCameraPhoto():Promise<JsAny>
@JsName("camera_photo_url") external fun browserCameraPhotoUrl(file:JsAny):String
@JsName("render_math") external fun browserRenderMath(target:org.w3c.dom.HTMLElement,mathml:String)
@JsName("profile_image") external fun browserProfileImage(reference:String):Promise<JsString>
@JsName("profile_stage") external fun browserProfileStage(file:JsAny?):Promise<JsAny>
@JsName("wallpaper_stage") external fun browserWallpaperStage(peer:String,file:JsAny?):Promise<JsAny?>
@JsName("wallpaper_image") external fun browserWallpaperImage(peer:String):Promise<JsString>

@JsName("file_stream_supported") external fun browserFileStreamSupported():Boolean
@JsName("file_destination") external fun browserFileDestination(name:String):Promise<JsAny?>
@JsName("file_save") external fun browserFileSave(handle:JsAny,peer:String,author:String,message:String,draft:String,length:Double):Promise<JsAny?>
@JsName("browser_call_connect") external fun browserCallConnect(call:String,frame:(String,JsAny)->Unit):Promise<JsAny>
@JsName("browser_call_control") external fun browserCallControl(request:String):Promise<JsString>
@JsName("browser_call_send") external fun browserCallSend(kind:Int,timestamp:Double,keyframe:Boolean,bytes:JsAny):Promise<JsBoolean>
@JsName("browser_call_close") external fun browserCallClose()
@JsName("browser_call_transport_state") external fun browserCallTransportState():String

@JsName("LocationWatch") external class BrowserLocationWatch(changed:(String)->Unit,failed:(String)->Unit):JsAny {
    fun free()
}

@JsName("map_resource") external fun browserMapResource(path:String):Promise<JsAny>
