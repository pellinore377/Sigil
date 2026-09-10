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
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.AnnotatedString
import kotlinx.coroutines.CompletableDeferred
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

class PushSettingsTest {
    @get:Rule val ui=createComposeRule()
    private val initial=PushConfiguration("12",true,"mailto:admin@example.org","example-project","sender@example-project.iam.gserviceaccount.com","public-key")
    private fun open(read:suspend ()->PushConfiguration={initial},save:suspend (PushUpdate)->PushConfiguration) {
        ui.setContent { MaterialTheme { Column(Modifier.verticalScroll(rememberScrollState())) {
            PushSettings(read,save) {label,value,change,secret,enabled->
                OutlinedTextField(value,change,enabled=enabled,label={Text(label)},
                    modifier=Modifier.semantics {contentDescription=label},
                    visualTransformation=if(secret)PasswordVisualTransformation() else VisualTransformation.None)
            }
        } } }
        ui.waitForIdle()
    }
    private fun click(label:String) { ui.onNodeWithText(label).performScrollTo().performClick() }

    @Test fun independent_loading_retries_then_preserves_google_credentials_for_unified_changes() {
        var reads=0
        val updates=mutableListOf<PushUpdate>()
        open(read={if(++reads==1)error("Temporarily unavailable") else initial},save={
            updates+=it;initial.copy(revision="13",contact=it.contact)
        })
        ui.onNodeWithText("Temporarily unavailable").assertExists()
        click("Retry notification settings")
        ui.onNodeWithContentDescription("Contact · mailto:admin@example.org or HTTPS URL")
            .performScrollTo().performTextReplacement("mailto:replacement@example.org")
        click("Save notification settings")
        ui.onNodeWithText("Saved. Device registration and delivery still need to complete.").assertExists()
        assertEquals(listOf(PushUpdate("12",true,"mailto:replacement@example.org",false,null,false)),updates)
        ui.onNodeWithText("Save notification settings").assertIsNotEnabled()
    }

    @Test fun credential_replacement_is_single_flight_and_clears_the_form_after_success() {
        val waiting=CompletableDeferred<PushConfiguration>()
        val updates=mutableListOf<PushUpdate>()
        open(save={updates+=it;waiting.await()})
        click("Replace credentials")
        ui.onNodeWithContentDescription("Firebase service-account JSON").performScrollTo().performTextInput("synthetic credential fixture")
        click("Save notification settings")
        ui.onNodeWithText("Save notification settings").assertIsNotEnabled()
        ui.onNodeWithText("Reload").assertIsNotEnabled()
        assertEquals(1,updates.size)
        assertEquals("synthetic credential fixture",updates.single().credentials)
        waiting.complete(initial.copy(revision="13"))
        ui.waitForIdle()
        ui.onNodeWithContentDescription("Firebase service-account JSON").assertDoesNotExist()
        click("Replace credentials")
        ui.onNodeWithContentDescription("Firebase service-account JSON").assert(SemanticsMatcher.expectValue(SemanticsProperties.EditableText,AnnotatedString("")))
        ui.onNodeWithText("Save notification settings").assertIsNotEnabled()
    }

    @Test fun disable_requires_confirmation_and_conflict_never_advances_revision() {
        val updates=mutableListOf<PushUpdate>()
        open(save={updates+=it;error("Settings changed. Reload before saving.")})
        ui.onNodeWithContentDescription("Enable Google notifications").performScrollTo().performClick()
        click("Save notification settings")
        ui.onNodeWithText("Change notification delivery?").assertExists()
        click("Cancel")
        assertTrue(updates.isEmpty())
        click("Save notification settings")
        click("Save changes")
        ui.onNodeWithText("Settings changed. Reload before saving.").assertExists()
        assertEquals(PushUpdate("12",true,initial.contact,true,null,false),updates.single())
        click("Reload")
        ui.onNodeWithContentDescription("Enable Google notifications").assertIsOn()
        ui.onNodeWithText("Save notification settings").assertIsNotEnabled()
    }
}
