@file:OptIn(androidx.compose.ui.test.ExperimentalTestApi::class)
package org.sigil

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.size
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private fun rich(value:String)=RichText(value)
private fun service(kind:String,title:String,attribution:String="Example reference provider",source:String?=null,language:String="",
    original:RichText?=null,pronunciation:RichText?=null,senses:List<DefinitionSense> = emptyList(),
    current:WeatherConditions?=null,days:List<WeatherDay> = emptyList(),hours:List<WeatherConditions> = emptyList(),today:String="")=
    ServiceContent(kind,rich(title),rich(attribution),"Sep 15, 2025 · 3:00 PM UTC",source,language,null,original,pronunciation,null,senses,current,days,hours,today,false)

class ServiceCardTest {
    @get:Rule val ui=createComposeRule()

    @Test fun readings_are_formatted_for_people() {
        assertEquals("21°",degrees("21.0 °C")); assertEquals("0°",degrees("-0.4 °C")); assertEquals("\u22124°",degrees("-3.6 °F"))
        assertEquals("8 km/h N",wholeReading("8.0 km/h N")); assertEquals("3 PM",weatherHour("Mon, Sep 15 · 3:00 PM UTC")); assertEquals("12 AM",weatherHour("Tue, Sep 16 · 12:00 AM UTC"))
        val languages=translationLanguages("es (detected) → pt-BR")!!
        assertEquals("Spanish",languages.source); assertEquals("Portuguese (BR)",languages.target); assertTrue(languages.detected)
        assertEquals("XX",languageName("xx"))
    }

    @Test fun translation_labels_both_languages_and_keeps_the_source_link() {
        val value=service("translation","Where is the library?",source="https://example.invalid/entry",language="es (detected) → en",
            original=rich("¿Dónde está la biblioteca?"))
        ui.setContent {MaterialTheme {Box(Modifier.size(360.dp,640.dp)) {ServiceCard(value)}}}
        ui.onNodeWithText("Where is the library?").assertIsDisplayed()
        ui.onNodeWithText("¿Dónde está la biblioteca?").assertIsDisplayed()
        ui.onNodeWithText("Translation into English").assertIsDisplayed()
        ui.onNodeWithText("Original, Spanish, detected").assertIsDisplayed()
        ui.onAllNodesWithText("Snapshot",substring=true).assertCountEquals(0)
        ui.onNode(hasText("Example reference provider") and hasClickAction()).assertIsDisplayed()
    }

    @Test fun definition_groups_senses_by_part_of_speech_and_folds_long_entries() {
        val value=service("definition","petrichor",language="en",pronunciation=rich("ˈpɛtrɪkɔː"),senses=listOf(
            DefinitionSense(rich("noun"),rich("The smell of rain on dry ground."),rich("A study of petrichor."),null,emptyList(),emptyList(),null),
            DefinitionSense(rich("noun"),rich("The oil released by that rain."),null,null,emptyList(),emptyList(),null))+
            (3..7).map {DefinitionSense(rich("verb"),rich("Sense $it."),null,null,emptyList(),emptyList(),null)})
        ui.setContent {MaterialTheme {Box(Modifier.size(360.dp,900.dp)) {ServiceCard(value)}}}
        ui.onNode(hasContentDescription("Definition. petrichor")).assertExists()
        ui.onNodeWithText("ˈpɛtrɪkɔː",substring=true).assertIsDisplayed()
        ui.onNodeWithText("1.").assertIsDisplayed()
        ui.onNodeWithText("A study of petrichor.").assertIsDisplayed()
        ui.onAllNodesWithText("noun").assertCountEquals(1)
        ui.onAllNodesWithText("verb").assertCountEquals(1)
        ui.onNodeWithText("Sense 5.").assertIsDisplayed()
        ui.onNodeWithText("Sense 6.").assertDoesNotExist()
        ui.onNodeWithText("Show all 7").performClick()
        ui.onNodeWithText("Sense 7.").assertIsDisplayed()
        ui.onNodeWithText("Example reference provider").assertIsDisplayed()
    }

    @Test fun a_definition_without_senses_says_so() {
        ui.setContent {MaterialTheme {Box(Modifier.size(360.dp,640.dp)) {ServiceCard(service("definition","qwzx",language="en"))}}}
        ui.onNodeWithText("No definition found.").assertIsDisplayed()
    }

    @Test fun weather_leads_with_now_then_hours_and_days_and_switches_units() {
        fun conditions(time:String,c:String,f:String)=WeatherConditions("Mon, Sep 15 · $time UTC","2025-09-15",listOf(c,f),listOf("20.0 °C","68.0 °F"),
            rich("Partly cloudy"),"partly_cloudy_day","1.2 mm","20%",listOf("8.0 km/h N","5.0 mph N"),"55%","3.1")
        val current=conditions("3:10 PM","21.0 °C","69.8 °F")
        val hours=listOf(conditions("3:00 PM","21.0 °C","69.8 °F"),conditions("4:00 PM","22.0 °C","71.6 °F"),conditions("5:00 PM","19.6 °C","67.3 °F"))
        val days=listOf(
            WeatherDay("Mon, Sep 15","2025-09-15",listOf("13.0 °C","55.4 °F"),listOf("22.0 °C","71.6 °F"),"sunny","20%",rich("Sunny")),
            WeatherDay("Tue, Sep 16","2025-09-16",listOf("14.0 °C","57.2 °F"),listOf("24.0 °C","75.2 °F"),"rainy","60%",rich("Rain")))
        WeatherUnits.imperial=false
        ui.setContent {MaterialTheme {Box(Modifier.size(360.dp,900.dp)) {ServiceCard(service("weather","Riverbend",current=current,days=days,hours=hours,today="2025-09-15"))}}}
        ui.onNode(hasContentDescription("Weather. Riverbend")).assertExists()
        ui.onNodeWithContentDescription("21.0 °C").assertIsDisplayed()
        ui.onNodeWithText("As of 3:10 PM UTC").assertIsDisplayed()
        ui.onNodeWithText("Rain").assertIsDisplayed()
        ui.onNodeWithText("Wind N").assertIsDisplayed()
        ui.onNodeWithText("8 km/h").assertIsDisplayed()
        ui.onNodeWithText("Now",useUnmergedTree=true).assertIsDisplayed()
        ui.onNodeWithContentDescription("Next hours: 4 PM 22.0 °C; 5 PM 19.6 °C").assertExists()
        ui.onNodeWithContentDescription("Today",substring=true).assertExists()
        ui.onNodeWithContentDescription("Tue: Rain, high 24.0 °C",substring=true).assertExists()
        ui.onNodeWithContentDescription("Show Fahrenheit and miles").performClick()
        ui.onNodeWithContentDescription("69.8 °F").assertIsDisplayed()
        ui.onNodeWithContentDescription("Tue: Rain, high 75.2 °F",substring=true).assertExists()
    }

    private fun reading(time:String,c:String,f:String,at:Long)=WeatherConditions("Mon, Sep 15 · $time UTC","2025-09-15",listOf(c,f),null,
        rich("Clear"),"sunny",null,"10%",listOf("8.0 km/h N","5.0 mph N"),"55%","3.1",at)
    private val stamped=service("weather","Riverbend",current=reading("3:00 PM","18.0 °C","64.4 °F",1_757_948_400),
        hours=listOf(reading("4:00 PM","19.0 °C","66.2 °F",1_757_952_000),reading("5:00 PM","17.0 °C","62.6 °F",1_757_955_600)),
        days=listOf(WeatherDay("Mon, Sep 15","2025-09-15",listOf("13.0 °C","55.4 °F"),listOf("22.0 °C","71.6 °F"),"sunny","20%",rich("Sunny")),
            WeatherDay("Tue, Sep 16","2025-09-16",listOf("14.0 °C","57.2 °F"),listOf("24.0 °C","75.2 °F"),"rainy","60%",rich("Rain"))),today="2025-09-15")

    @Test fun an_old_weather_snapshot_shows_its_date_and_never_says_now_or_today() {
        WeatherUnits.imperial=false
        ui.setContent {MaterialTheme {Box(Modifier.size(360.dp,900.dp)) {ServiceCard(stamped,nowSeconds=1_757_948_400+3*86_400)}}}
        ui.onNodeWithText("As of Mon, Sep 15 · 3:00 PM UTC").assertIsDisplayed()
        ui.onNodeWithText("Now",useUnmergedTree=true).assertDoesNotExist()
        ui.onNodeWithContentDescription("Today",substring=true).assertDoesNotExist()
        ui.onNodeWithContentDescription("Mon: Sunny",substring=true).assertExists()
        ui.onNodeWithText("4 PM",useUnmergedTree=true).assertIsDisplayed()
    }

    @Test fun a_fresh_weather_snapshot_leads_with_now_and_today() {
        val time=weatherTime(stamped,1_757_948_400+20*60)
        assertEquals("As of 3:00 PM UTC",time.label); assertTrue(time.now); assertTrue(time.today)
        val later=weatherTime(stamped,1_757_948_400+2*3600)
        assertEquals(false,later.now); assertTrue(later.today)
        assertEquals(false,weatherTime(stamped,1_757_948_400+10*3600).today)
    }

    @Test fun one_unit_choice_drives_every_weather_card_and_the_reply_quote() {
        WeatherUnits.imperial=false
        ui.setContent {MaterialTheme {Column {ServiceCard(stamped,nowSeconds=1_757_948_400);ServiceCard(stamped.copy(title=rich("Lakeside")),nowSeconds=1_757_948_400)}}}
        ui.onAllNodesWithContentDescription("18.0 °C").assertCountEquals(2)
        assertEquals("Weather · Riverbend · 18°",cardQuote(listOf(MessagePart("w","service","",service=stamped)))?.line)
        ui.onAllNodesWithContentDescription("Show Fahrenheit and miles")[0].performClick()
        ui.onAllNodesWithContentDescription("64.4 °F").assertCountEquals(2)
        assertEquals("Weather · Riverbend · 64°",cardQuote(listOf(MessagePart("w","service","",service=stamped)))?.line)
        WeatherUnits.imperial=false
    }

    @Test fun copy_yields_the_translation_not_the_lookup_syntax() {
        val value=service("translation","¿Dónde está la biblioteca?",language="en → es",original=rich("Where is the library?")).copy(copy="¿Dónde está la biblioteca?")
        val message=ChatMessage("t","author","translate::es::Where is the library?;",false,"","read",false,emptyList(),emptyList(),null,true,parts=listOf(MessagePart("s","service","",service=value)))
        assertEquals("¿Dónde está la biblioteca?",serviceMessageCopy(message))
        assertEquals(null,serviceMessageCopy(message.copy(parts=listOf(MessagePart("s","service","",service=value.copy(copy=null))))))
    }

    @Test fun language_search_finds_names_and_codes() {
        assertEquals("Spanish",languageMatches("spa").first().second)
        assertEquals("ja",languageMatches("ja").first {it.second=="Japanese"}.first)
        assertTrue(languageMatches("Chinese").any {it.first=="zh-TW"})
        assertTrue(sameLanguage("en-GB","en")); assertTrue(sameLanguage("iw","he"))
    }
}
