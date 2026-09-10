package org.sigil

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import kotlinx.coroutines.CompletableDeferred
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class AndroidPushSettingsTest {
    @get:Rule val ui=createComposeRule()
    @Test fun failed_saves_retain_input_and_success_clears_it_without_duplicate_submissions() {
        val saved=mutableListOf<String?>()
        val waiting=CompletableDeferred<AndroidPushConfiguration>()
        ui.setContent {MaterialTheme {Column(Modifier.verticalScroll(rememberScrollState())) {
            AndroidPushSettings("synthetic-project",read={AndroidPushConfiguration("")},save={
                saved+=it
                if(saved.size==1)error("Settings changed. Reload.")
                waiting.await()
            }) {label,value,change,_,enabled->OutlinedTextField(value,change,enabled=enabled,label={Text(label)},modifier=Modifier.semantics {contentDescription=label})}
        }}}
        ui.onNodeWithContentDescription("Android google-services.json").performScrollTo().performTextInput("synthetic public configuration")
        ui.onNodeWithText("Save Android configuration").performScrollTo().performClick()
        ui.onNodeWithText("Settings changed. Reload.").assertExists()
        ui.onNodeWithContentDescription("Android google-services.json").assertTextContains("synthetic public configuration")
        ui.onNodeWithText("Save Android configuration").performScrollTo().performClick()
        ui.onNodeWithText("Save Android configuration").assertIsNotEnabled()
        ui.onNodeWithText("Reload Android configuration").assertIsNotEnabled()
        assertEquals(2,saved.size)
        waiting.complete(AndroidPushConfiguration("1:123456789:android:0123456789abcdef"))
        ui.waitForIdle()
        ui.onNodeWithContentDescription("Android google-services.json").assertTextContains("")
        ui.onNodeWithText("Save Android configuration").assertIsNotEnabled()
        ui.onNodeWithText("Configured · 1:123456789:android:0123456789abcdef").assertExists()
    }
}
