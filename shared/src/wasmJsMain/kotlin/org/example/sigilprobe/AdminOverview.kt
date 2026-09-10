@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)
package org.sigil

import androidx.compose.runtime.*
import kotlinx.browser.document
import kotlinx.browser.window
import kotlinx.coroutines.*
import kotlinx.serialization.json.*
import kotlin.js.*

private external interface VisibleDocument : JsAny { val hidden:Boolean }

private fun diagnostics(data:JsonElement):OperationalSample {
    fun JsonElement.count(key:String)=jsonObject.getValue(key).jsonPrimitive.long.also { require(it>=0) }
    fun n(key:String)=data.count(key)
    val maintenance=data.jsonObject.getValue("maintenance")
    val backup=maintenance.jsonObject["last_backup_at"]?.jsonPrimitive?.longOrNull?.also { require(it>=0) }
    return OperationalSample(n("as_of"),n("accounts"),n("devices"),n("mailbox_pending"),n("federation_pending"),n("push_pending"),
        n("federation_failed_peers"),n("push_retries"),n("push_invalid"),n("database_bytes"),n("attachment_bytes"),n("recovery_bytes"),
        maintenance.count("queued_or_running"),maintenance.count("failed"),backup,maintenance.jsonObject.getValue("restore_pending").jsonPrimitive.boolean,
        data.jsonObject.getValue("version").jsonPrimitive.content,n("schema"))
}

@Composable
internal fun AdminOverview(navigate:(String)->Unit) {
    var samples by remember { mutableStateOf(emptyList<OperationalSample>()) }
    var loading by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    var lastReceived by remember { mutableDoubleStateOf(0.0) }
    var stale by remember { mutableStateOf(false) }
    val scope=rememberCoroutineScope()
    suspend fun refresh() {
        if(loading)return
        loading=true
        try {
            val sample=diagnostics(api("/admin/v0/diagnostics"))
            samples=observationWindow(samples,sample)
            lastReceived=window.performance.now();stale=false;error=""
        } catch(cancelled:CancellationException) { throw cancelled }
        catch(failure:Exception) { error=failure.message ?: "Could not refresh server observations." }
        finally { loading=false }
    }
    LaunchedEffect(Unit) {
        while(isActive) {
            if(!document.unsafeCast<VisibleDocument>().hidden)refresh()
            stale=lastReceived>0 && window.performance.now()-lastReceived>45000
            delay(15000)
        }
    }
    OperationalDashboard(samples,loading,error,stale,{scope.launch { refresh() }},navigate)
}
