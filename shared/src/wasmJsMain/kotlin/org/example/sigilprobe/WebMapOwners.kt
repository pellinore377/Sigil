package org.sigil

import androidx.compose.runtime.mutableStateListOf

internal class WebMapOwners {
    private val owners = mutableStateListOf<Any>()
    val current: Any? get() = owners.lastOrNull()
    fun attach(owner: Any) { if (owners.none { it === owner }) owners.add(owner) }
    fun detach(owner: Any) { owners.removeAll { it === owner } }
}
