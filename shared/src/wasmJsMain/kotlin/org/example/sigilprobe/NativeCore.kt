@file:JsModule("./sigil_core.js")
@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)

package org.sigil

import kotlin.js.*

@JsName("default")
external fun initializeRust(): Promise<JsAny>

@JsName("palette")
external fun rustPalette(accent: Int, dark: Boolean): String
