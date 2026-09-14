@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)
package org.sigil

import kotlin.js.JsAny
import kotlin.js.JsException
import kotlin.js.Promise
import kotlinx.coroutines.await

internal inline fun <T> browserBoundary(action:()->T):T = try { action() }
catch(error:JsException) { throw IllegalStateException(error.message ?: "Browser operation failed",error) }

internal suspend fun <T:JsAny?> Promise<T>.awaitBrowser():T = browserBoundary { await() }
