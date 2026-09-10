package org.sigil

import androidx.compose.runtime.staticCompositionLocalOf

data class TemporalPreview(val source:String,val timezone:String,val label:String)
val LocalTemporalPreview=staticCompositionLocalOf<((String,String)->TemporalPreview?)?> {null}
