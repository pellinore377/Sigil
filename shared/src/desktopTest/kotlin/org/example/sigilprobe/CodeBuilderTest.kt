package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.unit.dp
import org.junit.Test
import kotlin.test.*

class CodeBuilderTest {
    @OptIn(ExperimentalTestApi::class)
    @Test fun code_preserves_literal_draft_and_restores_preview_before_explicit_send() = runComposeUiTest {
        val ui=this
        val sent=mutableListOf<String>()
        var registry by mutableStateOf(SaveableStateRegistry(null){true})
        var visible by mutableStateOf(true)
        val body="\tlet x = \"👩🏽‍💻\";\n```\nredact::literal;\n"
        ui.setContent {if(visible)MaterialTheme {CompositionLocalProvider(LocalSaveableStateRegistry provides registry,LocalBuilderSource provides NativeCore::builderSource,LocalCodePreview provides NativeCore::codePreview) {
            Box(Modifier.width(380.dp).height(600.dp)) {CodeBuilder(true,{},sent::add)}
        }}}
        ui.onNodeWithText("Preview code").assertIsNotEnabled()
        ui.onNodeWithText("Plain text").performClick()
        ui.onNodeWithText("Rust").performClick()
        ui.onNodeWithText("Code",substring=false).performTextInput(body.replace("\n","\r\n"))
        ui.onNodeWithText("Code",substring=false).assertTextContains(body)
        ui.onNodeWithText("Code",substring=false).performTextReplacement("x".repeat(17000))
        ui.onNodeWithText("Code",substring=false).assertTextContains(body)
        ui.onNodeWithText("Preview code").performClick()
        ui.onNodeWithText(body,substring=false).assertExists()
        ui.onNodeWithText("Open code · 3 lines").performScrollTo().performClick()
        ui.onNodeWithContentDescription("Close code").performClick()
        var saved:Map<String,List<Any?>> = emptyMap()
        ui.runOnIdle {saved=registry.performSave();visible=false}
        ui.waitForIdle()
        ui.runOnIdle {registry=SaveableStateRegistry(saved){true};visible=true}
        ui.onNodeWithText(body,substring=false).assertExists()
        assertTrue(sent.isEmpty())
        ui.onNodeWithText("Send code").performClick()
        assertEquals(listOf(NativeCore.builderSource("Code\nrust\n$body")),sent)
        ui.onNodeWithContentDescription("Edit code").performClick()
        ui.onNodeWithText("Code",substring=false).assertTextContains(body)
    }

    @Test fun preview_uses_canonical_unicode_offsets_and_rejects_invalid_ranges() {
        val value=codePreview(NativeCore.codePreview("Code\nrust\nlet x = \"👩🏽‍💻\";"))!!
        assertEquals("rust",value.first)
        val token=value.second.codeTokens.first {it.role=="string"}
        assertEquals("\"👩🏽‍💻\"",value.second.text.substring(token.start,token.end))
        for(input in listOf("","rust\n0,99,string\nx","rust\nx,2,string\nx","rust\n2,1,string\nxx"))assertNull(codePreview(input))
    }
}
