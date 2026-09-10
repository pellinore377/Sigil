package org.sigil

import kotlinx.serialization.json.*
import kotlin.test.*

class AdminPushTest {
    @Test fun android_download_is_project_and_package_bound_and_strips_unrelated_fields() {
        val source=buildJsonObject {
            putJsonObject("project_info") {put("project_id","synthetic-project");put("project_number","123456789");put("storage_bucket","ignored")}
            putJsonArray("client") {add(buildJsonObject {
                putJsonObject("client_info") {put("mobilesdk_app_id","1:123456789:android:0123456789abcdef");putJsonObject("android_client_info") {put("package_name","org.sigil.compose")}}
                putJsonArray("api_key") {add(buildJsonObject {put("current_key","AIza"+"x".repeat(35))})}
                put("private_key","must not leave browser")
            })}
        }.toString()
        val result=androidFirebaseRequest(source,"synthetic-project")
        assertEquals(setOf("project_id","sender_id","application_id","api_key"),result.keys)
        assertEquals("123456789",result.getValue("sender_id").jsonPrimitive.content)
        assertFailsWith<IllegalArgumentException> {androidFirebaseRequest(source,"another-project")}
        assertFailsWith<IllegalArgumentException> {androidFirebaseRequest(source.replace("org.sigil.compose","org.other.app"),"synthetic-project")}
        assertFailsWith<IllegalArgumentException> {androidFirebaseRequest("{\"private_key\":\"secret\"}","synthetic-project")}
    }
    private val previous=Json.parseToJsonElement("""{"revision":9007199254740993,"exceptions":[{"host":"push.example.org","port":443,"networks":["192.0.2.0/24"],"root_ca":null}]}""").jsonObject
    private val update=PushUpdate("9007199254740993",true,"mailto:admin@example.org",false,null,false)
    @Test fun saves_preserve_exact_revision_and_private_network_policy_without_reading_a_secret() {
        val request=pushConfigurationRequest(previous,update)
        assertEquals(previous.getValue("revision"),request.getValue("expected_revision"))
        assertEquals(previous.getValue("exceptions"),request.getValue("exceptions"))
        assertEquals("""{"action":"keep"}""",request.getValue("fcm").toString())
        assertFailsWith<IllegalStateException> {pushConfigurationRequest(previous,update.copy(revision="9007199254740992"))}
    }
    @Test fun downloaded_key_keeps_pem_newlines_and_never_selects_an_external_token_endpoint() {
        val key="-----BEGIN PRIVATE KEY-----\nsynthetic only\n-----END PRIVATE KEY-----\n"
        val source=buildJsonObject {
            put("type","service_account");put("project_id","synthetic-project")
            put("client_email","sender@synthetic-project.iam.gserviceaccount.com");put("private_key",key)
            put("token_uri","https://attacker.example/token");put("client_id","ignored")
        }
        val request=pushConfigurationRequest(previous,update.copy(credentials=source.toString()))
        val credentials=request.getValue("fcm").jsonObject.getValue("credentials").jsonObject
        assertEquals(setOf("project_id","client_email","private_key"),credentials.keys)
        assertEquals(key,credentials.getValue("private_key").jsonPrimitive.content)
        val disabled=pushConfigurationRequest(previous,update.copy(disableGoogle=true,credentials=source.toString()))
        assertEquals("""{"action":"disable"}""",disabled.getValue("fcm").toString())
    }
    @Test fun malformed_or_wrong_configuration_has_only_a_generic_error() {
        for(source in listOf("secret fixture", "{}", "[]", """{"type":"service_account","private_key":"secret fixture"}""", "x".repeat(32769))) {
            val error=assertFailsWith<IllegalArgumentException> {pushConfigurationRequest(previous,update.copy(credentials=source))}
            assertEquals("Paste a complete Firebase service-account JSON key. Its contents have not been sent.",error.message)
            assertNull(error.cause)
        }
    }
}
