@file:JsModule("./sigil_core.js")
@file:OptIn(kotlin.js.ExperimentalWasmJsInterop::class)

package org.sigil

import kotlin.js.*

@JsName("default")
external fun initializeRust(): Promise<JsAny>

@JsName("palette")
external fun rustPalette(accent: Int, dark: Boolean): String

@JsName("analyze") external fun rustAnalyze(input:String):String
@JsName("editor") external fun rustEditor(input:String):String
@JsName("builder_source") external fun rustBuilder(input:String):String
@JsName("structured_preview") external fun rustPreview(input:String):String
@JsName("playground") external fun rustPlayground(input:String):String
@JsName("code_preview") external fun rustCode(input:String):String
@JsName("help_catalog") external fun rustHelp(input:String):String
@JsName("temporal_preview") external fun rustTemporal(input:String):String
@JsName("motion_seeds") external fun rustMotionSeeds(input:String):String
