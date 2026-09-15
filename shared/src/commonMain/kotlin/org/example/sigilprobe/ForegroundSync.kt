package org.sigil

fun foregroundSyncWait(nextAtSeconds: Long, nowMillis: Long, ceilingMillis: Long = 1000): Long {
    val nowSeconds = nowMillis / 1000
    if (nextAtSeconds <= nowSeconds) return 50
    if (nextAtSeconds - nowSeconds > 1) return ceilingMillis
    return (1000 - nowMillis % 1000).coerceIn(50, ceilingMillis)
}
