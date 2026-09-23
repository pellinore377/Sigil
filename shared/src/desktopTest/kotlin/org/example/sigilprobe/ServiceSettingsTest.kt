package org.sigil

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import kotlinx.serialization.json.*
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class ServiceSettingsTest {
    @get:Rule val ui=createComposeRule()
    private val empty=Json.parseToJsonElement("""{"revision":0,"per_account_daily":0,"total_daily":0,"providers":[]}""")
    private val custom="""{"provider":{"id":"local-dictionary","kind":"dictionary_index","endpoint":"https://dictionary.example.org/define","attribution":{"body":"Synthetic","spans":[]},"version":null,"source_url":null},"has_secret":true,"exceptions":[{"host":"dictionary.example.org","port":443,"networks":["192.0.2.0/24"],"root_ca":null}]}"""
    private fun configured(google:Boolean)=Json.parseToJsonElement("""{"revision":4,"per_account_daily":50,"total_daily":900,"providers":[
        {"provider":{"id":"libretranslate","kind":"libre_translate","endpoint":"https://translate.example.org/translate","attribution":{"body":"Translated by LibreTranslate","spans":[]},"version":null,"source_url":"https://libretranslate.com/"},"has_secret":true,"exceptions":[]},
        ${if(google) """{"provider":{"id":"google-translate","kind":"google_translate","endpoint":"https://translation.googleapis.com/language/translate/v2","attribution":{"body":"Translated by Google","spans":[]},"version":null,"source_url":"https://cloud.google.com/translate"},"has_secret":true,"exceptions":[]},""" else ""}
        $custom]}""")
    private fun JsonObject.providers()=getValue("providers").jsonArray.map { it.jsonObject }
    private fun JsonObject.entry(id:String)=providers().single { it["provider"]!!.jsonObject["id"]!!.jsonPrimitive.content==id }
    private fun JsonObject.action(id:String)=entry(id)["secret"]!!.jsonObject["action"]!!.jsonPrimitive.content

    @Test fun keyless_presets_enable_definitions_and_both_weather_endpoints_with_attribution() {
        val request=serviceConfigurationRequest(empty,ServiceChoices(definitions=true,weather=true))
        assertEquals(0,request["expected_revision"]!!.jsonPrimitive.int)
        assertEquals(100,request["per_account_daily"]!!.jsonPrimitive.int)
        assertEquals(listOf("wiktionary","open-meteo","open-meteo-places"),request.providers().map { it["provider"]!!.jsonObject["id"]!!.jsonPrimitive.content })
        assertEquals(listOf("wiktionary","open_meteo","geocoder"),request.providers().map { it["provider"]!!.jsonObject["kind"]!!.jsonPrimitive.content })
        request.providers().forEach {
            val provider=it["provider"]!!.jsonObject
            assertTrue(provider["endpoint"]!!.jsonPrimitive.content.startsWith("https://"))
            assertTrue(provider["attribution"]!!.jsonObject["body"]!!.jsonPrimitive.content.isNotBlank())
            assertEquals("clear",it["secret"]!!.jsonObject["action"]!!.jsonPrimitive.content)
        }
    }

    @Test fun keys_are_write_only_and_custom_providers_survive() {
        val current=configured(google=true)
        val choices=serviceChoices(current)
        assertTrue(choices.libre && choices.google && !choices.definitions)
        assertEquals("https://translate.example.org/translate",choices.libreEndpoint)
        assertEquals("50",choices.perAccount)
        val kept=serviceConfigurationRequest(current,choices)
        assertEquals("keep",kept.action("libretranslate"))
        assertEquals("keep",kept.action("google-translate"))
        assertEquals("keep",kept.action("local-dictionary"))
        assertEquals(Json.parseToJsonElement(custom).jsonObject["exceptions"],kept.entry("local-dictionary")["exceptions"])
        val moved=serviceConfigurationRequest(current,choices.copy(libreEndpoint="https://other.example.org/translate",googleKey=" new-key "))
        assertEquals("clear",moved.action("libretranslate"))
        assertEquals(JsonPrimitive("new-key"),moved.entry("google-translate")["secret"]!!.jsonObject["value"])
        assertEquals("clear",serviceConfigurationRequest(current,choices.copy(libreClear=true)).action("libretranslate"))
    }

    @Test fun invalid_choices_explain_the_fix() {
        assertFailsWith<IllegalArgumentException> { serviceConfigurationRequest(empty,ServiceChoices(google=true)) }
        assertFailsWith<IllegalArgumentException> { serviceConfigurationRequest(empty,ServiceChoices(libre=true,libreEndpoint="http://translate.example.org/translate")) }
        assertFailsWith<IllegalArgumentException> { serviceConfigurationRequest(empty,ServiceChoices(definitions=true,perAccount="0")) }
        assertFailsWith<IllegalArgumentException> { serviceConfigurationRequest(empty,ServiceChoices(definitions=true,perAccount="500",total="100")) }
    }

    @Test fun one_tap_presets_save_and_reload() {
        val sent=mutableListOf<JsonObject>()
        ui.setContent { MaterialTheme { Column(Modifier.verticalScroll(rememberScrollState())) {
            ServiceSettings(read={empty},save={request->sent+=request
                buildJsonObject { put("revision",1);put("per_account_daily",request["per_account_daily"]!!);put("total_daily",request["total_daily"]!!)
                    put("providers",JsonArray(request.providers().map { buildJsonObject { put("provider",it["provider"]!!);put("has_secret",false);put("exceptions",it["exceptions"]!!) } })) }
            }) {label,value,change,secret,enabled->
                OutlinedTextField(value,change,enabled=enabled,label={Text(label)},modifier=Modifier.semantics {contentDescription=label},
                    visualTransformation=if(secret)PasswordVisualTransformation() else VisualTransformation.None)
            }
        } } }
        ui.onNodeWithText("Save reference services").assertIsNotEnabled()
        ui.onNodeWithContentDescription("Enable Wiktionary definitions").performScrollTo().performClick()
        ui.onNodeWithContentDescription("Enable Open-Meteo weather").performScrollTo().performClick()
        ui.onNodeWithContentDescription("Lookups per person per day").assertExists()
        ui.onNodeWithText("Save reference services").performScrollTo().performClick()
        ui.waitForIdle()
        assertEquals(3,sent.single().providers().size)
        ui.onNodeWithText("Saved. Apps pick up these providers the next time someone opens a reference tool.").assertExists()
        ui.onNodeWithText("Save reference services").assertIsNotEnabled()
        ui.onNodeWithContentDescription("Enable Wiktionary definitions").assertIsOn()
    }
}
