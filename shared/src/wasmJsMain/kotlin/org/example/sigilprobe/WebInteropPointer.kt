package org.sigil

import org.w3c.dom.HTMLElement
import org.w3c.dom.ShadowRoot

internal fun webInteropPointerPassThrough(element: HTMLElement) {
    val host=(element.parentNode as? ShadowRoot)?.host as? HTMLElement ?: element.parentElement as? HTMLElement
    host?.style?.setProperty("pointer-events","none")
}
