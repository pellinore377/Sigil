package org.sigil

import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.*

internal class BuilderSizing(private val density:Density,private val inset:Dp,private val report:(Dp)->Unit) {
    private val heights=mutableMapOf<String,Dp>()
    fun measure(part:String)=Modifier.onSizeChanged {size->
        heights[part]=with(density){size.height.toDp()}
        if("header" in heights && "body" in heights)report(heights.values.fold(inset) {a,b->a+b})
    }
}

@Composable
internal fun rememberBuilderSizing(inset:Dp):BuilderSizing {
    val density=LocalDensity.current
    val report by rememberUpdatedState(LocalComposerPanelHeight.current)
    return remember(density,inset){BuilderSizing(density,inset){report?.invoke(it)}}
}

@Composable
fun naturalPanelHeight():Modifier {
    val density=LocalDensity.current
    val report=LocalComposerPanelHeight.current
    return Modifier.onSizeChanged {report?.invoke(with(density){it.height.toDp()})}
}
