@file:OptIn(kotlin.time.ExperimentalTime::class)
package org.sigil

// Synthetic resolved snapshots so the design workbench can show service cards without a provider.
private fun rich(text: String) = RichText(text)
private fun reading(celsius: Double) = listOf("${(celsius * 10).toInt() / 10.0} °C", "${((celsius * 1.8 + 32) * 10).toInt() / 10.0} °F")
private fun clock(minutes: Int): String { val h = minutes / 60 % 24; return "${if (h % 12 == 0) 12 else h % 12}:00 ${if (h < 12) "AM" else "PM"}" }
private val week = listOf("Mon, Sep 15", "Tue, Sep 16", "Wed, Sep 17", "Thu, Sep 18", "Fri, Sep 19", "Sat, Sep 20", "Sun, Sep 21")
private val skies = listOf("partly_cloudy_day" to "Clouds giving way", "sunny" to "Clear", "rainy" to "Light rain", "cloud" to "Overcast")

private fun weather(place: String, forecast: Boolean, at: Long): ServiceContent {
    val hours = (0 until 48).map { i ->
        val minutes = (15 + i) * 60
        val day = minutes / 1440
        val t = 16.0 + 5 * kotlin.math.sin((minutes % 1440 - 540) / 1440.0 * 2 * kotlin.math.PI)
        val sky = skies[if (i in 3..6) 2 else if (i > 10) 1 else 0]
        WeatherConditions("${week[day]} · ${clock(minutes)} PDT", "2025-09-${15 + day}", reading(t), null, rich(sky.second), sky.first, null, "${(i * 7) % 40}%",
            listOf("12.0 km/h NW", "7.5 mph NW"), "64%", "3.0")
    }
    val current = WeatherConditions("Mon, Sep 15 · 2:40 PM PDT", "2025-09-15", reading(18.0), reading(17.2), rich("Clouds giving way"), "partly_cloudy_day",
        "0.0 mm", "20%", listOf("12.4 km/h NW", "7.7 mph NW"), "64%", "4.2", at)
    val highs = listOf(21.0, 22.0, 20.0, 18.0, 21.0, 23.0, 19.0); val lows = listOf(13.0, 14.0, 12.0, 12.0, 13.0, 15.0, 11.0)
    val chances = listOf("20%", "10%", "30%", "65%", "20%", "0%", "40%"); val icons = listOf("partly_cloudy_day", "sunny", "partly_cloudy_day", "rainy", "partly_cloudy_day", "sunny", "cloud")
    val days = (0 until if (forecast) 7 else 1).map { WeatherDay(week[it], "2025-09-${15 + it}", reading(lows[it]), reading(highs[it]), icons[it], chances[it], rich(skies.first { s -> s.first == icons[it] }.second)) }
    return ServiceContent("weather", rich(place), rich("Weather data by Open-Meteo.com, CC BY 4.0"), "Sep 15, 2025 · 2:40 PM PDT", "https://open-meteo.com/",
        "", null, null, null, null, emptyList(), current, days, hours, "2025-09-15", false)
}

internal fun previewService(source: String): ServiceContent? {
    val fields = source.trim().removeSuffix(";").split("::")
    return when (fields.firstOrNull()) {
        "translate" -> if (fields.getOrNull(1) == "auto")
            ServiceContent("translation", rich("Where is the library?"), rich("Translated by Google"), "", null, "es (detected) → en", "Where is the library?",
                rich(fields.drop(2).joinToString("::")), null, null, emptyList(), null, emptyList(), emptyList(), "", false)
        else ServiceContent("translation", rich("¿Dónde está la biblioteca?"), rich("Translated by Google"), "", null, "en (detected) → ${fields.getOrNull(1) ?: "es"}",
            "¿Dónde está la biblioteca?", rich(fields.drop(2).joinToString("::")), null, null, emptyList(), null, emptyList(), emptyList(), "", false)
        "define" -> {
            val word = fields.getOrNull(1).orEmpty()
            val senses = if (word != "petrichor") emptyList() else listOf(
                DefinitionSense(rich("noun"), rich("The distinctive, pleasant scent of rain falling on dry ground."), rich("The garden filled with petrichor after the storm."), null, emptyList(), emptyList(), null),
                DefinitionSense(rich("noun"), rich("An oil exuded by plants during dry spells and absorbed by clay and rock, released when rain falls."), null, null, emptyList(), emptyList(), null))
            ServiceContent("definition", rich(word), rich("From Wiktionary, CC BY-SA 4.0"), "", "https://en.wiktionary.org/wiki/$word", "en", null, null,
                if (senses.isEmpty()) null else rich("/ˈpɛt.ɹɪ.kɔːɹ/"), null, senses, null, emptyList(), emptyList(), "", false)
        }
        // Springfield stands in for an old snapshot; the others read as just fetched.
        "weather" -> if (fields.getOrNull(1) == "Springfield") weather("Springfield, Illinois, US", false, 1_757_972_400)
            else weather("${fields.getOrNull(1)}, Washington, US", fields.getOrNull(2) == "forecast", kotlin.time.Clock.System.now().epochSeconds - 20 * 60)
        else -> null
    }
}

// Scales a plain leading quantity, as the core does for the common case.
internal suspend fun previewRecipeScale(message: ChatMessage, part: MessagePart, serves: Int): RecipeContent {
    val recipe = part.recipe ?: error("Recipe unavailable")
    val from = recipe.serves ?: error("Recipe has no servings")
    val scaled = recipe.ingredients.map { text ->
        val match = Regex("^\\d+(\\.\\d+)?(?=[ A-Za-z])").find(text.text)?.takeIf { text.spans.isEmpty() } ?: return@map text to false
        val value = match.value.toDouble() * serves / from
        val shown = if (value == kotlin.math.floor(value)) value.toLong().toString() else ((value * 100).toLong() / 100.0).toString()
        RichText(shown + text.text.substring(match.value.length)) to true
    }
    return recipe.copy(serves = serves, ingredients = scaled.map { it.first }, scaled = scaled.map { it.second })
}
