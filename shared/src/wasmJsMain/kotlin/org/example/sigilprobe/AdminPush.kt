package org.sigil

import androidx.compose.runtime.*
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
    PushSettings(read={load(api("/admin/v0/push"))},save={update->
        load(api("/admin/v0/push","PUT",pushConfigurationRequest(checkNotNull(configuration),update)))
    },field={label,value,change,secret,enabled->Field(label,value,change,secret,enabled)})
}

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
