package org.sigil.compose

import org.json.JSONObject
import org.sigil.ContentDecoder

internal fun nativeStructuredPreview(source: String): org.sigil.MessagePart? = runCatching {
    previewPart(JSONObject(org.sigil.NativeCore.structuredPreview(JSONObject().put("source",source).put("now",System.currentTimeMillis()/1000).put("timezone",java.util.TimeZone.getDefault().id).toString())))
}.getOrNull()

internal fun previewPart(p:JSONObject)=ContentDecoder.part(p.toString()) {at->java.text.DateFormat.getDateTimeInstance(java.text.DateFormat.MEDIUM,java.text.DateFormat.SHORT).format(java.util.Date(at*1000))}
internal fun JSONObject.richText()=ContentDecoder.richText(toString())
internal fun JSONObject.richValue()=ContentDecoder.richValue(toString())
internal fun JSONObject.tableContent()=ContentDecoder.tableContent(toString())
internal fun JSONObject.recipeContent()=ContentDecoder.recipeContent(toString())
internal fun JSONObject.chartContent()=ContentDecoder.chartContent(toString())
internal fun JSONObject.diagramContent()=ContentDecoder.diagramContent(toString())
internal fun JSONObject.utilityContent()=ContentDecoder.utilityContent(toString())
internal fun JSONObject.serviceContent()=ContentDecoder.serviceContent(toString())
internal fun JSONObject.contactContent()=ContentDecoder.contactContent(toString())
