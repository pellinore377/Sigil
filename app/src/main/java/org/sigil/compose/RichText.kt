package org.sigil.compose

import org.json.JSONArray
import org.json.JSONObject
import org.sigil.RichText
import org.sigil.RichSpan
import org.sigil.RichBlock

internal fun JSONObject.richText(): RichText? = optJSONObject("rich")?.let { rich ->
    fun JSONArray.objects() = (0 until length()).map(::getJSONObject)
    RichText(rich.getString("text"), rich.getJSONArray("spans").objects().map { span ->
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
        RichBlock(block.getInt("start"), block.getInt("end"), kind.getString("kind"), kind.optInt("level"))
    })
}
