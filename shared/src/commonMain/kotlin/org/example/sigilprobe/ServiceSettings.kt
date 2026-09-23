package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.launch
import kotlinx.serialization.json.*

internal data class ServicePreset(val id:String, val kind:String, val endpoint:String, val attribution:String, val source:String)

internal object ServicePresets {
    val definitions=ServicePreset("wiktionary","wiktionary","https://en.wiktionary.org/api/rest_v1/page/definition",
        "Definitions from Wiktionary, CC BY-SA 4.0","https://en.wiktionary.org/")
    val forecast=ServicePreset("open-meteo","open_meteo","https://api.open-meteo.com/v1/forecast",
        "Weather data by Open-Meteo.com, CC BY 4.0","https://open-meteo.com/")
    val places=ServicePreset("open-meteo-places","geocoder","https://geocoding-api.open-meteo.com/v1/search",
        "Place search by Open-Meteo.com with GeoNames data, CC BY 4.0","https://open-meteo.com/en/docs/geocoding-api")
    val libre=ServicePreset("libretranslate","libre_translate","https://libretranslate.com/translate",
        "Translated by LibreTranslate","https://libretranslate.com/")
    val google=ServicePreset("google-translate","google_translate","https://translation.googleapis.com/language/translate/v2",
        "Translated by Google","https://cloud.google.com/translate")
    val all=listOf(definitions,forecast,places,libre,google)
}

internal data class ServiceChoices(val definitions:Boolean=false, val weather:Boolean=false,
    val libre:Boolean=false, val libreEndpoint:String=ServicePresets.libre.endpoint, val libreKey:String="", val libreClear:Boolean=false,
    val google:Boolean=false, val googleKey:String="", val perAccount:String="100", val total:String="2000")

internal fun JsonElement.serviceEntry(id:String)=jsonObject["providers"]?.jsonArray?.firstOrNull {
    it.jsonObject["provider"]?.jsonObject?.get("id")?.jsonPrimitive?.content==id }?.jsonObject

internal fun JsonElement.serviceSecret(id:String)=serviceEntry(id)?.get("has_secret")?.jsonPrimitive?.booleanOrNull==true

internal fun serviceChoices(current:JsonElement):ServiceChoices {
    fun on(preset:ServicePreset)=current.serviceEntry(preset.id)!=null
    fun limit(key:String,fallback:String)=current.jsonObject[key]?.jsonPrimitive?.intOrNull?.takeIf { it>0 }?.toString() ?: fallback
    val defaults=ServiceChoices()
    return ServiceChoices(on(ServicePresets.definitions),on(ServicePresets.forecast) || on(ServicePresets.places),
        on(ServicePresets.libre),current.serviceEntry(ServicePresets.libre.id)?.get("provider")?.jsonObject?.get("endpoint")?.jsonPrimitive?.content ?: ServicePresets.libre.endpoint,
        "",false,on(ServicePresets.google),"",limit("per_account_daily",defaults.perAccount),limit("total_daily",defaults.total))
}

internal fun otherServiceProviders(current:JsonElement)=current.jsonObject["providers"]?.jsonArray.orEmpty().map { it.jsonObject }
    .filter { e-> ServicePresets.all.none { it.id==e["provider"]?.jsonObject?.get("id")?.jsonPrimitive?.content } }

internal fun serviceConfigurationRequest(current:JsonElement, choices:ServiceChoices):JsonObject {
    val perAccount=choices.perAccount.trim().toIntOrNull()
    val total=choices.total.trim().toIntOrNull()
    require(perAccount!=null && perAccount in 1..100000) { "Lookups per person must be between 1 and 100,000." }
    require(total!=null && total in 1..10000000) { "Lookups for the whole server must be between 1 and 10,000,000." }
    require(perAccount<=total) { "The server-wide limit must be at least the per-person limit." }
    fun entry(preset:ServicePreset, endpoint:String=preset.endpoint, secret:JsonObject=buildJsonObject { put("action","keep") }):JsonObject {
        val prior=current.serviceEntry(preset.id)
        val priorEndpoint=prior?.get("provider")?.jsonObject?.get("endpoint")?.jsonPrimitive?.content
        val action=if(secret["action"]?.jsonPrimitive?.content=="keep" && (prior==null || priorEndpoint!=endpoint))
            buildJsonObject { put("action","clear") } else secret
        return buildJsonObject {
            put("provider",buildJsonObject {
                put("id",preset.id);put("kind",preset.kind);put("endpoint",endpoint)
                put("attribution",buildJsonObject { put("body",preset.attribution);put("spans",JsonArray(emptyList())) })
                put("version",JsonNull);put("source_url",preset.source)
            })
            put("secret",action)
            put("exceptions",prior?.get("exceptions") ?: JsonArray(emptyList()))
        }
    }
    fun key(value:String,clear:Boolean=false)=buildJsonObject {
        when { value.isNotBlank() -> { put("action","set");put("value",value.trim()) }; clear -> put("action","clear"); else -> put("action","keep") }
    }
    val providers=buildList {
        if(choices.definitions) add(entry(ServicePresets.definitions))
        if(choices.weather) { add(entry(ServicePresets.forecast));add(entry(ServicePresets.places)) }
        if(choices.libre) {
            val endpoint=choices.libreEndpoint.trim().trimEnd('/')
            require(endpoint.startsWith("https://") && '?' !in endpoint && endpoint.length<=1000) { "Enter the LibreTranslate address as https://… ending in /translate, without a query." }
            add(entry(ServicePresets.libre,endpoint,key(choices.libreKey,choices.libreClear)))
        }
        if(choices.google) {
            require(choices.googleKey.isNotBlank() || current.serviceSecret(ServicePresets.google.id)) { "Enter a Google Cloud Translation API key." }
            add(entry(ServicePresets.google,secret=key(choices.googleKey)))
        }
        otherServiceProviders(current).forEach { e-> add(buildJsonObject {
            put("provider",e.getValue("provider"));put("secret",buildJsonObject { put("action","keep") });put("exceptions",e.getValue("exceptions"))
        }) }
    }
    require(providers.size<=8) { "A server can offer at most 8 providers." }
    return buildJsonObject {
        put("expected_revision",current.jsonObject.getValue("revision"))
        put("per_account_daily",perAccount);put("total_daily",total)
        put("providers",JsonArray(providers))
    }
}

@Composable
@OptIn(ExperimentalLayoutApi::class)
internal fun ServiceSettings(read:suspend ()->JsonElement, save:suspend (JsonObject)->JsonElement,
    field:@Composable (String,String,(String)->Unit,Boolean,Boolean)->Unit) {
    var current by remember { mutableStateOf<JsonElement?>(null) }
    var choices by remember { mutableStateOf(ServiceChoices()) }
    var dirty by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    var saved by remember { mutableStateOf(false) }
    val scope=rememberCoroutineScope()
    fun load(value:JsonElement) { current=value;choices=serviceChoices(value);dirty=false }
    suspend fun operation(action:suspend ()->Unit) {
        if(busy)return
        busy=true;error="";saved=false
        try { action() }
        catch(cancelled:CancellationException) { throw cancelled }
        catch(failure:Exception) { error=failure.message ?: "Reference services could not be saved. Reload and try again." }
        finally { busy=false }
    }
    fun refresh() { scope.launch(start=CoroutineStart.UNDISPATCHED) { operation { load(read()) } } }
    fun edit(next:ServiceChoices) { choices=next;dirty=true;saved=false;error="" }
    LaunchedEffect(Unit) { operation { load(read()) } }
    val config=current
    val editing=!busy && config!=null
    val any=choices.definitions || choices.weather || choices.libre || choices.google
    val configured=config?.jsonObject?.get("providers")?.jsonArray?.isNotEmpty()==true
    fun submit() {
        val base=config ?: return
        if(!editing || !dirty)return
        scope.launch(start=CoroutineStart.UNDISPATCHED) { operation {
            val request=serviceConfigurationRequest(base,choices)
            load(save(request));saved=true
        } }
    }
    Column(Modifier.fillMaxWidth(),horizontalAlignment=Alignment.CenterHorizontally) {
      Column(Modifier.widthIn(max=680.dp).fillMaxWidth().padding(horizontal=16.dp),verticalArrangement=Arrangement.spacedBy(16.dp)) {
        SettingsNote("Definition, weather and translation cards look things up through this server. The query goes from this server to the provider; the finished card is encrypted when shared.")
        if(busy) LinearProgressIndicator(Modifier.fillMaxWidth())
        if(config==null) {
            if(error.isNotEmpty()) Text(error,Modifier.padding(horizontal=12.dp),style=MaterialTheme.typography.bodyMedium,color=MaterialTheme.colorScheme.error)
            if(!busy) SigilButton(::refresh) { Text("Retry reference services") }
        } else {
            SettingsToggle("Definitions · Wiktionary","Free, no key. Definitions are CC BY-SA and credited on each card.",choices.definitions,editing,"Enable Wiktionary definitions") {
                edit(choices.copy(definitions=it))
            }
            SettingsToggle("Weather · Open-Meteo","Free for non-commercial use, no key. Includes place search.",choices.weather,editing,"Enable Open-Meteo weather") {
                edit(choices.copy(weather=it))
            }
            SettingsToggle("Translation · LibreTranslate","Your own instance, or a libretranslate.com API key.",choices.libre,editing,"Enable LibreTranslate") {
                edit(choices.copy(libre=it))
            }
            Expandable(choices.libre) {
                Column(verticalArrangement=Arrangement.spacedBy(12.dp)) {
                    field("LibreTranslate address · https://…/translate",choices.libreEndpoint,{edit(choices.copy(libreEndpoint=it))},false,editing)
                    val stored=config.serviceSecret(ServicePresets.libre.id)
                    field(if(stored) "Saved API key · type to replace" else "API key · optional",choices.libreKey,{if(it.length<=4096) edit(choices.copy(libreKey=it,libreClear=false))},true,editing)
                    if(stored) SigilTextButton(onClick={edit(choices.copy(libreKey="",libreClear=!choices.libreClear))},enabled=editing) {
                        Text(if(choices.libreClear) "Keep saved key" else "Remove saved key")
                    }
                    SettingsNote("A self-hosted instance needs no key unless you set one. The public libretranslate.com service needs a paid key. Changing the address removes a saved key unless you enter it again.")
                }
            }
            SettingsToggle("Translation · Google Cloud","Needs a Cloud Translation API key from a billing-enabled project.",choices.google,editing,"Enable Google Cloud Translation") {
                edit(choices.copy(google=it))
            }
            Expandable(choices.google) {
                Column(verticalArrangement=Arrangement.spacedBy(12.dp)) {
                    val stored=config.serviceSecret(ServicePresets.google.id)
                    field(if(stored) "Saved API key · type to replace" else "Cloud Translation API key",choices.googleKey,{if(it.length<=4096) edit(choices.copy(googleKey=it))},true,editing)
                    SettingsNote("Keys are sent only to this server and are never shown again. Restrict the key to the Cloud Translation API. Google bills translated characters beyond the monthly free allowance.")
                }
            }
            otherServiceProviders(config).takeIf { it.isNotEmpty() }?.let { others->
                SettingsNote("Also offered, unchanged here: "+others.joinToString { it["provider"]?.jsonObject?.get("id")?.jsonPrimitive?.content.orEmpty() }+".")
            }
            Expandable(any || otherServiceProviders(config).isNotEmpty()) {
                Column(verticalArrangement=Arrangement.spacedBy(12.dp)) {
                    field("Lookups per person per day",choices.perAccount,{edit(choices.copy(perAccount=it.filter(Char::isDigit).take(8)))},false,editing)
                    field("Lookups per day for the whole server",choices.total,{edit(choices.copy(total=it.filter(Char::isDigit).take(8)))},false,editing)
                    SettingsNote("Each lookup counts once against both limits, which reset daily. They cap what one person can spend and keep the server inside provider allowances such as Open-Meteo's 10,000 free calls a day.")
                }
            }
            if(error.isNotEmpty()) Text(error,Modifier.padding(horizontal=12.dp),style=MaterialTheme.typography.bodyMedium,color=MaterialTheme.colorScheme.error)
            FlowRow(horizontalArrangement=Arrangement.spacedBy(12.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                SigilButton(onClick=::submit,enabled=editing && dirty) { Text("Save reference services") }
                SigilTextButton(onClick=::refresh,enabled=!busy) { Text("Reload") }
            }
            if(saved) SettingsNote(if(configured) "Saved. Apps pick up these providers the next time someone opens a reference tool." else "Saved. Reference tools are off for everyone.")
        }
      }
    }
}
