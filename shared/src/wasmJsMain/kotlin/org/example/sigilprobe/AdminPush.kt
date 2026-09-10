package org.sigil

import androidx.compose.runtime.*
import androidx.compose.foundation.layout.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.serialization.json.*

@Composable
internal fun AdminPush() {
    var configuration by remember { mutableStateOf<JsonObject?>(null) }
    fun load(value:JsonElement):PushConfiguration {
        val data=value.jsonObject
        fun text(key:String)=data[key]?.jsonPrimitive?.contentOrNull.orEmpty()
        val result=PushConfiguration(text("revision"),data.getValue("unified_push").jsonPrimitive.boolean,
            text("contact"),text("fcm_project_id"),text("fcm_client_email"),text("vapid_public_key"))
        configuration=data
        return result
    }
    Column(Modifier.fillMaxWidth(),verticalArrangement=Arrangement.spacedBy(24.dp)) {
    PushSettings(read={load(api("/admin/v0/push"))},save={update->
        load(api("/admin/v0/push","PUT",pushConfigurationRequest(checkNotNull(configuration),update)))
    },field={label,value,change,secret,enabled->Field(label,value,change,secret,enabled)})
    configuration?.get("fcm_project_id")?.jsonPrimitive?.contentOrNull?.let {project->
        key(configuration?.get("revision")) {
        var android by remember {mutableStateOf<JsonObject?>(null)}
        fun androidLoaded(value:JsonElement):AndroidPushConfiguration {
            android=value.jsonObject
            val config=android?.get("android")?.takeUnless {it==JsonNull}?.jsonObject
            return AndroidPushConfiguration(config?.get("application_id")?.jsonPrimitive?.content.orEmpty())
        }
        AndroidPushSettings(project,read={androidLoaded(api("/admin/v0/push/android"))},save={source->
            val previous=checkNotNull(android)
            androidLoaded(api("/admin/v0/push/android","PUT",buildJsonObject {
                put("expected_revision",previous.getValue("revision"));put("expected_push_revision",previous.getValue("push_revision"))
                put("android",source?.let {androidFirebaseRequest(it,project)} ?: JsonNull)
            }))
        },field={label,value,change,secret,enabled->Field(label,value,change,secret,enabled)})
        }
    }
    }
}

internal fun androidFirebaseRequest(source:String,project:String):JsonObject = try {
    require(source.length<=32768)
    val root=Json.parseToJsonElement(source).jsonObject
    val info=root.getValue("project_info").jsonObject
    require(info.getValue("project_id").jsonPrimitive.content==project)
    val client=root.getValue("client").jsonArray.single {
        it.jsonObject["client_info"]?.jsonObject?.get("android_client_info")?.jsonObject?.get("package_name")?.jsonPrimitive?.content=="org.sigil.compose"
    }.jsonObject
    fun string(value:JsonElement):JsonPrimitive {val v=value.jsonPrimitive;require(v.isString && v.content.isNotBlank());return v}
    buildJsonObject {
        put("project_id",string(info.getValue("project_id")))
        put("sender_id",string(info.getValue("project_number")))
        put("application_id",string(client.getValue("client_info").jsonObject.getValue("mobilesdk_app_id")))
        put("api_key",string(client.getValue("api_key").jsonArray.single().jsonObject.getValue("current_key")))
    }
} catch(_:Exception) {throw IllegalArgumentException("Use google-services.json for org.sigil.compose in project $project. Its contents have not been sent.")}

internal fun pushConfigurationRequest(previous:JsonObject,update:PushUpdate):JsonObject {
    check(previous.getValue("revision").jsonPrimitive.content==update.revision) { "Settings changed. Reload before saving." }
    val fcm=buildJsonObject {
        put("action",when {update.disableGoogle->"disable";update.credentials!=null->"configure";else->"keep"})
        if(!update.disableGoogle) update.credentials?.let { source->
            val credentials=try {
                require(source.length<=32768)
                val parsed=Json.parseToJsonElement(source).jsonObject
                require(parsed["type"]?.jsonPrimitive?.content=="service_account")
                buildJsonObject {
                    for(key in listOf("project_id","client_email","private_key")) {
                        val value=parsed.getValue(key).jsonPrimitive
                        require(value.isString && value.content.isNotBlank())
                        put(key,value)
                    }
                }
            } catch(_:Exception) { throw IllegalArgumentException("Paste a complete Firebase service-account JSON key. Its contents have not been sent.") }
            put("credentials",credentials)
        }
    }
    return buildJsonObject {
        put("expected_revision",previous.getValue("revision"));put("unified_push",update.unified)
        put("contact",update.contact.takeIf {it.isNotBlank()}?.let(::JsonPrimitive) ?: JsonNull)
        put("exceptions",previous.getValue("exceptions"));put("rotate_vapid",update.rotate);put("fcm",fcm)
    }
}
