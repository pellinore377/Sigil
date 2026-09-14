@file:JsModule("./sigil_browser_events.js")
@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)
package org.sigil
import kotlin.js.*
@JsName("default") external fun initializeNotificationWasm():Promise<JsAny?>
@JsName("notifications_supported") external fun webNotificationsSupported():Boolean
@JsName("notifications_permission") external fun webNotificationsPermission():String
@JsName("notifications_request") external fun webNotificationsRequest():Promise<JsAny?>
@JsName("notifications_subscribe") external fun webNotificationsSubscribe(vapid:String):Promise<JsString>
@JsName("notifications_pending") external fun webNotificationsPending():Promise<JsString>
@JsName("notifications_clear") external fun webNotificationsClear(payload:String):Promise<JsAny?>
@JsName("notifications_enabled") external fun webNotificationsEnabled():Promise<JsBoolean>
@JsName("notifications_disable") external fun webNotificationsDisable():Promise<JsAny?>
