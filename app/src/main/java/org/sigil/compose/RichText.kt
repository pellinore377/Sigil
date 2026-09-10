package org.sigil.compose

import org.json.JSONArray
import org.json.JSONObject
import org.sigil.RichText
import org.sigil.RichSpan
import org.sigil.RichBlock
import org.sigil.CodeToken
import org.sigil.TableContent
import org.sigil.RecipeContent
import org.sigil.ChartContent
import org.sigil.ChartPoint
import org.sigil.DiagramContent
import org.sigil.DiagramNode
import org.sigil.DiagramEdge
import org.sigil.DiagramEntry

internal fun JSONObject.richText(): RichText? = optJSONObject("rich")?.let { it.richValue() }
internal fun JSONObject.contactContent(): org.sigil.ContactContent? = optJSONObject("contact")?.let { c ->
    org.sigil.ContactContent(c.getString("address"),c.getJSONObject("name").richValue(),c.getString("identity"),if(c.isNull("vcard"))null else c.getString("vcard"))
}
internal fun JSONObject.richValue(): RichText {
    val rich = this
    fun JSONArray.objects() = (0 until length()).map(::getJSONObject)
    return RichText(rich.getString("text"), rich.getJSONArray("spans").objects().map { span ->
        val flags = mutableSetOf<String>()
        var colors = emptyList<String>()
        var size = 0
        var reveal = ""
        var link: String? = null
        span.getJSONArray("effects").objects().forEach { effect ->
            when (val kind = effect.getString("kind")) {
                "emphasis", "decoration" -> flags += effect.getString("value")
                "code", "monospace" -> flags += kind
                "size" -> size = effect.getInt("value")
                "reveal" -> reveal = effect.getString("value")
                "link" -> link = effect.getString("value")
                "color", "background" -> {
                    if (kind == "background") flags += "background"
                    val paint = effect.getJSONObject("value")
                    colors = when (paint.getString("type")) {
                        "solid" -> listOf(paint.getString("color"))
                        "gradient" -> paint.getJSONArray("stops").let { a -> (0 until a.length()).map(a::getString) }
                        "rainbow" -> listOf("red2", "orange2", "yellow2", "green2", "cyan2", "blue2", "purple2")
                        else -> emptyList()
                    }
                }
            }
        }
        RichSpan(span.getInt("start"), span.getInt("end"), flags, colors, size, reveal, link)
    }, rich.getJSONArray("blocks").objects().map { block ->
        val kind = block.getJSONObject("kind")
        RichBlock(block.getInt("start"), block.getInt("end"), kind.getString("kind"), kind.optInt("level"), if (kind.isNull("language")) "" else kind.getString("language"))
    }, rich.optJSONArray("code_tokens")?.objects()?.map { CodeToken(it.getInt("start"), it.getInt("end"), it.getString("role")) }.orEmpty(),
        rich.optJSONArray("motion")?.objects()?.map { run ->
            val p=run.getJSONObject("parameters")
            val units=run.getJSONArray("units")
            org.sigil.TextMotion(run.getString("animation"),p.getInt("duration_ms"),p.getInt("cycles"),p.getInt("displacement"),p.getInt("rotation"),p.getInt("scale_per_mille"),p.getInt("stagger_ms"),p.getInt("particles"),
                (0 until units.length()).map { units.getJSONArray(it).let { u->u.getInt(0) to u.getInt(1) } },
                p.getJSONArray("easing").let {a->(0 until a.length()).map {a.getInt(it)/1000f}},p.getInt("particle_lifetime_ms"),p.getInt("spring_stiffness"),p.getInt("spring_damping"))
        }.orEmpty())
}

internal fun JSONObject.tableContent(): TableContent? = optJSONObject("table")?.let { table ->
    fun JSONArray.texts() = (0 until length()).map { getJSONObject(it).richValue() }
    val rows = table.getJSONArray("rows")
    val orders = table.getJSONArray("numeric_order")
    val copies = table.getJSONArray("copy_rows")
    TableContent(table.getJSONArray("columns").texts(), (0 until rows.length()).map { rows.getJSONArray(it).texts() },
        (0 until orders.length()).map { column -> orders.optJSONArray(column)?.let { order -> (0 until order.length()).map(order::getInt) } },
        (0 until copies.length()).map { if (copies.isNull(it)) null else copies.getString(it) },
        if (table.isNull("copy_table")) null else table.getString("copy_table"))
}
internal fun JSONObject.recipeContent(): RecipeContent? = optJSONObject("recipe")?.let { recipe ->
    fun texts(name: String) = recipe.getJSONArray(name).let { a -> (0 until a.length()).map { a.getJSONObject(it).richValue() } }
    RecipeContent(recipe.getJSONObject("title").richValue(), if (recipe.isNull("serves")) null else recipe.getInt("serves"),
        if (recipe.isNull("original_serves")) null else recipe.getInt("original_serves"), if (recipe.isNull("seconds")) null else recipe.getLong("seconds"),
        texts("ingredients"), recipe.getJSONArray("scaled").let { a -> (0 until a.length()).map(a::getBoolean) }, texts("steps"))
}
internal fun JSONObject.chartContent(): ChartContent? = optJSONObject("chart")?.let { chart ->
    fun strings(name: String) = chart.getJSONArray(name).let { a -> (0 until a.length()).map(a::getString) }
    ChartContent(chart.getString("kind"), chart.getJSONObject("title").richValue(), chart.getBoolean("horizontal"), chart.getDouble("zero").toFloat(),
        strings("y_ticks"), strings("x_ticks"), if (chart.isNull("copy_data")) null else chart.getString("copy_data"),
        chart.getJSONArray("points").let { a -> (0 until a.length()).map { index ->
            val p = a.getJSONObject(index)
            ChartPoint(p.getJSONObject("label").richValue(), p.getDouble("x").toFloat(), p.getDouble("y").toFloat(), p.getString("value"),
                if (p.isNull("x_value")) null else p.getString("x_value"), p.getDouble("share").toFloat(), p.getString("percent"))
        } })
}
internal fun JSONObject.diagramContent(): DiagramContent? = optJSONObject("diagram")?.let { diagram ->
    fun <T> list(name: String, read: (JSONObject) -> T) = diagram.getJSONArray(name).let { a -> (0 until a.length()).map { read(a.getJSONObject(it)) } }
    DiagramContent(diagram.getString("kind"), diagram.getJSONObject("title").richValue(), diagram.getDouble("width").toFloat(), diagram.getDouble("height").toFloat(),
        list("nodes") { DiagramNode(it.getJSONObject("label").richValue(), it.getString("shape"), it.getDouble("x").toFloat(), it.getDouble("y").toFloat()) },
        list("edges") { DiagramEdge(it.getInt("from"), it.getInt("to"), it.getJSONObject("label").richValue(), it.getBoolean("dashed"), it.getDouble("y").toFloat()) },
        list("entries") { DiagramEntry(it.getJSONObject("date").richValue(), it.getJSONObject("label").richValue()) })
}

internal fun JSONObject.utilityContent(): org.sigil.UtilityContent? = optJSONObject("utility")?.let { u ->
    fun string(name: String) = if (u.isNull(name)) null else u.getString(name)
    org.sigil.UtilityContent(u.getString("kind"), u.optString("display"), u.optString("alternate"), string("copy"), u.richText(),
        u.optJSONObject("secondary")?.richValue(), u.optJSONArray("details")?.let { a -> (0 until a.length()).map { a.getJSONObject(it).richValue() } }.orEmpty(),
        if (u.isNull("selected")) null else u.getInt("selected"), if (u.isNull("ratio")) null else u.getDouble("ratio").toFloat(),
        if (u.isNull("rgba")) null else u.getLong("rgba"), string("mathml"), u.optJSONObject("qr")?.let { q ->
            org.sigil.QrContent(q.getString("kind"), q.getInt("width"), q.getString("cells"), q.getString("payload"), q.optJSONObject("password")?.richValue(), q.getBoolean("concealed"))
        })
}

internal fun JSONObject.serviceContent(): org.sigil.ServiceContent? = optJSONObject("service")?.let { s ->
    fun JSONObject.string(name: String) = if (isNull(name)) null else getString(name)
    fun JSONObject.strings(name: String) = getJSONArray(name).let { a -> (0 until a.length()).map(a::getString) }
    fun JSONObject.texts(name: String) = getJSONArray(name).let { a -> (0 until a.length()).map { a.getJSONObject(it).richValue() } }
    fun <T> rows(name: String, read: (JSONObject)->T) = s.optJSONArray(name)?.let { a -> (0 until a.length()).map { read(a.getJSONObject(it)) } }.orEmpty()
    fun condition(c: JSONObject) = org.sigil.WeatherConditions(c.getString("date"),c.optString("key"),c.strings("temperature"),if(c.isNull("feels_like"))null else c.strings("feels_like"),
        c.getJSONObject("description").richValue(),c.getString("icon"),c.string("rain"),c.string("chance"),c.strings("wind"),c.string("humidity"),c.string("uv"))
    org.sigil.ServiceContent(s.getString("kind"),s.getJSONObject("title").richValue(),s.getJSONObject("attribution").richValue(),s.getString("stamp"),s.string("source"),s.optString("language"),s.string("copy"),
        s.optJSONObject("original")?.richValue(),s.optJSONObject("pronunciation")?.richValue(),s.string("audio"),rows("senses") { d ->
            org.sigil.DefinitionSense(d.getJSONObject("part").richValue(),d.getJSONObject("definition").richValue(),d.optJSONObject("example")?.richValue(),d.optJSONObject("etymology")?.richValue(),d.texts("synonyms"),d.texts("antonyms"),d.string("copy"))
        },s.optJSONObject("current")?.let(::condition),rows("days") { d -> org.sigil.WeatherDay(d.getString("date"),d.getString("key"),d.strings("low"),d.strings("high"),d.getString("icon"),d.getString("chance"),d.getJSONObject("description").richValue(),d.optJSONArray("charts")?.let { a -> (0 until a.length()).mapNotNull { JSONObject().put("chart",a.getJSONObject(it)).chartContent() } }.orEmpty()) },rows("hours",::condition),s.optString("today"),s.optBoolean("historical"))
}
