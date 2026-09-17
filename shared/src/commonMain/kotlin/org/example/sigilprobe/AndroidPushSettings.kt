package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.launch

internal data class AndroidPushConfiguration(val application:String)

@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun AndroidPushSettings(project:String,read:suspend ()->AndroidPushConfiguration,
    save:suspend (String?)->AndroidPushConfiguration,
    field:@Composable (String,String,(String)->Unit,Boolean,Boolean)->Unit) {
    var current by remember(project) {mutableStateOf<AndroidPushConfiguration?>(null)}
    var source by remember(project) {mutableStateOf("")}
    var busy by remember {mutableStateOf(false)}
    var error by remember {mutableStateOf("")}
    var saved by remember {mutableStateOf(false)}
    val scope=rememberCoroutineScope()
    suspend fun perform(operation:suspend ()->AndroidPushConfiguration) {
        if(busy)return
        busy=true;error="";saved=false
        try {current=operation();source=""}
        catch(cancelled:CancellationException) {throw cancelled}
        catch(failure:Exception) {error=failure.message ?: "Android configuration could not be saved. Reload and try again."}
        finally {busy=false}
    }
    LaunchedEffect(project) {perform(read)}
    Column(Modifier.fillMaxWidth(),horizontalAlignment=Alignment.CenterHorizontally) {
      Column(Modifier.widthIn(max=680.dp).fillMaxWidth().padding(horizontal=16.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
        SettingsSectionLabel("Android app configuration")
        SettingsNote("Register org.sigil.compose as an Android app in Firebase project $project. Paste its google-services.json below. The standard Sigil app receives these public settings after sign-in; no custom build is needed.")
        if(busy)LinearProgressIndicator(Modifier.fillMaxWidth())
        if(error.isNotEmpty())Text(error,Modifier.padding(horizontal=12.dp),style=MaterialTheme.typography.bodyMedium,color=MaterialTheme.colorScheme.error)
        current?.let {value->
            if(value.application.isNotEmpty())SettingsNote("Configured · ${value.application}")
            field("Android google-services.json",source,{if(it.length<=32768) {source=it;saved=false}},false,!busy)
            SettingsNote("Only the app ID, project ID, sender ID and public API key are retained. Service-account private keys belong in Google notification credentials above.")
            FlowRow(horizontalArrangement=Arrangement.spacedBy(12.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                SigilButton({scope.launch(start=CoroutineStart.UNDISPATCHED) {perform {save(source).also {saved=true}}}},enabled=!busy && source.isNotBlank()) {Text("Save Android configuration")}
                if(value.application.isNotEmpty())SigilTextButton({scope.launch(start=CoroutineStart.UNDISPATCHED) {perform {save(null).also {saved=true}}}},enabled=!busy) {Text("Remove")}
            }
        }
        SigilTextButton({scope.launch(start=CoroutineStart.UNDISPATCHED) {perform(read)}},enabled=!busy) {Text("Reload Android configuration")}
        if(saved)SettingsNote("Saved. Reopen Sigil on each Android device to complete registration. Changing projects requires closing the app completely first.")
      }
    }
}
