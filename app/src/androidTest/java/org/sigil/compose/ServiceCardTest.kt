package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.runtime.*
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*

class ServiceCardTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    @Test fun offline_snapshots_keep_originals_definitions_dates_and_local_weather_units() {
        val t={s:String->RichText(s)}
        var service by mutableStateOf(ServiceContent("translation",t("Hola"),t("Synthetic provider"),"Jan 15, 2027 · 8:00 AM UTC",null,"en → es","Hola",t("Hello"),null,null,emptyList(),null,emptyList(),emptyList(),"",false))
        val chat=ChatSummary("self","@sam:example.test","","",true,emptyList())
        val commands=mutableListOf<String>()
        ui.runOnUiThread { ui.activity.setSigilContent {
            val message=ChatMessage("service","sam","Snapshot",true,"9:33","sent",false,emptyList(),emptyList(),null,true,timestamp=1000,parts=listOf(MessagePart("card","service","Snapshot",service=service)))
            SigilApp(NativeCore::palette,NativeCore::analyze,MessengerState(phase="connected",chats=listOf(chat),selected="self",messages=listOf(message)),{name,_->commands+=name})
        } }
        ui.onNodeWithText("Hello").assertDoesNotExist()
        ui.onNodeWithText("Show original").performClick()
        ui.onNodeWithText("Hello").assertIsDisplayed()
        ui.onNodeWithText("Open translation").performClick()
        ui.onNodeWithContentDescription("Copy translation").performClick()
        val clipboard=ui.activity.getSystemService(android.content.ClipboardManager::class.java)
        ui.runOnIdle { assertEquals("Hola",clipboard.primaryClip!!.getItemAt(0).text.toString()) }
        ui.onNodeWithContentDescription("Close translation").performClick()
        ui.runOnIdle { service=service.copy(title=RichText("Hola",listOf(RichSpan(0,4,reveal="spoiler"))),copy=null) }
        ui.onNodeWithText("Hola").assertDoesNotExist()
        ui.onNodeWithText("Open translation").performClick()
        ui.onNodeWithContentDescription("Copy translation").assertIsNotEnabled()
        ui.onNodeWithContentDescription("Close translation").performClick()
        val first=DefinitionSense(t("noun"),t("A written message"),t("A letter arrived."),null,listOf(t("correspondence")),emptyList(),"A written message")
        ui.runOnIdle { service=service.copy(kind="definition",title=t("letter"),language="en",copy=null,original=null,senses=listOf(first,first.copy(definition=t("An alphabet character"),copy="An alphabet character"))) }
        ui.onNodeWithText("An alphabet character").assertDoesNotExist()
        ui.onNodeWithText("More definitions").performClick()
        ui.onNode(hasText("An alphabet character") and hasAnyAncestor(isDialog())).performScrollTo().assertIsDisplayed()
        ui.onNodeWithContentDescription("Close definition").performClick()
        val current=WeatherConditions("Fri, Jan 15 · 2:00 AM CST","2027-01-15",listOf("0.0 °C","32.0 °F"),null,t("Snow"),"weather_snowy","1.5 mm","80%",listOf("3.6 km/h N","2.2 mph N"),"50%","1.5")
        val day=WeatherDay("Fri, Jan 15","2027-01-15",listOf("-5.0 °C","23.0 °F"),listOf("5.0 °C","41.0 °F"),"weather_snowy","80%",t("Snow"))
        ui.runOnIdle { service=service.copy(kind="weather",title=t("Synthetic place"),language="",senses=emptyList(),current=current,days=listOf(day),hours=listOf(current,current.copy(date="Fri, Jan 15 · 3:00 AM CST",temperature=listOf("1.0 °C","33.8 °F"))),today=day.key,historical=true) }
        ui.onNodeWithText("Historical weather snapshot").assertIsDisplayed()
        ui.onNodeWithText("Use °F and mph").performClick()
        ui.onNodeWithText("32.0 °F").assertIsDisplayed()
        ui.onNodeWithText("Open weather").performClick()
        ui.onNode(hasText("Next hour") and hasAnyAncestor(isDialog())).performScrollTo().performClick()
        ui.onNode(hasText("33.8 °F") and hasAnyAncestor(isDialog())).performScrollTo().assertIsDisplayed()
        ui.onNodeWithContentDescription("Close weather").performClick()
        ui.runOnIdle { assertFalse(commands.any { it in listOf("post","service_resolve","location_start","card_action") }) }
    }
}
