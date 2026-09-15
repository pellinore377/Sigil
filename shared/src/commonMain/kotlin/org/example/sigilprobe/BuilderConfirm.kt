package org.sigil

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier

internal class ComposerConfirmation {
    var action by mutableStateOf<ConfirmationAction?>(null)
}
internal class ConfirmationAction(val owner:Any,val label:String,val enabled:Boolean,val invoke:()->Unit)
internal val LocalComposerConfirmation=staticCompositionLocalOf<ComposerConfirmation?> {null}

@Composable
fun BuilderConfirm(label:String=LocalBuilderAction.current,enabled:Boolean=true,onClick:()->Unit) {
    val host=LocalComposerConfirmation.current
    if(host==null) {SigilButton(onClick,Modifier.fillMaxWidth(),enabled=enabled){Text(label)};return}
    val owner=remember {Any()}
    val current by rememberUpdatedState(onClick)
    val invoke=remember { {current()} }
    val action=remember(owner,label,enabled){ConfirmationAction(owner,label,enabled,invoke)}
    SideEffect {host.action=action}
    DisposableEffect(host) {onDispose {if(host.action?.owner===owner)host.action=null}}
}
