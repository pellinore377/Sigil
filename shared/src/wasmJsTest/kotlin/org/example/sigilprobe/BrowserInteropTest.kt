@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)
package org.sigil

import kotlin.js.*
import kotlin.test.*
import kotlinx.coroutines.CancellationException

@JsName("JSON") private external object BrowserJson:JsAny {
    fun parse(value:String):JsAny
}

class BrowserInteropTest {
    @Test fun browser_errors_reach_existing_exception_handlers() {
        val error=assertFailsWith<IllegalStateException> {
            browserBoundary { BrowserJson.parse("{") }
        }
        assertIs<JsException>(error.cause)
    }

    @Test fun cancellation_is_not_wrapped() {
        val cancelled=CancellationException("Stopped")
        assertSame(cancelled,assertFailsWith<CancellationException> {
            browserBoundary { throw cancelled }
        })
    }
}
