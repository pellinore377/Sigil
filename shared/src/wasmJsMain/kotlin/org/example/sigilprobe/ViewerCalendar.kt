@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)
package org.sigil

@JsName("Date") private external class ViewerDate(value:Double):JsAny {fun getDay():Int}
@JsName("Intl") private external object ViewerIntl:JsAny {fun DateTimeFormat():ViewerFormat}
private external interface ViewerFormat:JsAny {fun resolvedOptions():ViewerOptions}
private external interface ViewerOptions:JsAny {val timeZone:String?}

internal actual fun viewerWeekday(at:Long):Int=runCatching {ViewerDate(at*1000.0).getDay().let {if(it==0)7 else it}}.getOrDefault(0)
internal actual fun viewerZone():String=runCatching {ViewerIntl.DateTimeFormat().resolvedOptions().timeZone.orEmpty()}.getOrDefault("")
