@file:OptIn(androidx.compose.ui.test.ExperimentalTestApi::class)
package org.sigil

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test

private fun rich(value:String)=RichText(value)
private fun service(kind:String,title:String,attribution:String="Example reference provider",source:String?=null,language:String="",
    original:RichText?=null,pronunciation:RichText?=null,senses:List<DefinitionSense> = emptyList(),
    current:WeatherConditions?=null,days:List<WeatherDay> = emptyList(),today:String="")=
    ServiceContent(kind,rich(title),rich(attribution),"Sep 15, 2025 · 3:00 PM UTC",source,language,null,original,pronunciation,null,senses,current,days,emptyList(),today,false)

class ServiceCardTest {
    @get:Rule val ui=createComposeRule()

    @Test fun translation_keeps_the_original_inline_and_drops_the_snapshot_time() {
        val value=service("translation","Where is the library?",source="https://example.invalid/entry",language="es (detected) → en",
            original=rich("¿Dónde está la biblioteca?"))
        ui.setContent {MaterialTheme {Box(Modifier.size(360.dp,640.dp)) {ServiceCard(value)}}}
        ui.onNodeWithText("Where is the library?").assertIsDisplayed()
        ui.onNodeWithText("¿Dónde está la biblioteca?").assertIsDisplayed()
        ui.onNodeWithText("Show original").assertDoesNotExist()
        ui.onNodeWithText("Open translation").assertDoesNotExist()
        ui.onAllNodesWithText("Snapshot",substring=true).assertCountEquals(0)
        ui.onNodeWithText("Example reference provider").assertIsDisplayed()
        ui.onNodeWithText("Open source").assertIsDisplayed()
    }

    @Test fun definition_numbers_every_sense_and_keeps_the_source_line() {
        val value=service("definition","petrichor",language="en",pronunciation=rich("ˈpɛtrɪkɔː"),senses=listOf(
            DefinitionSense(rich("noun"),rich("The smell of rain on dry ground."),rich("A study of petrichor."),null,emptyList(),emptyList(),null),
            DefinitionSense(rich("noun"),rich("The oil released by that rain."),null,null,emptyList(),emptyList(),null),
            DefinitionSense(rich("verb"),rich("To carry that scent."),null,null,emptyList(),emptyList(),null)))
        ui.setContent {MaterialTheme {Box(Modifier.size(360.dp,640.dp)) {ServiceCard(value)}}}
        ui.onNodeWithText("petrichor").assertIsDisplayed()
        ui.onNodeWithText("ˈpɛtrɪkɔː").assertIsDisplayed()
        ui.onNodeWithText("1.").assertIsDisplayed()
        ui.onNodeWithText("2.").assertIsDisplayed()
        ui.onNodeWithText("3.").assertIsDisplayed()
        ui.onNodeWithText("The oil released by that rain.").assertIsDisplayed()
        ui.onNodeWithText("A study of petrichor.").assertIsDisplayed()
        ui.onNodeWithText("verb").assertIsDisplayed()
        ui.onNodeWithText("More definitions").assertDoesNotExist()
        ui.onAllNodesWithText("definitions",substring=true).assertCountEquals(0)
        ui.onNodeWithText("Example reference provider").assertIsDisplayed()
    }

    @Test fun weather_shows_metrics_and_the_forecast_inline_without_a_reading_time() {
        val current=WeatherConditions("Mon, Sep 15 · 3:00 PM UTC","2025-09-15",listOf("21.0 °C","69.8 °F"),listOf("20.0 °C","68.0 °F"),
            rich("Partly cloudy"),"partly_cloudy_day","1.2 mm","20%",listOf("8.0 km/h N","5.0 mph N"),"55%","3.1")
        val days=listOf(
            WeatherDay("Mon, Sep 15","2025-09-15",listOf("13.0 °C","55.4 °F"),listOf("22.0 °C","71.6 °F"),"sunny","20%",rich("Sunny")),
            WeatherDay("Tue, Sep 16","2025-09-16",listOf("14.0 °C","57.2 °F"),listOf("24.0 °C","75.2 °F"),"rainy","60%",rich("Rain")))
        ui.setContent {MaterialTheme {Box(Modifier.size(360.dp,640.dp)) {ServiceCard(service("weather","Riverbend",current=current,days=days,today="2025-09-15"))}}}
        ui.onNodeWithText("Riverbend").assertIsDisplayed()
        ui.onNodeWithText("21.0 °C").assertIsDisplayed()
        ui.onNodeWithText("Mon, Sep 15 · 3:00 PM UTC").assertDoesNotExist()
        ui.onAllNodesWithText("Snapshot",substring=true).assertCountEquals(0)
        ui.onNodeWithText("Rain chance").assertIsDisplayed()
        ui.onNodeWithText("Feels like").assertIsDisplayed()
        ui.onNodeWithText("Tue").assertIsDisplayed()
        ui.onNodeWithText("Use °F and mph").performClick()
        ui.onNodeWithText("69.8 °F").assertIsDisplayed()
        ui.onNodeWithText("75.2 °F").assertIsDisplayed()
    }
}
