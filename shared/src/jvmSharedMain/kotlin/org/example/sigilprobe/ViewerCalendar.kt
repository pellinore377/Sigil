package org.sigil

internal actual fun viewerWeekday(at:Long):Int=runCatching {java.time.Instant.ofEpochSecond(at).atZone(java.time.ZoneId.systemDefault()).dayOfWeek.value}.getOrDefault(0)
internal actual fun viewerZone():String=runCatching {java.time.ZoneId.systemDefault().id}.getOrDefault("")
