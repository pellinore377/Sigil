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

internal fun JSONObject.richText(): RichText? = optJSONObject("rich")?.let { it.richValue() }
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
    }, rich.optJSONArray("code_tokens")?.objects()?.map { CodeToken(it.getInt("start"), it.getInt("end"), it.getString("role")) }.orEmpty())
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
