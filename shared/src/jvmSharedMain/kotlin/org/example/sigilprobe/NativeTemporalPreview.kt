package org.sigil

import java.time.Instant
import java.time.ZoneOffset
import java.time.chrono.IsoChronology
import java.time.format.DateTimeFormatter
import java.time.format.DateTimeFormatterBuilder
import java.time.format.FormatStyle
import java.util.Locale
import java.util.TimeZone

fun nativeTemporalPreview(kind:String,input:String):TemporalPreview? = temporalPreview(kind,input,System.currentTimeMillis()/1000,TimeZone.getDefault().id,Locale.getDefault())

internal fun temporalPreview(kind:String,input:String,now:Long,timezone:String,locale:Locale):TemporalPreview? {
    if(input.length>256 || input.any {it=='\n' || it=='\r'})return null
    val pattern=DateTimeFormatterBuilder.getLocalizedDateTimePattern(FormatStyle.SHORT,null,IsoChronology.INSTANCE,locale).replace(Regex("'(?:[^']|'')*'"),"")
    val month=pattern.indexOfFirst {it=='M' || it=='L'}
    val day=pattern.indexOf('d')
    val order=if(month>=0 && (day<0 || month<day))"month" else "day"
    val result=NativeCore.temporalPreview("$kind\n$now\n$timezone\n$order\n$input").split('\n')
    if(result.size!=3)return null
    val at=result[1].toLongOrNull() ?: return null
    val label=if(kind=="Timer") {
        var rest=at
        listOf(86400L to "day",3600L to "hour",60L to "minute",1L to "second").mapNotNull {(size,name)->
            val count=rest/size;rest%=size
            if(count>0)"$count $name${if(count==1L)"" else "s"}" else null
        }.joinToString(" ")
    } else {
        val offset=runCatching {ZoneOffset.of(result[2])}.getOrNull() ?: return null
        DateTimeFormatter.ofLocalizedDateTime(FormatStyle.MEDIUM,FormatStyle.SHORT).withLocale(locale).withZone(offset).format(Instant.ofEpochSecond(at))+" · $timezone (UTC${result[2]})"
    }
    return TemporalPreview(result[0],timezone,label)
}
